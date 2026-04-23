use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterCredentials {
    pub username: String,
    pub access_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterStreamConfig {
    pub rtsp_port: u16,
    pub rtsp_path: String,
}

impl Default for PrinterStreamConfig {
    fn default() -> Self {
        Self {
            rtsp_port: 322,
            rtsp_path: "/streaming/live/1".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterRecord {
    pub id: String,
    pub host: String,
    pub device_id: String,
    pub credentials: PrinterCredentials,
    pub stream: PrinterStreamConfig,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertPrinterRequest {
    pub id: String,
    pub host: String,
    pub device_id: String,
    pub username: Option<String>,
    pub access_code: String,
    pub rtsp_port: Option<u16>,
    pub rtsp_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchUpsertRequest {
    pub printers: Vec<UpsertPrinterRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchUpsertResponse {
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub errors: Vec<BatchUpsertError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchUpsertError {
    pub id: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterSummaryResponse {
    pub id: String,
    pub host: String,
    pub device_id: String,
    pub updated_at: DateTime<Utc>,
    pub stream_state: StreamState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterDetailResponse {
    pub printer: PrinterRecord,
    pub stream_state: StreamState,
    pub stream_url: Option<String>,
    pub rtsp_source_url: String,
    pub rtsp_publish_url: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamState {
    Stopped,
    Starting,
    Running,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamActionResponse {
    pub printer_id: String,
    pub state: StreamState,
    pub url: Option<String>,
    pub rtsp_source_url: Option<String>,
    pub rtsp_publish_url: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub ok: bool,
    pub printers_registered: usize,
    pub streams_running: usize,
}
