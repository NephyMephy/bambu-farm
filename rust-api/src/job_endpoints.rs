use crate::gcode_validate;
use crate::jobs::PrinterModel;
use crate::state::AppState;
use axum::extract::{Multipart, State};
use axum::http::{StatusCode, HeaderMap};
use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Request to submit a print job (JSON API — no file upload)
#[derive(Debug, Deserialize)]
pub struct SubmitJobRequest {
    pub student_name: String,
    pub class_period: String,
    pub teacher: Option<String>,
    pub filename: String,
    pub printer_model: String,
}

/// Job submission response
#[derive(Debug, Serialize)]
pub struct JobResponse {
    pub id: String,
    pub student_name: String,
    pub class_period: String,
    pub teacher: String,
    pub filename: String,
    pub printer_model: String,
    pub status: String,
    pub progress_percent: u32,
    pub created_at: String,
}

impl JobResponse {
    fn from_job(job: &crate::jobs::PrintJob) -> Self {
        Self {
            id: job.id.clone(),
            student_name: job.student_name.clone(),
            class_period: job.class_period.clone(),
            teacher: job.teacher.clone(),
            filename: job.filename.clone(),
            printer_model: job.printer_model.as_str().to_string(),
            status: format!("{:?}", job.status).to_lowercase(),
            progress_percent: job.progress_percent,
            created_at: job.created_at.to_rfc3339(),
        }
    }
}

/// Parse a printer model string into a PrinterModel enum
fn parse_printer_model(model_str: &str) -> Result<PrinterModel, String> {
    match model_str.to_lowercase().as_str() {
        "a1" => Ok(PrinterModel::A1),
        "a1mini" | "a1 mini" => Ok(PrinterModel::A1Mini),
        "p1p" => Ok(PrinterModel::P1P),
        "p1s" => Ok(PrinterModel::P1S),
        "x1c" => Ok(PrinterModel::X1C),
        "x1e" => Ok(PrinterModel::X1E),
        _ => Err(format!("Invalid printer model: '{model_str}'")),
    }
}

/// POST /api/v2/jobs/submit (public - students can submit via JSON)
#[axum::debug_handler]
pub async fn submit_job(
    State(state): State<AppState>,
    Json(req): Json<SubmitJobRequest>,
) -> Result<(StatusCode, Json<JobResponse>), (StatusCode, Json<serde_json::Value>)> {
    let model = parse_printer_model(&req.printer_model).map_err(|e| {
        (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e })))
    })?;

    let teacher = req.teacher.unwrap_or_default();
    let file_path = format!("/uploads/{}/{}", chrono::Utc::now().timestamp(), req.filename);

    match state.jobs
        .submit_job(
            req.student_name,
            req.class_period,
            teacher,
            req.filename,
            model,
            file_path,
        )
        .await
    {
        Ok(job) => Ok((StatusCode::CREATED, Json(JobResponse::from_job(&job)))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e })),
        )),
    }
}

