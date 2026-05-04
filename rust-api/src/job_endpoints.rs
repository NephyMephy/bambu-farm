use crate::jobs::PrinterModel;
use crate::state::AppState;
use axum::extract::{Multipart, State, ConnectInfo};
use axum::http::{StatusCode, HeaderMap};
use axum::Json;
use std::net::SocketAddr;
use serde::{Deserialize, Serialize};
use tracing::{info, warn, error};

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

/// Extract client IP from request headers
fn get_client_ip(headers: &HeaderMap) -> String {
    // Try X-Forwarded-For first (when behind a proxy)
    if let Some(value) = headers.get("X-Forwarded-For") {
        if let Ok(s) = value.to_str() {
            // X-Forwarded-For can contain multiple IPs; take the first one
            if let Some(first_ip) = s.split(',').next() {
                return first_ip.trim().to_string();
            }
        }
    }
    
    // Try X-Real-IP (common proxy header)
    if let Some(value) = headers.get("X-Real-IP") {
        if let Ok(s) = value.to_str() {
            return s.to_string();
        }
    }
    
    // Fallback to 127.0.0.1 (local session verification)
    "127.0.0.1".to_string()
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

/// POST /api/v2/jobs/upload (public - students submit with file upload)
///
/// Accepts `multipart/form-data` with fields:
/// - `name` (text): Student name
/// - `class_period` (text): Class period
/// - `teacher` (text): Teacher name (Johnson or Friesen)
/// - `file` (file): The unsliced .stl or .3mf model file
/// - `printer_model` (text, optional): Printer model (defaults to A1)
#[axum::debug_handler]
pub async fn upload_job(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<JobResponse>), (StatusCode, Json<serde_json::Value>)> {
    info!("=== upload_job endpoint called ===");
    
    let mut student_name = None;
    let mut class_period = None;
    let mut teacher = None;
    let mut printer_model_str: Option<String> = Some("A1".to_string()); // Default to A1
    let mut file_data = None;
    let mut file_name = None;

    // Parse multipart fields
    let mut field_count = 0;
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        error!("[MULTIPART ERROR] Failed to read next field: {}", e);
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Failed to parse upload form: {e}") })),
        )
    })? {
        field_count += 1;
        let field_name = field.name().unwrap_or_default().to_string();
        let content_type = field.content_type().map(|c| c.to_string()).unwrap_or_else(|| "unknown".to_string());
        
        info!("[FIELD {}] name='{}', content_type='{}'", field_count, field_name, content_type);

        match field_name.as_str() {
            "name" => {
                match field.text().await {
                    Ok(value) => {
                        info!("[FIELD {}] ✓ name parsed: '{}'", field_count, value);
                        student_name = Some(value);
                    }
                    Err(e) => {
                        error!("[FIELD {}] ✗ Failed to parse name: {}", field_count, e);
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({ "error": format!("Invalid name field: {e}") })),
                        ));
                    }
                }
            }
            "class_period" => {
                match field.text().await {
                    Ok(value) => {
                        info!("[FIELD {}] ✓ class_period parsed: '{}'", field_count, value);
                        class_period = Some(value);
                    }
                    Err(e) => {
                        error!("[FIELD {}] ✗ Failed to parse class_period: {}", field_count, e);
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({ "error": format!("Invalid class_period field: {e}") })),
                        ));
                    }
                }
            }
            "teacher" => {
                match field.text().await {
                    Ok(value) => {
                        info!("[FIELD {}] ✓ teacher parsed: '{}'", field_count, value);
                        teacher = Some(value);
                    }
                    Err(e) => {
                        error!("[FIELD {}] ✗ Failed to parse teacher: {}", field_count, e);
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({ "error": format!("Invalid teacher field: {e}") })),
                        ));
                    }
                }
            }
            "printer_model" => {
                // Printer model is optional; defaults to A1
                match field.text().await {
                    Ok(value) => {
                        info!("[FIELD {}] ✓ printer_model parsed: '{}'", field_count, value);
                        printer_model_str = Some(value);
                    }
                    Err(e) => {
                        error!("[FIELD {}] ✗ Failed to parse printer_model (optional): {}", field_count, e);
                        // Don't fail, just use default
                    }
                }
            }
            "file" => {
                file_name = field.file_name().map(|s| s.to_string());
                info!("[FIELD {}] ✓ file_name detected: {:?}", field_count, file_name);
                
                match field.bytes().await {
                    Ok(bytes) => {
                        let file_size = bytes.len();
                        info!("[FIELD {}] ✓ file read successfully, size: {} bytes", field_count, file_size);
                        file_data = Some(bytes);
                    }
                    Err(e) => {
                        error!("[FIELD {}] ✗ Failed to read file bytes: {}", field_count, e);
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({ "error": format!("Failed to read file: {e}") })),
                        ));
                    }
                }
            }
            _ => {
                info!("[FIELD {}] ⊘ Ignoring unknown field: '{}'", field_count, field_name);
            }
        }
    }

    info!("[PARSE COMPLETE] Processed {} fields", field_count);

    // Validate required fields
    let student_name = student_name.ok_or_else(|| {
        error!("[VALIDATION] ✗ Missing required field: name");
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Name is required" })),
        )
    })?;
    info!("[VALIDATION] ✓ name = '{}'", student_name);
    
    let class_period = class_period.ok_or_else(|| {
        error!("[VALIDATION] ✗ Missing required field: class_period");
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Class period is required" })),
        )
    })?;
    info!("[VALIDATION] ✓ class_period = '{}'", class_period);
    
    let teacher = teacher.ok_or_else(|| {
        error!("[VALIDATION] ✗ Missing required field: teacher");
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Teacher is required" })),
        )
    })?;
    info!("[VALIDATION] ✓ teacher = '{}'", teacher);
    
    let file_data = file_data.ok_or_else(|| {
        error!("[VALIDATION] ✗ Missing required field: file");
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "File is required" })),
        )
    })?;
    info!("[VALIDATION] ✓ file_data received, size = {} bytes", file_data.len());
    
    let file_name = file_name.ok_or_else(|| {
        error!("[VALIDATION] ✗ Missing required field: file_name");
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "File must have a name" })),
        )
    })?;
    info!("[VALIDATION] ✓ file_name = '{}'", file_name);

    // Ensure printer_model_str has a value (should already default to "A1")
    let printer_model_str = printer_model_str.unwrap_or_else(|| "A1".to_string());
    info!("[VALIDATION] ✓ printer_model = '{}'", printer_model_str);

    // Validate teacher
    let teacher_lower = teacher.to_lowercase();
    if teacher_lower != "johnson" && teacher_lower != "friesen" {
        error!("[TEACHER VALIDATION] ✗ Invalid teacher: '{}'", teacher);
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Please select a valid teacher (Johnson or Friesen)" })),
        ));
    }
    info!("[TEACHER VALIDATION] ✓ Teacher is valid: '{}'", teacher);

    // Validate file extension — accept only .stl and .3mf (no gcode validation for unsliced models)
    let extension = file_name.rsplit('.').next().unwrap_or("").to_lowercase();
    info!("[FILE EXTENSION] Detected extension: '.{}'", extension);
    
    match extension.as_str() {
        "stl" | "3mf" => {
            // Accept unsliced STL and 3MF model files
            info!("[FILE EXTENSION] ✓ Accepted unsliced model file: {}", file_name);
        }
        "gcode" | "gco" => {
            error!("[FILE EXTENSION] ✗ Rejected gcode file: {}", file_name);
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": ".gcode and .gco files cannot be uploaded. Please upload the unsliced .stl or .3mf model file instead."
                })),
            ));
        }
        _ => {
            error!("[FILE EXTENSION] ✗ Unsupported file type: '.{}'", extension);
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("Unsupported file type '.{}'. Please upload a .stl or .3mf file.", extension)
                })),
            ));
        }
    }

    // Parse printer model
    let model = parse_printer_model(&printer_model_str).map_err(|e| {
        error!("[PRINTER MODEL] ✗ Failed to parse printer model '{}': {}", printer_model_str, e);
        (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e })))
    })?;
    info!("[PRINTER MODEL] ✓ Printer model parsed: {:?}", model);

    // Create the BambuTasks directory in the user's Documents folder
    let bambu_tasks_dir = get_bambu_tasks_dir().map_err(|e| {
        error!("[DIRECTORY] ✗ Failed to get BambuTasks directory: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
    })?;
    info!("[DIRECTORY] ✓ BambuTasks dir path: {}", bambu_tasks_dir.display());
    
    if !bambu_tasks_dir.exists() {
        info!("[DIRECTORY] Creating directory...");
        std::fs::create_dir_all(&bambu_tasks_dir).map_err(|e| {
            error!("[DIRECTORY] ✗ Failed to create directory: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to create BambuTasks directory: {e}") })),
            )
        })?;
        info!("[DIRECTORY] ✓ Directory created successfully");
    } else {
        info!("[DIRECTORY] ✓ Directory already exists");
    }

    // Submit the job first to get a job ID
    info!("[JOB SUBMISSION] Submitting job for student: '{}', period: '{}', teacher: '{}'", 
          student_name, class_period, teacher);
    let job = state.jobs
        .submit_job(
            student_name.clone(),
            class_period,
            teacher,
            file_name.clone(),
            model,
            String::new(), // placeholder, will update after rename
        )
        .await
        .map_err(|e| {
            error!("[JOB SUBMISSION] ✗ Failed to submit job: {}", e);
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
        })?;
    info!("[JOB SUBMISSION] ✓ Job created with ID: {}", job.id);

    // Rename and save the file: requestorname-jobid.extension
    info!("[FILE SAVE] Building renamed path for student '{}', job ID '{}'", student_name, job.id);
    let renamed_path = build_renamed_path(&student_name, &job.id, &extension).map_err(|e| {
        error!("[FILE SAVE] ✗ Failed to build renamed path: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
    })?;
    info!("[FILE SAVE] ✓ Renamed path: {}", renamed_path.display());

    info!("[FILE SAVE] Writing {} bytes to disk...", file_data.len());
    std::fs::write(&renamed_path, &file_data).map_err(|e| {
        error!("[FILE SAVE] ✗ Failed to write file to disk: {}", e);
        // Clean up the job if file write fails
        let _ = state.jobs.cancel_job(&job.id);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to save file: {e}") })),
        )
    })?;
    info!("[FILE SAVE] ✓ File written successfully");

    // Update the job's file_path to the actual saved location
    info!("[FILE UPDATE] Updating job file_path to: {}", renamed_path.display());
    state.jobs.update_file_path(&job.id, renamed_path.to_string_lossy().to_string()).await;
    info!("[FILE UPDATE] ✓ Job file_path updated");

    // Re-fetch the job with updated path
    let updated_job = state.jobs.get_job(&job.id).await.unwrap_or(job);
    
    info!("[SUCCESS] ✓ Upload complete! Job ID: {}, File: {}", updated_job.id, file_name);

    Ok((StatusCode::CREATED, Json(JobResponse::from_job(&updated_job))))
}

