use crate::config::Settings;
use crate::models::PrinterRecord;
use crate::stream::WorkerManager;
use chrono::{DateTime, Utc};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, Outgoing, QoS, TlsConfiguration, Transport};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{watch, Mutex, RwLock};
use tokio::time::{Duration, MissedTickBehavior};
use tokio_rustls::rustls;
use tracing::{debug, error, info, warn};

const MQTT_KEEP_ALIVE_SECS: u64 = 30;
const MQTT_RECONNECT_DELAY_SECS: u64 = 5;
const MQTT_FULL_STATUS_INTERVAL_SECS: u64 = 300; // 5 min — avoid lagging P1 series
const MQTT_CONNECTION_TIMEOUT_SECS: u64 = 10;
const MQTT_CLEAN_SESSION: bool = true;

/// Global sequence ID counter for MQTT requests
static SEQUENCE_ID: AtomicU64 = AtomicU64::new(1);

fn next_sequence_id() -> String {
    SEQUENCE_ID.fetch_add(1, Ordering::Relaxed).to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterTelemetry {
    #[serde(default = "Utc::now")]
    pub updated_at: DateTime<Utc>,
    pub gcode_state: Option<String>,
    pub task_name: Option<String>,
    pub progress: Option<u8>,
    pub remaining_minutes: Option<i32>,
    pub layer_num: Option<i32>,
    pub total_layer_num: Option<i32>,
    pub nozzle_temper: Option<f64>,
    pub nozzle_target_temper: Option<f64>,
    pub bed_temper: Option<f64>,
    pub bed_target_temper: Option<f64>,
    pub chamber_temper: Option<f64>,
    pub print_error: Option<i32>,
    pub speed_level: Option<i32>,
    pub print_type: Option<String>,
}

impl Default for PrinterTelemetry {
    fn default() -> Self {
        Self {
            updated_at: Utc::now(),
            gcode_state: None,
            task_name: None,
            progress: None,
            remaining_minutes: None,
            layer_num: None,
            total_layer_num: None,
            nozzle_temper: None,
            nozzle_target_temper: None,
            bed_temper: None,
            bed_target_temper: None,
            chamber_temper: None,
            print_error: None,
            speed_level: None,
            print_type: None,
        }
    }
}

impl PrinterTelemetry {
    pub fn is_printing(&self) -> bool {
        matches!(
            self.gcode_state.as_deref().unwrap_or(""),
            "PREPARE" | "SLICING" | "RUNNING" | "PAUSE"
        )
    }

    pub fn is_idle(&self) -> bool {
        matches!(self.gcode_state.as_deref().unwrap_or(""), "IDLE" | "FINISH")
    }

    pub fn title(&self) -> String {
        self.task_name
            .clone()
            .or_else(|| self.print_type.clone())
            .unwrap_or_else(|| "Printing".to_string())
    }

    pub fn progress_label(&self) -> String {
        self.progress
            .map(|p| format!("{}%", p.min(100)))
            .unwrap_or_else(|| "--".to_string())
    }
}

#[derive(Debug, Clone, Deserialize)]
struct BambuMessage {
    #[serde(default)]
    print: Option<BambuPrint>,
}

#[derive(Debug, Clone, Deserialize)]
struct BambuPrint {
    #[serde(default)]
    nozzle_temper: Option<f64>,
    #[serde(default)]
    nozzle_target_temper: Option<f64>,
    #[serde(default)]
    bed_temper: Option<f64>,
    #[serde(default)]
    bed_target_temper: Option<f64>,
    #[serde(default)]
    chamber_temper: Option<f64>,
    #[serde(default)]
    mc_percent: Option<i32>,
    #[serde(default)]
    mc_remaining_time: Option<i32>,
    #[serde(default)]
    print_error: Option<i32>,
    #[serde(default)]
    spd_lvl: Option<i32>,
    #[serde(default)]
    gcode_state: Option<String>,
    #[serde(default)]
    subtask_name: Option<String>,
    #[serde(default)]
    gcode_file: Option<String>,
    #[serde(default)]
    print_type: Option<String>,
    #[serde(default)]
    layer_num: Option<i32>,
    #[serde(default)]
    total_layer_num: Option<i32>,
}

pub struct TelemetryManager {
    cache: Arc<RwLock<HashMap<String, PrinterTelemetry>>>,
    auto_managed: Arc<RwLock<HashMap<String, bool>>>,
    tasks: Mutex<HashMap<String, watch::Sender<bool>>>,
}

impl TelemetryManager {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            auto_managed: Arc::new(RwLock::new(HashMap::new())),
            tasks: Mutex::new(HashMap::new()),
        }
    }

    pub fn cache(&self) -> Arc<RwLock<HashMap<String, PrinterTelemetry>>> {
        Arc::clone(&self.cache)
    }

    pub async fn snapshot(&self) -> HashMap<String, PrinterTelemetry> {
        self.cache.read().await.clone()
    }

    pub async fn telemetry_for(&self, printer_id: &str) -> Option<PrinterTelemetry> {
        self.cache.read().await.get(printer_id).cloned()
    }

    pub async fn is_auto_managed(&self, printer_id: &str) -> bool {
        self.auto_managed
            .read()
            .await
            .get(printer_id)
            .copied()
            .unwrap_or(false)
    }

    pub async fn mark_manual(&self, printer_id: &str) {
        self.auto_managed
            .write()
            .await
            .insert(printer_id.to_string(), false);
    }

    pub async fn mark_auto(&self, printer_id: &str) {
        self.auto_managed
            .write()
            .await
            .insert(printer_id.to_string(), true);
    }

    pub async fn clear_ownership(&self, printer_id: &str) {
        self.auto_managed.write().await.remove(printer_id);
    }

    pub async fn register_printer(
        &self,
        printer: PrinterRecord,
        settings: Settings,
        workers: Arc<WorkerManager>,
    ) {
        self.unregister_printer(&printer.id).await;

        let (cancel_tx, cancel_rx) = watch::channel(false);
        self.tasks
            .lock()
            .await
            .insert(printer.id.clone(), cancel_tx);

        let cache = self.cache.clone();
        let ownership = self.auto_managed.clone();
        tokio::spawn(async move {
            run_printer_telemetry(printer, settings, workers, cache, ownership, cancel_rx).await;
        });
    }

    pub async fn unregister_printer(&self, printer_id: &str) {
        if let Some(cancel) = self.tasks.lock().await.remove(printer_id) {
            let _ = cancel.send(true);
        }
        self.cache.write().await.remove(printer_id);
        self.clear_ownership(printer_id).await;
    }
}

