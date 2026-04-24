use crate::models::{
    BatchStreamError, BatchStreamResponse, BatchUpsertError, BatchUpsertRequest,
    BatchUpsertResponse, HealthResponse, PrinterDetailResponse, PrinterModel, PrinterRecord,
    PrinterSummaryResponse, StreamActionResponse, StreamState, StreamType, UpsertPrinterRequest,
};
use crate::state::AppState;
use crate::stream::WorkerManager;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;

fn is_safe_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn normalize_rtsp_path(path: Option<String>) -> String {
    let p = path.unwrap_or_else(|| "/streaming/live/1".to_string());
    if p.starts_with('/') {
        p
    } else {
        format!("/{p}")
    }
}

/// Build a `PrinterStreamConfig` from the request, using the model to set defaults.
fn build_stream_config(req: &UpsertPrinterRequest) -> crate::models::PrinterStreamConfig {
    let model = req.model.unwrap_or(PrinterModel::Unknown);
    let defaults = crate::models::PrinterStreamConfig::for_model(model);

    crate::models::PrinterStreamConfig {
        rtsp_port: req.rtsp_port.unwrap_or(defaults.rtsp_port),
        rtsp_path: normalize_rtsp_path(req.rtsp_path.clone().or_else(|| Some(defaults.rtsp_path))),
        stream_type: defaults.stream_type,
    }
}

/// Get the appropriate stream URL for a printer based on its stream type.
/// - RTSPS models: WebRTC URL (MediaMTX)
/// - Proprietary models: MJPEG stream URL (served by this API)
fn stream_url_for(printer: &PrinterRecord, settings: &crate::config::Settings) -> String {
    match printer.stream.stream_type {
        StreamType::Rtsp => settings.webrtc_url_for(&printer.id),
        StreamType::Proprietary => format!("/v1/printers/{}/stream/mjpeg", printer.id),
    }
}

pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let printers_registered = state.printers.read().await.len();
    let streams_running = state.workers.running_count().await;
    Json(HealthResponse {
        ok: true,
        printers_registered,
        streams_running,
    })
}

pub async fn upsert_printer(
    State(state): State<AppState>,
    Json(req): Json<UpsertPrinterRequest>,
) -> impl IntoResponse {
    if !is_safe_id(&req.id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "id must be [A-Za-z0-9_-]"})),
        )
            .into_response();
    }
    if req.host.trim().is_empty() || req.device_id.trim().is_empty() || req.access_code.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "host, device_id and access_code are required"})),
        )
            .into_response();
    }

    let now = Utc::now();

    let mut printers = state.printers.write().await;
    let created_at = printers
        .get(&req.id)
        .map(|p| p.created_at)
        .unwrap_or(now);

    let model = req.model.unwrap_or(PrinterModel::Unknown);
    let stream = build_stream_config(&req);

    let record = PrinterRecord {
        id: req.id.clone(),
        host: req.host,
        device_id: req.device_id,
        model,
        credentials: crate::models::PrinterCredentials {
            username: req.username.unwrap_or_else(|| "bblp".to_string()),
            access_code: req.access_code,
        },
        stream,
        created_at,
        updated_at: now,
    };

    printers.insert(record.id.clone(), record.clone());

    (StatusCode::OK, Json(record)).into_response()
}

pub async fn batch_upsert_printers(
    State(state): State<AppState>,
    Json(req): Json<BatchUpsertRequest>,
) -> impl IntoResponse {
    let mut created = Vec::new();
    let mut updated = Vec::new();
    let mut errors = Vec::new();

    let now = Utc::now();
    let mut printers = state.printers.write().await;

    for printer_req in req.printers {
        if !is_safe_id(&printer_req.id) {
            errors.push(BatchUpsertError {
                id: printer_req.id,
                error: "id must be [A-Za-z0-9_-]".to_string(),
            });
            continue;
        }
        if printer_req.host.trim().is_empty()
            || printer_req.device_id.trim().is_empty()
            || printer_req.access_code.trim().is_empty()
        {
            errors.push(BatchUpsertError {
                id: printer_req.id,
                error: "host, device_id and access_code are required".to_string(),
            });
            continue;
        }

        let is_new = !printers.contains_key(&printer_req.id);
        let created_at = printers
            .get(&printer_req.id)
            .map(|p| p.created_at)
            .unwrap_or(now);

        let model = printer_req.model.unwrap_or(PrinterModel::Unknown);
        let stream = build_stream_config(&printer_req);

        let record = PrinterRecord {
            id: printer_req.id.clone(),
            host: printer_req.host,
            device_id: printer_req.device_id,
            model,
            credentials: crate::models::PrinterCredentials {
                username: printer_req.username.unwrap_or_else(|| "bblp".to_string()),
                access_code: printer_req.access_code,
            },
            stream,
            created_at,
            updated_at: now,
        };

        printers.insert(record.id.clone(), record);

        if is_new {
            created.push(printer_req.id);
        } else {
            updated.push(printer_req.id);
        }
    }

    (StatusCode::OK, Json(BatchUpsertResponse { created, updated, errors })).into_response()
}