/// GET /api/v2/jobs (list all jobs - staff only)
#[axum::debug_handler]
pub async fn list_jobs(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Json<Vec<JobResponse>>, (StatusCode, Json<serde_json::Value>)> {
    info!("[LIST_JOBS] GET /api/v2/jobs called");
    
    let client_ip = addr.ip().to_string();
    info!("[LIST_JOBS] Client IP: {}", client_ip);
    
    // Verify staff access
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| {
            error!("[LIST_JOBS] ✗ No Bearer token found");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "No token" })))
        })?;
    
    info!("[LIST_JOBS] ✓ Token extracted: {}", token.chars().take(10).collect::<String>() + "...");

    let user = state.users.verify_session(token, &client_ip).await
        .ok_or_else(|| {
            error!("[LIST_JOBS] ✗ Invalid or expired token");
            (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid token" })))
        })?;
    
    info!("[LIST_JOBS] ✓ Token verified, user: '{}', role: {:?}", user.username, user.role);

    if !user.role.can_manage_queue() {
        error!("[LIST_JOBS] ✗ User '{}' lacks queue management permission", user.username);
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Insufficient permissions" }))));
    }

    let jobs = state.jobs.list_jobs().await;
    info!("[LIST_JOBS] ✓ Returning {} jobs", jobs.len());
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
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> Result<Json<JobResponse>, (StatusCode, Json<serde_json::Value>)> {
    info!("[COMPLETE_JOB] POST /api/v2/jobs/{}/complete called", job_id);
    
    let client_ip = addr.ip().to_string();
    info!("[COMPLETE_JOB] Client IP: {}", client_ip);
    
    // Verify staff access
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| {
            error!("[COMPLETE_JOB] ✗ No Bearer token found");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "No token" })))
        })?;

    let user = state.users.verify_session(token, &client_ip).await
        .ok_or_else(|| {
            error!("[COMPLETE_JOB] ✗ Invalid or expired token");
            (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid token" })))
        })?;
    
    info!("[COMPLETE_JOB] ✓ Token verified, user: '{}'", user.username);

    if !user.role.can_dispatch_jobs() {
        error!("[COMPLETE_JOB] ✗ User lacks dispatch permission");
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Insufficient permissions" }))));
    }

    match state.jobs.complete_job(&job_id).await {
        Ok(job) => {
            info!("[COMPLETE_JOB] ✓ Job {} marked complete", job_id);
            Ok(Json(JobResponse::from_job(&job)))
        }
        Err(e) => {
            error!("[COMPLETE_JOB] ✗ Error: {}", e);
            Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            ))
        }
    }
}

