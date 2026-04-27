use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Bambu printer model — determines streaming capabilities.
///
/// - **X1C / X1E**: RTSPS on port 322, path `/streaming/live/1` — FFmpeg → MediaMTX → WebRTC.
/// - **P1P / P1S / A1 / A1Mini**: Proprietary TCP JPEG streaming on port 6000 —
///   native MJPEG stream served directly by the API (no external bridge needed).
/// - **Unknown**: Falls back to user-provided RTSP settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrinterModel {
    Unknown,
    A1,
    A1Mini,
    P1P,
    P1S,
    X1C,
    X1E,
}

impl PrinterModel {
    /// Whether this model supports native RTSPS streaming (FFmpeg can connect directly).
    pub fn supports_rtsp(&self) -> bool {
        matches!(self, Self::X1C | Self::X1E)
    }

    /// Whether this model uses the proprietary TCP JPEG protocol on port 6000.
    pub fn uses_proprietary_stream(&self) -> bool {
        matches!(self, Self::A1 | Self::A1Mini | Self::P1P | Self::P1S)
    }

    /// Human-readable model name.
    #[allow(dead_code)]
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::A1 => "A1",
            Self::A1Mini => "A1 Mini",
            Self::P1P => "P1P",
            Self::P1S => "P1S",
            Self::X1C => "X1 Carbon",
            Self::X1E => "X1E",
        }
    }

    /// Default RTSP port for this model (322 for RTSPS models, 6000 for proprietary).
    pub fn default_rtsp_port(&self) -> u16 {
        if self.supports_rtsp() {
            322
        } else if self.uses_proprietary_stream() {
            6000
        } else {
            322
        }
    }

    /// Default RTSP path for this model.
    pub fn default_rtsp_path(&self) -> &'static str {
        if self.supports_rtsp() {
            "/streaming/live/1"
        } else if self.uses_proprietary_stream() {
            // Proprietary protocol — no RTSP path, but we store a placeholder
            "/streaming/live/1"
        } else {
            "/streaming/live/1"
        }
    }
}

impl Default for PrinterModel {
    fn default() -> Self {
        Self::Unknown
    }
}

impl std::fmt::Display for PrinterModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unknown => "unknown",
            Self::A1 => "a1",
            Self::A1Mini => "a1mini",
            Self::P1P => "p1p",
            Self::P1S => "p1s",
            Self::X1C => "x1c",
            Self::X1E => "x1e",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterCredentials {
    pub username: String,
    pub access_code: String,
}

/// Stream type — distinguishes RTSPS (FFmpeg → MediaMTX) from proprietary (native MJPEG).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamType {
    /// RTSPS — FFmpeg relays to MediaMTX for WebRTC (X1C, X1E).
    Rtsp,
    /// Proprietary TCP JPEG — native MJPEG stream served by the API (P1P, P1S, A1, A1 Mini).
    Proprietary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterStreamConfig {
    pub rtsp_port: u16,
    pub rtsp_path: String,
    /// Stream type derived from the printer model.
    pub stream_type: StreamType,
}

impl PrinterStreamConfig {
    /// Create config appropriate for the given model.
    pub fn for_model(model: PrinterModel) -> Self {
        let stream_type = if model.supports_rtsp() {
            StreamType::Rtsp
        } else if model.uses_proprietary_stream() {
            StreamType::Proprietary
        } else {
            // Unknown model — assume RTSPS (user can override)
            StreamType::Rtsp
        };

        Self {
            rtsp_port: model.default_rtsp_port(),
            rtsp_path: model.default_rtsp_path().to_string(),
            stream_type,
        }
    }
}

impl Default for PrinterStreamConfig {
    fn default() -> Self {
        Self::for_model(PrinterModel::Unknown)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterRecord {
    pub id: String,
    pub host: String,
    pub device_id: String,
    pub model: PrinterModel,
    pub credentials: PrinterCredentials,
    pub stream: PrinterStreamConfig,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterTelemetrySnapshot {
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

impl From<crate::telemetry::PrinterTelemetry> for PrinterTelemetrySnapshot {
    fn from(value: crate::telemetry::PrinterTelemetry) -> Self {
        Self {
            updated_at: value.updated_at,
            gcode_state: value.gcode_state,
            task_name: value.task_name,
            progress: value.progress,
            remaining_minutes: value.remaining_minutes,
            layer_num: value.layer_num,
            total_layer_num: value.total_layer_num,
            nozzle_temper: value.nozzle_temper,
            nozzle_target_temper: value.nozzle_target_temper,
            bed_temper: value.bed_temper,
            bed_target_temper: value.bed_target_temper,
            chamber_temper: value.chamber_temper,
            print_error: value.print_error,
            speed_level: value.speed_level,
            print_type: value.print_type,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertPrinterRequest {
    pub id: String,
    pub host: String,
    pub device_id: String,
    /// Printer model (e.g. "x1c", "p1s", "a1mini"). Determines stream type and defaults.
    pub model: Option<PrinterModel>,
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
    pub model: PrinterModel,
    pub stream_type: StreamType,
    pub updated_at: DateTime<Utc>,
    pub stream_state: StreamState,
    pub stream_url: Option<String>,
    pub telemetry: Option<PrinterTelemetrySnapshot>,
    pub stream_auto_managed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterDetailResponse {
    pub printer: PrinterRecord,
    pub stream_state: StreamState,
    pub stream_url: Option<String>,
    pub rtsp_source_url: String,
    pub rtsp_publish_url: String,
    pub telemetry: Option<PrinterTelemetrySnapshot>,
    pub stream_auto_managed: bool,
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
pub struct BatchStreamResponse {
    pub started: Vec<String>,
    pub stopped: Vec<String>,
    pub errors: Vec<BatchStreamError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchStreamError {
    pub id: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub ok: bool,
    pub printers_registered: usize,
    pub streams_running: usize,
}