/// POST /api/v2/jobs/upload (public - students submit with file upload + gcode validation)
///
/// Accepts `multipart/form-data` with fields:
/// - `name` (text): Student name
/// - `class_period` (text): Class period
/// - `teacher` (text): Teacher name (Johnson or Friesen)
/// - `printer_model` (text): Printer model (A1, A1 Mini, P1S)
/// - `file` (file): The .gcode or .3mf sliced file
#[axum::debug_handler]
pub async fn upload_job(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<JobResponse>), (StatusCode, Json<serde_json::Value>)> {
    let mut student_name = None;
    let mut class_period = None;
    let mut teacher = None;
    let mut printer_model_str = None;
    let mut file_data = None;
    let mut file_name = None;

    // Parse multipart fields
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        warn!("multipart parse error: {e}");
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Failed to parse upload form" })),
        )
    })? {
        let field_name = field.name().unwrap_or_default().to_string();

        match field_name.as_str() {
            "name" => {
                student_name = Some(field.text().await.map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({ "error": format!("Invalid name field: {e}") })),
                    )
                })?);
            }
            "class_period" => {
                class_period = Some(field.text().await.map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({ "error": format!("Invalid class_period field: {e}") })),
                    )
                })?);
            }
            "teacher" => {
                teacher = Some(field.text().await.map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({ "error": format!("Invalid teacher field: {e}") })),
                    )
                })?);
            }
            "printer_model" => {
                printer_model_str = Some(field.text().await.map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({ "error": format!("Invalid printer_model field: {e}") })),
                    )
                })?);
            }
            "file" => {
                file_name = field.file_name().map(|s| s.to_string());
                file_data = Some(field.bytes().await.map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({ "error": format!("Failed to read file: {e}") })),
                    )
                })?);
            }
            _ => {
                // Ignore unknown fields
            }
        }
    }

    // Validate required fields
    let student_name = student_name.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Name is required" })),
        )
    })?;
    let class_period = class_period.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Class period is required" })),
        )
    })?;
    let teacher = teacher.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Teacher is required" })),
        )
    })?;
    let printer_model_str = printer_model_str.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Printer model is required" })),
        )
    })?;
    let file_data = file_data.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "File is required" })),
        )
    })?;
    let file_name = file_name.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "File must have a name" })),
        )
    })?;

    // Validate teacher
    let teacher_lower = teacher.to_lowercase();
    if teacher_lower != "johnson" && teacher_lower != "friesen" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Please select a valid teacher (Johnson or Friesen)" })),
        ));
    }

    // Parse printer model
    let model = parse_printer_model(&printer_model_str).map_err(|e| {
        (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e })))
    })?;

    // Validate gcode against printer model
    let validation = gcode_validate::validate_file(&file_data, &file_name, model.as_str());
    if !validation.is_valid {
        warn!(
            %file_name,
            detected = ?validation.detected_printer,
            "gcode validation failed"
        );
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": validation.error_message.unwrap_or_else(|| "Gcode validation failed. Please re-upload or contact a TA or Teacher.".to_string()),
                "detected_printer": validation.detected_printer,
            })),
        ));
    }

    info!(
        %file_name,
        detected = ?validation.detected_printer,
        "gcode validation passed"
    );

    // Save the file to disk
    let upload_dir = std::path::Path::new("uploads");
    if !upload_dir.exists() {
        std::fs::create_dir_all(upload_dir).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to create upload directory: {e}") })),
            )
        })?;
    }

    let job_id = crate::jobs::uuid_simple();
    let extension = file_name.rsplit('.').next().unwrap_or("gcode");
    let saved_name = format!("{job_id}.{extension}");
    let file_path = upload_dir.join(&saved_name);

    std::fs::write(&file_path, &file_data).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to save file: {e}") })),
        )
    })?;

    // Submit the job
    match state.jobs
        .submit_job(
            student_name,
            class_period,
            teacher,
            file_name,
            model,
            file_path.to_string_lossy().to_string(),
        )
        .await
    {
        Ok(job) => Ok((StatusCode::CREATED, Json(JobResponse::from_job(&job)))),
        Err(e) => {
            // Clean up saved file on error
            let _ = std::fs::remove_file(&file_path);
            Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            ))
        }
    }
}

/// GET /api/v2/jobs (list all jobs - staff only)
#[axum::debug_handler]
pub async fn list_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<JobResponse>>, (StatusCode, Json<serde_json::Value>)> {
    // Verify staff access
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "No token" }))))?;

    let user = state.users.verify_session(token, "127.0.0.1").await
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid token" }))))?;

    if !user.role.can_manage_queue() {
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Insufficient permissions" }))));
    }

    let jobs = state.jobs.list_jobs().await;
    Ok(Json(jobs.iter().map(JobResponse::from_job).collect()))
}

/// GET /api/v2/jobs/queue (get queued jobs - staff only)
#[axum::debug_handler]
pub async fn get_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<JobResponse>>, (StatusCode, Json<serde_json::Value>)> {
    // Verify staff access
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "No token" }))))?;

    let user = state.users.verify_session(token, "127.0.0.1").await
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid token" }))))?;

    if !user.role.can_manage_queue() {
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Insufficient permissions" }))));
    }

    let jobs = state.jobs.list_queued_jobs().await;
    Ok(Json(jobs.iter().map(JobResponse::from_job).collect()))
}

/// POST /api/v2/jobs/{id}/cancel (cancel a queued job - staff only)
#[axum::debug_handler]
pub async fn cancel_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> Result<Json<JobResponse>, (StatusCode, Json<serde_json::Value>)> {
    // Verify staff access
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "No token" }))))?;

    let user = state.users.verify_session(token, "127.0.0.1").await
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid token" }))))?;

    if !user.role.can_manage_queue() {
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Insufficient permissions" }))));
    }

    match state.jobs.cancel_job(&job_id).await {
        Ok(job) => Ok(Json(JobResponse::from_job(&job))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e })),
        )),
    }
}

/// POST /api/v2/jobs/{id}/dispatch (dispatch job to printer - staff only)
#[axum::debug_handler]
pub async fn dispatch_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path((job_id, printer_id)): axum::extract::Path<(String, String)>,
) -> Result<Json<JobResponse>, (StatusCode, Json<serde_json::Value>)> {
    // Verify staff access
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "No token" }))))?;

    let user = state.users.verify_session(token, "127.0.0.1").await
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid token" }))))?;

    if !user.role.can_dispatch_jobs() {
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Insufficient permissions" }))));
    }

    match state.jobs.dispatch_job(&job_id, printer_id).await {
        Ok(job) => Ok(Json(JobResponse::from_job(&job))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e })),
        )),
    }
}

/// GET /api/v2/jobs/{id} (get job status)
#[axum::debug_handler]
pub async fn get_job(
    State(state): State<AppState>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> Result<Json<JobResponse>, (StatusCode, Json<serde_json::Value>)> {
    match state.jobs.get_job(&job_id).await {
        Some(job) => Ok(Json(JobResponse::from_job(&job))),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Job not found" })),
        )),
    }
}