fn build_tls_config() -> rustls::ClientConfig {
    rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerifier))
        .with_no_client_auth()
}

fn build_mqtt_options(printer: &PrinterRecord) -> MqttOptions {
    // Per OpenBambuAPI: local MQTT uses username "bblp", password = LAN access code
    let username = if printer.credentials.username.is_empty() {
        "bblp".to_string()
    } else {
        printer.credentials.username.clone()
    };

    let mut options = MqttOptions::new(
        // Client ID must be unique per connection
        format!("bambu-farm-{}", printer.id),
        printer.host.clone(),
        8883, // Bambu printers use port 8883 with TLS
    );
    options.set_credentials(username, printer.credentials.access_code.clone());
    options.set_keep_alive(Duration::from_secs(MQTT_KEEP_ALIVE_SECS));
    options.set_clean_session(MQTT_CLEAN_SESSION);

    // Bambu printers use self-signed certs issued by BBL CA.
    // The printer cert CN is the serial number, but we connect by IP.
    // We must disable cert verification (or trust BBL CA + skip hostname check).
    let tls_config = build_tls_config();
    options.set_transport(Transport::tls_with_config(TlsConfiguration::Rustls(Arc::new(tls_config))));

    options
}

fn report_topic(printer: &PrinterRecord) -> String {
    format!("device/{}/report", printer.device_id)
}

fn request_topic(printer: &PrinterRecord) -> String {
    format!("device/{}/request", printer.device_id)
}

async fn publish_full_status(client: &AsyncClient, printer: &PrinterRecord) -> Result<(), String> {
    // Per OpenBambuAPI: pushing.pushall request format
    let payload = serde_json::json!({
        "pushing": {
            "sequence_id": next_sequence_id(),
            "command": "pushall",
            "version": 1,
            "push_target": 1
        }
    });

    client
        .publish(
            request_topic(printer),
            QoS::AtMostOnce,
            false,
            serde_json::to_vec(&payload).map_err(|e| format!("encode pushall payload failed: {e}"))?,
        )
        .await
        .map_err(|e| format!("publish full status failed: {e}"))
}

