use crate::config::Settings;
use crate::models::PrinterRecord;
use crate::stream::WorkerManager;
use chrono::{DateTime, Utc};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS, TlsConfiguration, Transport};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{watch, Mutex, RwLock};
use tokio::time::{Duration, MissedTickBehavior};
use tokio_rustls::rustls;
use tracing::{error, info, warn};

const MQTT_KEEP_ALIVE_SECS: u64 = 30;
const MQTT_RECONNECT_DELAY_SECS: u64 = 5;
const MQTT_FULL_STATUS_INTERVAL_SECS: u64 = 600;

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
    let mut options = MqttOptions::new(
        format!("bambu-live-api-{}", printer.id),
        printer.host.clone(),
        8883,
    );
    options.set_credentials(printer.credentials.username.clone(), printer.credentials.access_code.clone());
    options.set_keep_alive(Duration::from_secs(MQTT_KEEP_ALIVE_SECS));
    options.set_transport(Transport::tls_with_config(TlsConfiguration::Rustls(Arc::new(build_tls_config()))));
    options
}

fn report_topic(printer: &PrinterRecord) -> String {
    format!("device/{}/report", printer.device_id)
}

fn request_topic(printer: &PrinterRecord) -> String {
    format!("device/{}/request", printer.device_id)
}

async fn publish_full_status(client: &AsyncClient, printer: &PrinterRecord) -> Result<(), String> {
    let payload = serde_json::json!({
        "pushing": {
            "command": "pushall",
            "push_target": 1,
            "version": 1,
            "sequence_id": "1"
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
        let connected = client
            .subscribe(report_topic.clone(), QoS::AtMostOnce)
            .await;

        if let Err(e) = connected {
            error!(printer = %printer.id, "failed to subscribe to telemetry topic: {e}");
            wait_before_reconnect(&mut cancel_rx).await;
            continue;
        }

        info!(printer = %printer.id, topic = %report_topic, "telemetry connected");

        let refresh_secs = MQTT_FULL_STATUS_INTERVAL_SECS;
        let mut refresh = tokio::time::interval(Duration::from_secs(refresh_secs));
        refresh.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let _ = publish_full_status(&client, &printer).await;

        loop {
            tokio::select! {
                _ = cancel_rx.changed() => {
                    info!(printer = %printer.id, "telemetry disconnect requested");
                    return;
                }
                _ = refresh.tick() => {
                    if let Err(e) = publish_full_status(&client, &printer).await {
                        warn!(printer = %printer.id, "failed to refresh telemetry: {e}");
                    }
                }
                event = eventloop.poll() => {
                    match event {
                        Ok(Event::Incoming(Incoming::Publish(packet))) if packet.topic == report_topic => {
                            if let Ok(body) = std::str::from_utf8(&packet.payload) {
                                if let Ok(message) = serde_json::from_str::<BambuMessage>(body) {
                                    if let Some(print) = message.print {
                                        let telemetry = map_print(&print);
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
                        Ok(_) => {}
                        Err(e) => {
                            warn!(printer = %printer.id, "telemetry connection error: {e}");
                            break;
                        }
                    }
                }
            }
        }

        wait_before_reconnect(&mut cancel_rx).await;
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

fn map_print(print: &BambuPrint) -> PrinterTelemetry {
    PrinterTelemetry {
        updated_at: Utc::now(),
        gcode_state: print.gcode_state.clone(),
        task_name: print.subtask_name.clone().or_else(|| print.gcode_file.clone()),
        progress: print.mc_percent.map(|p| p.clamp(0, 100) as u8),
        remaining_minutes: print.mc_remaining_time,
        layer_num: print.layer_num,
        total_layer_num: print.total_layer_num,
        nozzle_temper: print.nozzle_temper,
        nozzle_target_temper: print.nozzle_target_temper,
        bed_temper: print.bed_temper,
        bed_target_temper: print.bed_target_temper,
        chamber_temper: print.chamber_temper,
        print_error: print.print_error,
        speed_level: print.spd_lvl,
        print_type: print.print_type.clone(),
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