use crate::config::Settings;
use crate::models::{PrinterModel, PrinterRecord, PrinterStreamConfig};
use crate::stream::WorkerManager;
use crate::telemetry::TelemetryManager;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

#[derive(Clone)]
pub struct AppState {
    pub settings: Settings,
    pub printers: Arc<RwLock<HashMap<String, PrinterRecord>>>,
    pub workers: Arc<WorkerManager>,
    pub telemetry: Arc<TelemetryManager>,
}

impl AppState {
    pub fn new(settings: Settings) -> Self {
        let workers = WorkerManager::new(settings.max_concurrent_streams);
        let printers = if let Some(ref path) = settings.printers_file {
            match Self::load_printers_file(path) {
                Ok(p) => {
                    info!("loaded {} printer(s) from {}", p.len(), path);
                    p
                }
                Err(e) => {
                    warn!("failed to load printers file {}: {e}", path);
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };

        Self {
            settings,
            printers: Arc::new(RwLock::new(printers)),
            workers: Arc::new(workers),
            telemetry: Arc::new(TelemetryManager::new()),
        }
    }

    pub async fn start_telemetry(&self) {
        let printers: Vec<PrinterRecord> = self.printers.read().await.values().cloned().collect();
        for printer in printers {
            self.telemetry
                .register_printer(printer, self.settings.clone(), self.workers.clone())
                .await;
        }
    }

    fn load_printers_file(path: &str) -> Result<HashMap<String, PrinterRecord>, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("read error: {e}"))?;

        let defs: Vec<PrinterFileEntry> = if path.ends_with(".json") {
            serde_json::from_str(&content).map_err(|e| format!("JSON parse error: {e}"))?
        } else {
            // Try JSON first, then YAML-like (just JSON for now)
            serde_json::from_str(&content).map_err(|e| format!("parse error: {e}"))?
        };

        let now = chrono::Utc::now();
        let mut map = HashMap::new();
        for def in defs {
            let model = def.model.unwrap_or(PrinterModel::Unknown);
            let defaults = PrinterStreamConfig::for_model(model);

            let record = PrinterRecord {
                id: def.id.clone(),
                host: def.host,
                device_id: def.device_id,
                model,
                credentials: crate::models::PrinterCredentials {
                    username: def.username.unwrap_or_else(|| "bblp".to_string()),
                    access_code: def.access_code,
                },
                stream: PrinterStreamConfig {
                    rtsp_port: def.rtsp_port.unwrap_or(defaults.rtsp_port),
                    rtsp_path: def.rtsp_path.unwrap_or(defaults.rtsp_path),
                    stream_type: defaults.stream_type,
                },
                created_at: now,
                updated_at: now,
            };
            map.insert(def.id, record);
        }
        Ok(map)
    }
}

/// A printer definition as it appears in the printers.json config file.
#[derive(Debug, serde::Deserialize)]
struct PrinterFileEntry {
    pub id: String,
    pub host: String,
    pub device_id: String,
    pub model: Option<PrinterModel>,
    pub username: Option<String>,
    pub access_code: String,
    pub rtsp_port: Option<u16>,
    pub rtsp_path: Option<String>,
}
