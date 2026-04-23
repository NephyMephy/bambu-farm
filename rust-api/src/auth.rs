use axum::extract::Request;
use axum::http::{header::AUTHORIZATION, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

use crate::state::AppState;

pub async fn api_key_middleware(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let expected = format!("Bearer {}", state.settings.api_key);
    let supplied = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|val| val.to_str().ok())
        .unwrap_or_default();

    if supplied != expected {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(next.run(req).await)
}
