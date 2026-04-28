use crate::jobs::PrinterModel;
use crate::state::AppState;
use axum::extract::State;
use axum::http::{StatusCode, HeaderMap};
use axum::Json;
use serde::{Deserialize, Serialize};

/// Request to submit a print job
#[derive(Debug, Deserialize)]
pub struct SubmitJobRequest {
    pub student_name: String,
    pub class_period: String,
    pub filename: String,
    pub printer_model: String,
}

/// Job submission response
#[derive(Debug, Serialize)]
pub struct JobResponse {
    pub id: String,
    pub student_name: String,
    pub class_period: String,
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
            filename: job.filename.clone(),
            printer_model: job.printer_model.as_str().to_string(),
            status: format!("{:?}", job.status).to_lowercase(),
            progress_percent: job.progress_percent,
            created_at: job.created_at.to_rfc3339(),
        }
    }
}

/// POST /api/v2/jobs/submit (public - students can submit)
#[axum::debug_handler]
pub async fn submit_job(
    State(state): State<AppState>,
    Json(req): Json<SubmitJobRequest>,
) -> Result<(StatusCode, Json<JobResponse>), (StatusCode, Json<serde_json::Value>)> {
    // Parse printer model
    let model = match req.printer_model.to_lowercase().as_str() {
        "a1" => PrinterModel::A1,
        "a1mini" | "a1 mini" => PrinterModel::A1Mini,
        "p1p" => PrinterModel::P1P,
        "p1s" => PrinterModel::P1S,
        "x1c" => PrinterModel::X1C,
        "x1e" => PrinterModel::X1E,
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid printer model" })),
            ))
        }
    };

    // Generate a simple file path (in production, use file upload)
    let file_path = format!("/uploads/{}/{}", chrono::Utc::now().timestamp(), req.filename);

    match state.jobs
        .submit_job(
            req.student_name,
            req.class_period,
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
