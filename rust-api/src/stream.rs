use crate::config::Settings;
use crate::models::{PrinterRecord, StreamType, StreamState};
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::process::Stdio;
use tokio::process::{Child, Command};
use tokio::sync::{watch, Mutex, OwnedSemaphorePermit, Semaphore};
use tokio_rustls::rustls;
use tracing::{error, info, warn};

/// On Windows, create ffmpeg in a new process group so we can kill the tree.
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;

/// Maximum JPEG frame size (10 MB, same as Java).
const MAX_FRAME_SIZE: usize = 10_000_000;

/// Watchdog timeout — reconnect if no frame received within this duration.
const WATCHDOG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Reconnect delay after a watchdog timeout or connection error.
const RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(10);

// ─── Worker types ────────────────────────────────────────────────────────────

/// An FFmpeg child process (for RTSPS models).
struct FfmpegWorker {
    child: Child,
    _permit: OwnedSemaphorePermit,
}

/// A proprietary TCP JPEG stream (for P1P/P1S/A1/A1Mini models).
struct ProprietaryWorker {
    /// Channel to signal the background task to stop.
    cancel: watch::Sender<bool>,
    /// Latest JPEG frame receiver.
    latest_frame: watch::Receiver<Option<Arc<Vec<u8>>>>,
    /// Background task running the proprietary reconnect/read loop.
    task: tokio::task::JoinHandle<()>,
    _permit: OwnedSemaphorePermit,
}

/// Either an FFmpeg worker or a proprietary stream worker.
enum Worker {
    Ffmpeg(FfmpegWorker),
    Proprietary(ProprietaryWorker),
}

// ─── WorkerManager ───────────────────────────────────────────────────────────

pub struct WorkerManager {
    workers: Mutex<HashMap<String, Worker>>,
    stream_slots: Arc<Semaphore>,
}

impl WorkerManager {
    pub fn new(max_concurrent_streams: usize) -> Self {
        Self {
            workers: Mutex::new(HashMap::new()),
            stream_slots: Arc::new(Semaphore::new(max_concurrent_streams)),
        }
    }

    pub async fn running_count(&self) -> usize {
        let mut workers = self.workers.lock().await;
        let keys: Vec<String> = workers.keys().cloned().collect();
        for k in keys {
            if let Some(worker) = workers.get_mut(&k) {
                if !is_worker_alive(worker).await {
                    workers.remove(&k);
                }
            }
        }
        workers.len()
    }

    pub async fn state(&self, printer_id: &str) -> StreamState {
        let mut workers = self.workers.lock().await;
        if let Some(worker) = workers.get_mut(printer_id) {
            let alive = is_worker_alive(worker).await;
            if !alive {
                workers.remove(printer_id);
                StreamState::Stopped
            } else {
                StreamState::Running
            }
        } else {
            StreamState::Stopped
        }
    }

    /// Get the latest JPEG frame for a proprietary stream printer.
    pub async fn latest_frame(&self, printer_id: &str) -> Option<Arc<Vec<u8>>> {
        let workers = self.workers.lock().await;
        if let Some(Worker::Proprietary(pw)) = workers.get(printer_id) {
            pw.latest_frame.borrow().clone()
        } else {
            None
        }
    }

    pub async fn start_stream(
        &self,
        printer: &PrinterRecord,
        settings: &Settings,
    ) -> Result<StreamState, String> {
        if self.state(&printer.id).await == StreamState::Running {
            return Ok(StreamState::Running);
        }

        let permit = self
            .stream_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| "failed to reserve stream slot".to_string())?;

        let worker = match printer.stream.stream_type {
            StreamType::Rtsp => {
                Worker::Ffmpeg(start_ffmpeg(printer, settings, permit).await?)
            }
            StreamType::Proprietary => {
                Worker::Proprietary(start_proprietary(printer, permit).await?)
            }
        };

        let mut workers = self.workers.lock().await;
        workers.insert(printer.id.clone(), worker);
        Ok(StreamState::Starting)
    }

    pub async fn stop_stream(&self, printer_id: &str) -> Result<StreamState, String> {
        let mut workers = self.workers.lock().await;
        let Some(worker) = workers.remove(printer_id) else {
            return Ok(StreamState::Stopped);
        };

        match worker {
            Worker::Ffmpeg(mut fw) => {
                kill_process_tree(&mut fw.child).await?;
            }
            Worker::Proprietary(pw) => {
                // Signal the background task to stop
                let _ = pw.cancel.send(true);
            }
        }

        Ok(StreamState::Stopped)
    }

    pub fn rtsp_source_url(printer: &PrinterRecord) -> String {
        let encoded_user = urlencoding::encode(&printer.credentials.username);
        let encoded_password = urlencoding::encode(&printer.credentials.access_code);
        format!(
            "rtsps://{}:{}@{}:{}{}",
            encoded_user,
            encoded_password,
            printer.host,
            printer.stream.rtsp_port,
            printer.stream.rtsp_path
        )
    }

    pub fn rtsp_publish_url(printer: &PrinterRecord, settings: &Settings) -> String {
        format!("{}/{}", settings.mediamtx_rtsp_publish, printer.id)
    }
}