/// DELETE /api/v2/jobs/{id} (delete a completed job - staff only)
#[axum::debug_handler]
pub async fn delete_job(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    info!("[DELETE_JOB] DELETE /api/v2/jobs/{} called", job_id);
    
    let client_ip = addr.ip().to_string();
    info!("[DELETE_JOB] Client IP: {}", client_ip);
    
    // Verify staff access
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| {
            error!("[DELETE_JOB] ✗ No Bearer token found");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "No token" })))
        })?;

    let user = state.users.verify_session(token, &client_ip).await
        .ok_or_else(|| {
            error!("[DELETE_JOB] ✗ Invalid or expired token");
            (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid token" })))
        })?;
    
    info!("[DELETE_JOB] ✓ Token verified, user: '{}'", user.username);

    if !user.role.can_manage_queue() {
        error!("[DELETE_JOB] ✗ User lacks queue management permission");
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Insufficient permissions" }))));
    }

    match state.jobs.delete_job(&job_id).await {
        Ok(job) => {
            // Try to delete the file from disk
            let file_path = std::path::Path::new(&job.file_path);
            if file_path.exists() {
                info!("[DELETE_JOB] Deleting file: {}", job.file_path);
                if let Err(e) = std::fs::remove_file(file_path) {
                    warn!("[DELETE_JOB] ⚠ Failed to delete file: {}", e);
                }
            }
            info!("[DELETE_JOB] ✓ Job {} deleted", job.id);
            Ok(Json(serde_json::json!({ "message": "Job deleted", "id": job.id })))
        }
        Err(e) => {
            error!("[DELETE_JOB] ✗ Error deleting job: {}", e);
            Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            ))
        }
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