pub async fn list_printers(State(state): State<AppState>) -> impl IntoResponse {
    let printers: Vec<PrinterRecord> = state.printers.read().await.values().cloned().collect();
    let mut out = Vec::with_capacity(printers.len());

    for p in printers {
        let stream_state = state.workers.state(&p.id).await;
        let stream_url = if matches!(stream_state, StreamState::Running | StreamState::Starting) {
            Some(stream_url_for(&p, &state.settings))
        } else {
            None
        };
        out.push(PrinterSummaryResponse {
            id: p.id.clone(),
            host: p.host,
            device_id: p.device_id,
            model: p.model,
            stream_type: p.stream.stream_type,
            updated_at: p.updated_at,
            stream_state,
            stream_url,
        });
    }

    Json(out)
}

pub async fn get_printer(
    Path(printer_id): Path<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, StatusCode> {
    let printer = state
        .printers
        .read()
        .await
        .get(&printer_id)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;

    let stream_state = state.workers.state(&printer_id).await;
    let stream_url = if matches!(stream_state, StreamState::Running | StreamState::Starting) {
        Some(stream_url_for(&printer, &state.settings))
    } else {
        None
    };

    Ok(Json(PrinterDetailResponse {
        rtsp_source_url: WorkerManager::rtsp_source_url(&printer),
        rtsp_publish_url: WorkerManager::rtsp_publish_url(&printer, &state.settings),
        printer,
        stream_state,
        stream_url,
    }))
}

pub async fn start_stream(
    Path(printer_id): Path<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let printer = state
        .printers
        .read()
        .await
        .get(&printer_id)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "printer not found".to_string()))?;

    let stream_state = state
        .workers
        .start_stream(&printer, &state.settings)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;

    Ok(Json(StreamActionResponse {
        printer_id,
        state: stream_state,
        url: Some(stream_url_for(&printer, &state.settings)),
        rtsp_source_url: Some(WorkerManager::rtsp_source_url(&printer)),
        rtsp_publish_url: Some(WorkerManager::rtsp_publish_url(&printer, &state.settings)),
        message: "stream start requested".to_string(),
    }))
}

pub async fn stop_stream(
    Path(printer_id): Path<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    state
        .printers
        .read()
        .await
        .get(&printer_id)
        .ok_or((StatusCode::NOT_FOUND, "printer not found".to_string()))?;

    let stream_state = state
        .workers
        .stop_stream(&printer_id)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;

    Ok(Json(StreamActionResponse {
        printer_id,
        state: stream_state,
        url: None,
        rtsp_source_url: None,
        rtsp_publish_url: None,
        message: "stream stopped".to_string(),
    }))
}

pub async fn delete_printer(
    Path(printer_id): Path<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let mut printers = state.printers.write().await;
    let removed = printers.remove(&printer_id);
    if removed.is_none() {
        return Err((StatusCode::NOT_FOUND, "printer not found".to_string()));
    }

    // Stop any running stream for this printer
    drop(printers); // release write lock before acquiring worker lock
    let _ = state.workers.stop_stream(&printer_id).await;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn stream_url(
    Path(printer_id): Path<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let printer = state
        .printers
        .read()
        .await
        .get(&printer_id)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "printer not found".to_string()))?;

    let stream_state = state.workers.state(&printer_id).await;
    let url = if matches!(stream_state, StreamState::Running | StreamState::Starting) {
        Some(stream_url_for(&printer, &state.settings))
    } else {
        None
    };

    Ok(Json(StreamActionResponse {
        printer_id: printer_id.clone(),
        state: stream_state,
        url,
        rtsp_source_url: state
            .printers
            .read()
            .await
            .get(&printer_id)
            .map(WorkerManager::rtsp_source_url),
        rtsp_publish_url: state
            .printers
            .read()
            .await
            .get(&printer_id)
            .map(|p| WorkerManager::rtsp_publish_url(p, &state.settings)),
        message: "stream URL lookup complete".to_string(),
    }))
}