// ─── Worker liveness check ───────────────────────────────────────────────────

async fn is_worker_alive(worker: &mut Worker) -> bool {
    match worker {
        Worker::Ffmpeg(fw) => fw.child.try_wait().ok().flatten().is_none(),
        Worker::Proprietary(pw) => !pw.task.is_finished(),
    }
}

// ─── FFmpeg worker ───────────────────────────────────────────────────────────

async fn start_ffmpeg(
    printer: &PrinterRecord,
    settings: &Settings,
    permit: OwnedSemaphorePermit,
) -> Result<FfmpegWorker, String> {
    let input_url = WorkerManager::rtsp_source_url(printer);
    let output_url = WorkerManager::rtsp_publish_url(printer, settings);

    let mut cmd = Command::new(&settings.ffmpeg_bin);
    cmd.args([
        "-nostdin",
        "-rtsp_transport",
        "tcp",
        "-timeout",
        "30000000",
        "-i",
        &input_url,
        "-c:v",
        "copy",
        "-f",
        "rtsp",
        "-rtsp_transport",
        "tcp",
        &output_url,
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);

    let child = cmd
        .spawn()
        .map_err(|e| format!("failed to start ffmpeg: {e}"))?;

    Ok(FfmpegWorker {
        child,
        _permit: permit,
    })
}

// ─── Proprietary TCP JPEG Stream ─────────────────────────────────────────────

async fn start_proprietary(
    printer: &PrinterRecord,
    permit: OwnedSemaphorePermit,
) -> Result<ProprietaryWorker, String> {
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (frame_tx, frame_rx) = watch::channel(None);

    let printer_id = printer.id.clone();
    let host = printer.host.clone();
    let port = printer.stream.rtsp_port;
    let username = printer.credentials.username.clone();
    let access_code = printer.credentials.access_code.clone();

    // Spawn a background task that connects to the printer and reads JPEG frames
    let task = tokio::spawn(async move {
        proprietary_stream_loop(
            &printer_id,
            &host,
            port,
            &username,
            &access_code,
            &frame_tx,
            cancel_rx,
        )
        .await;
    });

    Ok(ProprietaryWorker {
        cancel: cancel_tx,
        latest_frame: frame_rx,
        task,
        _permit: permit,
    })
}

// ─── Proprietary TCP JPEG Stream Protocol ────────────────────────────────────
//
// Based on the Java implementation in BambuPrinterStream.java.
//
// 1. Connect via TLS to <printer_ip>:6000 (accept invalid certs)
// 2. Send 80-byte handshake:
//    - 4 bytes LE: 0x40 (header size = 64)
//    - 4 bytes LE: 0x3000 (protocol version)
//    - 8 bytes LE: 0 (reserved)
//    - 32 bytes: username (null-padded)
//    - 32 bytes: access code (null-padded)
// 3. Receive frames, each with a 16-byte header:
//    - 4 bytes LE: JPEG payload size (N)
//    - 4 bytes: unknown (skipped)
//    - 8 bytes: unknown/timestamp (skipped)
//    - N bytes: raw JPEG data
// 4. Watchdog: reconnect if no frame within timeout

async fn proprietary_stream_loop(
    printer_id: &str,
    host: &str,
    port: u16,
    username: &str,
    access_code: &str,
    frame_tx: &watch::Sender<Option<Arc<Vec<u8>>>>,
    mut cancel_rx: watch::Receiver<bool>,
) {
    loop {
        if cancel_rx.has_changed().ok().map(|v| v).unwrap_or(false) {
            info!(printer = printer_id, "proprietary stream cancelled");
            return;
        }

        match proprietary_connect(printer_id, host, port, username, access_code, frame_tx, &mut cancel_rx).await {
            Ok(()) => {
                // Stream ended normally (cancel signal received)
                return;
            }
            Err(e) => {
                error!(printer = printer_id, "proprietary stream error: {e}, reconnecting in {}s", RECONNECT_DELAY.as_secs());
            }
        }

        // Wait before reconnecting, checking cancel signal
        tokio::select! {
            _ = tokio::time::sleep(RECONNECT_DELAY) => {}
            _ = cancel_rx.changed() => {
                info!(printer = printer_id, "proprietary stream cancelled during reconnect delay");
                return;
            }
        }
    }
}

async fn proprietary_connect(
    printer_id: &str,
    host: &str,
    port: u16,
    username: &str,
    access_code: &str,
    frame_tx: &watch::Sender<Option<Arc<Vec<u8>>>>,
    cancel_rx: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    // Match the Java client: trust all certificates and disable hostname verification.
    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerifier))
        .with_no_client_auth();

    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));

    let addr = format!("{host}:{port}");

    let stream = tokio::net::TcpStream::connect(&addr)
        .await
        .map_err(|e| format!("TCP connect to {addr} failed: {e}"))?;

    let server_name = rustls_pki_types::ServerName::try_from(host.to_string())
        .unwrap_or_else(|_| rustls_pki_types::ServerName::try_from("localhost").unwrap());

    let mut tls_stream = connector.connect(server_name, stream).await
        .map_err(|e| format!("TLS handshake to {addr} failed: {e}"))?;

    info!(printer = printer_id, "connected to proprietary stream at {addr}");

    // Send the 80-byte handshake
    let handshake = build_handshake(username, access_code);
    use tokio::io::AsyncWriteExt;
    tls_stream.write_all(&handshake).await
        .map_err(|e| format!("failed to send handshake: {e}"))?;
    tls_stream.flush().await
        .map_err(|e| format!("failed to flush handshake: {e}"))?;

    info!(printer = printer_id, "sent proprietary stream handshake");

    // Read frames
    use tokio::io::AsyncReadExt;
    let mut buffer = Vec::with_capacity(65536);
    let mut tmp = [0u8; 65536];
    let mut last_frame_time = std::time::Instant::now();

    loop {
        // Check cancel signal
        if cancel_rx.has_changed().ok().map(|v| v).unwrap_or(false) {
            return Ok(());
        }

        // Check watchdog
        if last_frame_time.elapsed() > WATCHDOG_TIMEOUT {
            return Err("watchdog timeout — no frame received".to_string());
        }

        // Read with cancel awareness
        let read_result = tokio::select! {
            result = tls_stream.read(&mut tmp) => result,
            _ = cancel_rx.changed() => return Ok(()),
        };

        match read_result {
            Ok(0) => return Err("connection closed by printer".to_string()),
            Ok(n) => buffer.extend_from_slice(&tmp[..n]),
            Err(e) => return Err(format!("read error: {e}")),
        }

        // Try to parse frames from the buffer
        loop {
            if buffer.len() < 16 {
                break; // Not enough data for a frame header
            }

            // Read the JPEG payload size (4 bytes, little-endian)
            let payload_size = u32::from_le_bytes(
                buffer[0..4].try_into().map_err(|_| "invalid frame header")?
            ) as usize;

            if payload_size == 0 || payload_size > MAX_FRAME_SIZE {
                // Invalid frame — skip this byte and try to resync
                buffer.drain(0..1);
                continue;
            }

            let total_frame_size = 16 + payload_size;
            if buffer.len() < total_frame_size {
                break; // Not enough data for the full frame yet
            }

            // Extract the JPEG data (skip 16-byte header)
            let jpeg_data = buffer[16..total_frame_size].to_vec();

            // Remove the consumed frame from the buffer
            buffer.drain(0..total_frame_size);

            let frame = Arc::new(jpeg_data);
            let _ = frame_tx.send(Some(frame));
            last_frame_time = std::time::Instant::now();
        }

        // Prevent unbounded buffer growth
        if buffer.len() > MAX_FRAME_SIZE {
            warn!(printer = printer_id, "buffer overflow, clearing");
            buffer.clear();
        }
    }
}

