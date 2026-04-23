#[derive(Debug, Clone)]
pub struct Settings {
    pub bind_addr: String,
    pub api_key: String,
    pub ffmpeg_bin: String,
    pub mediamtx_rtsp_publish: String,
    pub webrtc_url_template: String,
    pub max_concurrent_streams: usize,
    pub printers_file: Option<String>,
}

impl Settings {
    pub fn from_env() -> Self {
        Self {
            bind_addr: std::env::var("API_BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
            api_key: std::env::var("API_KEY").unwrap_or_else(|_| "change-me".to_string()),
            ffmpeg_bin: std::env::var("FFMPEG_BIN").unwrap_or_else(|_| "ffmpeg".to_string()),
            mediamtx_rtsp_publish: std::env::var("MEDIAMTX_RTSP_PUBLISH")
                .unwrap_or_else(|_| "rtsp://127.0.0.1:8554".to_string()),
            webrtc_url_template: std::env::var("WEBRTC_URL_TEMPLATE")
                .unwrap_or_else(|_| "http://127.0.0.1:8889/{id}/".to_string()),
            max_concurrent_streams: std::env::var("MAX_CONCURRENT_STREAMS")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(25),
            printers_file: std::env::var("PRINTERS_FILE").ok(),
        }
    }

    pub fn webrtc_url_for(&self, printer_id: &str) -> String {
        self.webrtc_url_template.replace("{id}", printer_id)
    }
}