pub async fn start_all_streams(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let printers: Vec<PrinterRecord> = state.printers.read().await.values().cloned().collect();
    let workers = state.workers.clone();
    let settings = state.settings.clone();
    let mut started = Vec::new();
    let mut errors = Vec::new();

    let mut joins = tokio::task::JoinSet::new();
    for printer in printers {
        let id = printer.id.clone();
        let workers = workers.clone();
        let settings = settings.clone();
        joins.spawn(async move {
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                workers.start_stream(&printer, &settings),
            )
            .await;

            match result {
                Ok(Ok(_)) => Ok(id),
                Ok(Err(e)) => Err(BatchStreamError { id, error: e }),
                Err(_) => Err(BatchStreamError {
                    id,
                    error: "start timed out".to_string(),
                }),
            }
        });
    }

    while let Some(result) = joins.join_next().await {
        match result {
            Ok(Ok(id)) => started.push(id),
            Ok(Err(err)) => errors.push(err),
            Err(e) => errors.push(BatchStreamError {
                id: "<task>".to_string(),
                error: format!("start task failed: {e}"),
            }),
        }
    }

    Ok(Json(BatchStreamResponse {
        started,
        stopped: Vec::new(),
        errors,
    }))
}

pub async fn stop_all_streams(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let printers = state.printers.read().await;
    let mut stopped = Vec::new();
    let mut errors = Vec::new();

    for id in printers.keys() {
        match state.workers.stop_stream(id).await {
            Ok(_) => stopped.push(id.clone()),
            Err(e) => errors.push(BatchStreamError {
                id: id.clone(),
                error: e,
            }),
        }
    }

    Ok(Json(BatchStreamResponse {
        started: Vec::new(),
        stopped,
        errors,
    }))
}

pub async fn dashboard() -> impl IntoResponse {
    let html = include_str!("dashboard.html");
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
}

/// Get the latest JPEG snapshot for a proprietary stream printer.
/// Returns a single JPEG image — useful for dashboard thumbnails.
pub async fn stream_snapshot(
    Path(printer_id): Path<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let printer = state
        .printers
        .read()
        .await
        .get(&printer_id)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "printer not found".to_string()))?;

    if printer.stream.stream_type != StreamType::Proprietary {
        return Err((
            StatusCode::BAD_REQUEST,
            "snapshot only available for proprietary stream models (P1P, P1S, A1, A1 Mini)".to_string(),
        ));
    }

    let frame = state.workers.latest_frame(&printer_id).await
        .ok_or((StatusCode::NOT_FOUND, "no frame available — stream may not be running".to_string()))?;

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "image/jpeg")],
        (*frame).clone(),
    ))
}

/// MJPEG stream endpoint for proprietary stream printers.
/// Serves a continuous multipart JPEG stream that browsers can display in <img> tags.
pub async fn stream_mjpeg(
    Path(printer_id): Path<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let printer = state
        .printers
        .read()
        .await
        .get(&printer_id)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "printer not found".to_string()))?;

    if printer.stream.stream_type != StreamType::Proprietary {
        return Err((
            StatusCode::BAD_REQUEST,
            "MJPEG stream only available for proprietary stream models (P1P, P1S, A1, A1 Mini)".to_string(),
        ));
    }

    let stream_state = state.workers.state(&printer_id).await;
    if !matches!(stream_state, StreamState::Running | StreamState::Starting) {
        return Err((StatusCode::NOT_FOUND, "stream is not running".to_string()));
    }

    let workers = state.workers.clone();
    let pid = printer_id.clone();

    let body = Body::from_stream(async_stream::stream! {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(200));
        loop {
            interval.tick().await;
            if let Some(frame) = workers.latest_frame(&pid).await {
                let header = format!(
                    "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                    frame.len()
                );
                yield Ok::<_, std::convert::Infallible>(bytes::Bytes::from(header));
                yield Ok::<_, std::convert::Infallible>(bytes::Bytes::from((*frame).clone()));
                yield Ok::<_, std::convert::Infallible>(bytes::Bytes::from("\r\n"));
            }
        }
    });

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "multipart/x-mixed-replace; boundary=frame"),
            (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate"),
        ],
        body,
    ))
}
