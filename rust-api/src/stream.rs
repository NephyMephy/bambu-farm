use crate::config::Settings;
use crate::models::{PrinterRecord, StreamState};
use std::collections::HashMap;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

/// On Windows, create ffmpeg in a new process group so we can kill the tree.
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;

pub struct WorkerManager {
    workers: Mutex<HashMap<String, WorkerProc>>,
    stream_slots: Arc<Semaphore>,
}

struct WorkerProc {
    child: Child,
    _permit: OwnedSemaphorePermit,
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
            if let Some(proc_ref) = workers.get_mut(&k) {
                if proc_ref.child.try_wait().ok().flatten().is_some() {
                    workers.remove(&k);
                }
            }
        }
        workers.len()
    }

    pub async fn state(&self, printer_id: &str) -> StreamState {
        let mut workers = self.workers.lock().await;
        if let Some(proc_ref) = workers.get_mut(printer_id) {
            match proc_ref.child.try_wait() {
                Ok(Some(_)) => {
                    workers.remove(printer_id);
                    StreamState::Stopped
                }
                Ok(None) => StreamState::Running,
                Err(_) => StreamState::Error,
            }
        } else {
            StreamState::Stopped
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

        let input_url = Self::rtsp_source_url(printer);
        let output_url = Self::rtsp_publish_url(printer, settings);

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

        let mut workers = self.workers.lock().await;
        workers.insert(
            printer.id.clone(),
            WorkerProc {
                child,
                _permit: permit,
            },
        );

        Ok(StreamState::Starting)
    }

    pub async fn stop_stream(&self, printer_id: &str) -> Result<StreamState, String> {
        let mut workers = self.workers.lock().await;
        let Some(mut worker) = workers.remove(printer_id) else {
            return Ok(StreamState::Stopped);
        };

        kill_process_tree(&mut worker.child).await?;

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
