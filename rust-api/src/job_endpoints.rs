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
    pub file_path: String,
    pub created_at: String,
    pub completed_at: Option<String>,
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
            file_path: job.file_path.clone(),
            created_at: job.created_at.to_rfc3339(),
            completed_at: job.completed_at.map(|t| t.to_rfc3339()),
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

/// Get the BambuTasks folder path inside the user's Documents directory.
/// - Windows: `C:\Users\<user>\Documents\BambuTasks`
/// - macOS: `~/Documents/BambuTasks`
/// - Linux: `~/Documents/BambuTasks`
fn get_bambu_tasks_dir() -> Result<std::path::PathBuf, String> {
    let docs_dir = dirs::document_dir()
        .ok_or_else(|| "Could not locate user Documents folder".to_string())?;
    Ok(docs_dir.join("BambuTasks"))
}

/// Build the renamed file path: BambuTasks/<sanitized_name>-<jobid>.<extension>
fn build_renamed_path(student_name: &str, job_id: &str, extension: &str) -> Result<std::path::PathBuf, String> {
    let base_dir = get_bambu_tasks_dir()?;
    // Sanitize student name: replace spaces and special chars with underscores
    let safe_name: String = student_name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .collect();
    let filename = format!("{}-{}.{}", safe_name, job_id, extension);
    Ok(base_dir.join(&filename))
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
    let file_path = format!("BambuTasks/{}", req.filename);

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

    // Validate file extension — reject .stl and non-gcode .3mf early
    let extension = file_name.rsplit('.').next().unwrap_or("").to_lowercase();
    match extension.as_str() {
        "stl" => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "STL files cannot be printed directly. Please slice your model in Bambu Studio first, then upload the .gcode or .3mf file."
                })),
            ));
        }
        "3mf" => {
            // Validate that this is a sliced 3MF (contains gcode), not a raw model 3MF
            let precheck = gcode_validate::validate_file(&file_data, &file_name, "A1");
            if !precheck.is_valid {
                if let Some(ref msg) = precheck.error_message {
                    if msg.contains("No gcode found") || msg.contains("not a valid Bambu Studio slice") {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({
                                "error": "This .3mf file is an unsliced model, not a print-ready file. Please open it in Bambu Studio, slice it, then export the sliced file (File → Export → Export plate sliced file) and upload that instead."
                            })),
                        ));
                    }
                }
            }
        }
        "gcode" | "gco" => {
            // Valid — will be validated below against printer model
        }
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("Unsupported file type '.{}'. Please upload a .gcode or sliced .3mf file.", extension)
                })),
            ));
        }
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

    // Create the BambuTasks directory in the user's Documents folder
    let bambu_tasks_dir = get_bambu_tasks_dir().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
    })?;
    if !bambu_tasks_dir.exists() {
        std::fs::create_dir_all(&bambu_tasks_dir).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to create BambuTasks directory: {e}") })),
            )
        })?;
    }

    // Submit the job first to get a job ID
    let job = state.jobs
        .submit_job(
            student_name.clone(),
            class_period,
            teacher,
            file_name,
            model,
            String::new(), // placeholder, will update after rename
        )
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
        })?;

    // Rename and save the file: requestorname-jobid.extension
    let renamed_path = build_renamed_path(&student_name, &job.id, &extension).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
    })?;

    std::fs::write(&renamed_path, &file_data).map_err(|e| {
        // Clean up the job if file write fails
        let _ = state.jobs.cancel_job(&job.id);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to save file: {e}") })),
        )
    })?;

    // Update the job's file_path to the actual saved location
    state.jobs.update_file_path(&job.id, renamed_path.to_string_lossy().to_string()).await;

    // Re-fetch the job with updated path
    let updated_job = state.jobs.get_job(&job.id).await.unwrap_or(job);

    Ok((StatusCode::CREATED, Json(JobResponse::from_job(&updated_job))))
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

/// POST /api/v2/jobs/{id}/complete (mark job as completed - staff only)
#[axum::debug_handler]
pub async fn complete_job(
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

    if !user.role.can_dispatch_jobs() {
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Insufficient permissions" }))));
    }

    match state.jobs.complete_job(&job_id).await {
        Ok(job) => Ok(Json(JobResponse::from_job(&job))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e })),
        )),
    }
}

/// DELETE /api/v2/jobs/{id} (delete a completed job - staff only)
#[axum::debug_handler]
pub async fn delete_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
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

    match state.jobs.delete_job(&job_id).await {
        Ok(job) => {
            // Try to delete the file from disk
            let file_path = std::path::Path::new(&job.file_path);
            if file_path.exists() {
                if let Err(e) = std::fs::remove_file(file_path) {
                    warn!(path = %job.file_path, error = %e, "failed to delete job file");
                }
            }
            Ok(Json(serde_json::json!({ "message": "Job deleted", "id": job.id })))
        }
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

/// GET /api/v2/jobs/public/queue (public — no auth required, limited fields)
/// Returns queued jobs with only the info students need to see their position.
#[axum::debug_handler]
pub async fn public_queue(
    State(state): State<AppState>,
) -> Json<Vec<serde_json::Value>> {
    let jobs = state.jobs.list_queued_jobs().await;
    Json(jobs
        .iter()
        .enumerate()
        .map(|(i, j)| {
            serde_json::json!({
                "position": i + 1,
                "student_name": j.student_name,
                "class_period": j.class_period,
                "teacher": j.teacher,
                "status": format!("{:?}", j.status).to_lowercase(),
                "created_at": j.created_at.to_rfc3339(),
            })
        })
        .collect())
}