async fn run_printer_telemetry(
    printer: PrinterRecord,
    settings: Settings,
    workers: Arc<WorkerManager>,
    cache: Arc<RwLock<HashMap<String, PrinterTelemetry>>>,
    auto_managed: Arc<RwLock<HashMap<String, bool>>>,
    mut cancel_rx: watch::Receiver<bool>,
) {
    let report_topic = report_topic(&printer);

    loop {
        if *cancel_rx.borrow() {
            info!(printer = %printer.id, "telemetry cancelled");
            return;
        }

        let options = build_mqtt_options(&printer);
        let (client, mut eventloop) = AsyncClient::new(options, 10);

        // Wait for ConnAck before subscribing — rumqttc requires connection first
        let mut connected = false;
        let mut subscribe_requested = false;
        let mut pushall_requested = false;

        info!(printer = %printer.id, host = %printer.host, "connecting to MQTT...");

        let refresh_secs = MQTT_FULL_STATUS_INTERVAL_SECS;
        let mut refresh = tokio::time::interval(Duration::from_secs(refresh_secs));
        refresh.set_missed_tick_behavior(MissedTickBehavior::Delay);

        // Connection timeout
        let mut connect_timeout = tokio::time::interval(Duration::from_secs(MQTT_CONNECTION_TIMEOUT_SECS));

        loop {
            tokio::select! {
                _ = cancel_rx.changed() => {
                    info!(printer = %printer.id, "telemetry disconnect requested");
                    return;
                }
                _ = refresh.tick() => {
                    if connected {
                        if let Err(e) = publish_full_status(&client, &printer).await {
                            warn!(printer = %printer.id, "failed to refresh telemetry: {e}");
                        } else {
                            debug!(printer = %printer.id, "requested pushall refresh");
                        }
                    }
                }
                _ = connect_timeout.tick() => {
                    if !connected {
                        warn!(printer = %printer.id, "MQTT connection timeout, reconnecting...");
                        break;
                    }
                }
                event = eventloop.poll() => {
                    match event {
                        Ok(Event::Incoming(Incoming::ConnAck(ack))) => {
                            if ack.code == rumqttc::ConnectReturnCode::Success {
                                info!(printer = %printer.id, "MQTT connected, subscribing...");
                                connected = true;
                                // Subscribe after successful connection
                                if let Err(e) = client.subscribe(report_topic.clone(), QoS::AtLeastOnce).await {
                                    error!(printer = %printer.id, "failed to subscribe: {e}");
                                    break;
                                }
                                subscribe_requested = true;
                            } else {
                                error!(printer = %printer.id, "MQTT connection refused: {:?}", ack.code);
                                break;
                            }
                        }
                        Ok(Event::Incoming(Incoming::SubAck(_))) => {
                            if subscribe_requested && !pushall_requested {
                                // Request initial full status after subscription confirmed
                                if let Err(e) = publish_full_status(&client, &printer).await {
                                    warn!(printer = %printer.id, "failed to request initial pushall: {e}");
                                } else {
                                    info!(printer = %printer.id, "requested initial pushall");
                                }
                                pushall_requested = true;
                            }
                        }
                        Ok(Event::Incoming(Incoming::Publish(packet))) if packet.topic == report_topic => {
                            if let Ok(body) = std::str::from_utf8(&packet.payload) {
                                debug!(printer = %printer.id, "received MQTT message ({} bytes)", body.len());
                                if let Ok(message) = serde_json::from_str::<BambuMessage>(body) {
                                    if let Some(print_data) = message.print {
                                        // P1 series sends delta updates — merge with existing telemetry
                                        let telemetry = {
                                            let existing = cache.read().await.get(&printer.id).cloned();
                                            merge_print(existing, &print_data)
                                        };
                                        let auto = auto_managed.read().await.get(&printer.id).copied().unwrap_or(false);

                                        update_cache(&cache, &printer.id, telemetry.clone()).await;

                                        if telemetry.is_printing() && matches!(workers.state(&printer.id).await, crate::models::StreamState::Stopped) {
                                            match workers.start_stream(&printer, &settings).await {
                                                Ok(_) => {
                                                    self::TelemetryManager::mark_auto_helper(&auto_managed, &printer.id).await;
                                                    info!(printer = %printer.id, "auto-started stream");
                                                }
                                                Err(e) => warn!(printer = %printer.id, "auto-start stream failed: {e}"),
                                            }
                                        }

                                        if telemetry.is_idle() && auto && matches!(workers.state(&printer.id).await, crate::models::StreamState::Running) {
                                            if let Ok(_) = workers.stop_stream(&printer.id).await {
                                                auto_managed.write().await.insert(printer.id.clone(), false);
                                                info!(printer = %printer.id, "auto-stopped stream");
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Ok(Event::Incoming(Incoming::Disconnect)) => {
                            warn!(printer = %printer.id, "MQTT server sent disconnect");
                            break;
                        }
                        Ok(Event::Incoming(Incoming::PingResp)) => {
                            debug!(printer = %printer.id, "MQTT ping response");
                        }
                        Ok(Event::Outgoing(Outgoing::PingReq)) => {
                            debug!(printer = %printer.id, "MQTT ping sent");
                        }
                        Ok(Event::Incoming(Incoming::PubAck(_))) | Ok(Event::Incoming(Incoming::PubRec(_))) => {
                            // Acknowledgments — normal, ignore
                        }
                        Ok(_) => {
                            // Other incoming events (SubAck handled above, etc.)
                        }
                        Err(e) => {
                            warn!(printer = %printer.id, "MQTT connection error: {e}");
                            break;
                        }
                    }
                }
            }
        }

        warn!(printer = %printer.id, "MQTT disconnected, waiting {}s before reconnect...", MQTT_RECONNECT_DELAY_SECS);        wait_before_reconnect(&mut cancel_rx).await;
    }
}

impl TelemetryManager {
    async fn mark_auto_helper(auto_managed: &Arc<RwLock<HashMap<String, bool>>>, printer_id: &str) {
        auto_managed
            .write()
            .await
            .insert(printer_id.to_string(), true);
    }
}

async fn wait_before_reconnect(cancel_rx: &mut watch::Receiver<bool>) {
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_secs(MQTT_RECONNECT_DELAY_SECS)) => {}
        _ = cancel_rx.changed() => {}
    }
}

async fn update_cache(
    cache: &Arc<RwLock<HashMap<String, PrinterTelemetry>>>,
    printer_id: &str,
    telemetry: PrinterTelemetry,
) {
    cache.write().await.insert(printer_id.to_string(), telemetry);
}

/// Merge incoming print data with existing telemetry.
/// P1 series only sends changed fields (delta updates), so we must preserve
/// existing values for fields not present in the incoming message.
fn merge_print(existing: Option<PrinterTelemetry>, print: &BambuPrint) -> PrinterTelemetry {
    let base = existing.unwrap_or_default();

    PrinterTelemetry {
        updated_at: Utc::now(),
        // Only overwrite fields that are present in the incoming message
        gcode_state: print.gcode_state.clone().or(base.gcode_state),
        task_name: print.subtask_name.clone()
            .or_else(|| print.gcode_file.clone())
            .or(base.task_name),
        progress: print.mc_percent.map(|p| p.clamp(0, 100) as u8).or(base.progress),
        remaining_minutes: print.mc_remaining_time.or(base.remaining_minutes),
        layer_num: print.layer_num.or(base.layer_num),
        total_layer_num: print.total_layer_num.or(base.total_layer_num),
        nozzle_temper: print.nozzle_temper.or(base.nozzle_temper),
        nozzle_target_temper: print.nozzle_target_temper.or(base.nozzle_target_temper),
        bed_temper: print.bed_temper.or(base.bed_temper),
        bed_target_temper: print.bed_target_temper.or(base.bed_target_temper),
        chamber_temper: print.chamber_temper.or(base.chamber_temper),
        print_error: print.print_error.or(base.print_error),
        speed_level: print.spd_lvl.or(base.speed_level),
        print_type: print.print_type.clone().or(base.print_type),
    }
}

#[derive(Debug)]
struct NoVerifier;

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