/// Build the 80-byte proprietary stream handshake.
fn build_handshake(username: &str, access_code: &str) -> [u8; 80] {
    let mut buf = [0u8; 80];

    // Header size: 0x40 (64) as int32 LE
    buf[0..4].copy_from_slice(&0x40u32.to_le_bytes());
    // Protocol version: 0x3000 as int32 LE
    buf[4..8].copy_from_slice(&0x3000u32.to_le_bytes());
    // Reserved: 8 bytes of zero (already zero)

    // Username: offset 16, max 32 bytes, null-padded
    let username_bytes = username.as_bytes();
    let username_len = username_bytes.len().min(32);
    buf[16..16 + username_len].copy_from_slice(&username_bytes[..username_len]);

    // Access code: offset 48, max 32 bytes, null-padded
    let access_bytes = access_code.as_bytes();
    let access_len = access_bytes.len().min(32);
    buf[48..48 + access_len].copy_from_slice(&access_bytes[..access_len]);

    buf
}

// ─── TLS Certificate Verifier that accepts everything ────────────────────────

#[derive(Debug)]
pub struct NoVerifier;

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls_pki_types::CertificateDer<'_>,
        _intermediates: &[rustls_pki_types::CertificateDer<'_>],
        _server_name: &rustls_pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls_pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

// ─── FFmpeg Process Tree Kill ────────────────────────────────────────────────

/// Kill a child process and its entire process tree.
/// On Unix, `child.kill()` sends SIGKILL to the process group.
/// On Windows, we use `taskkill /F /T /PID` to kill the tree.
async fn kill_process_tree(child: &mut Child) -> Result<(), String> {
    #[cfg(unix)]
    {
        child
            .kill()
            .await
            .map_err(|e| format!("failed to stop stream worker: {e}"))?;
    }

    #[cfg(windows)]
    {
        // Try graceful kill first via taskkill /T (tree) /F (force)
        if let Some(id) = child.id() {
            let output = tokio::process::Command::new("taskkill")
                .args(["/F", "/T", "/PID", &id.to_string()])
                .output()
                .await
                .map_err(|e| format!("failed to run taskkill: {e}"))?;

            if !output.status.success() {
                // Fallback: try the standard kill
                let _ = child.kill().await;
            }
        } else {
            // Process already exited
        }
    }

    Ok(())
}
