use crate::auth::{LoginRequest, LoginResponse, Role};
use crate::state::AppState;
use axum::extract::{State, ConnectInfo, Path};
use axum::http::{StatusCode, HeaderMap};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: String,
    pub username: String,
    pub role: String,
    pub email: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Serialize)]
pub struct ListUsersResponse {
    pub users: Vec<UserResponse>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub email: Option<String>,
    pub password: String,
    pub role: String,
}

/// POST /auth/login
pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<serde_json::Value>)> {
    let ip = addr.ip().to_string();

    let user = state.users.authenticate(&req.username, &req.password, &ip).await
        .map_err(|e| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": e }))))?;

    let session = state.users.create_session(user.id.clone(), ip).await;
    Ok(Json(LoginResponse {
        user_id: user.id,
        username: user.username,
        role: user.role,
        token: session.token_hash,
    }))
}

/// POST /auth/logout
pub async fn logout(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "No token" }))))?;
    state.users.revoke_session(token).await;
    Ok(Json(serde_json::json!({ "message": "Logged out" })))
}

/// GET /auth/me
pub async fn get_current_user(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
) -> Result<Json<UserResponse>, (StatusCode, Json<serde_json::Value>)> {
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "No token" }))))?;
    let ip = addr.ip().to_string();
    let user = state.users.verify_session(token, &ip).await
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid session" }))))?;
    
    Ok(Json(UserResponse {
        id: user.id,
        username: user.username,
        role: format!("{:?}", user.role).to_lowercase(),
        email: user.email,
        is_active: user.is_active,
    }))
}

/// POST /admin/users (admin only)
#[axum::debug_handler]
pub async fn create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<UserResponse>), (StatusCode, Json<serde_json::Value>)> {
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "No token" }))))?;
    
    // For now, skip IP validation - just verify token exists
    let user = state.users.verify_session(token, "127.0.0.1").await
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid token" }))))?;
    
    if !user.role.can_manage_users() {
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Only admins can create users" }))));
    }

    let role = match req.role.to_lowercase().as_str() {
        "admin" => Role::Admin,
        "teacher" => Role::Teacher,
        "assistant" => Role::Assistant,
        _ => return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "Invalid role" }))))
    };

    let new_user = state.users.create_user(req.username, req.email, req.password, role).await
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))))?;

    Ok((StatusCode::CREATED, Json(UserResponse {
        id: new_user.id,
        username: new_user.username,
        role: format!("{:?}", new_user.role).to_lowercase(),
        email: new_user.email,
        is_active: new_user.is_active,
    })))
}

/// GET /admin/users (admin only)
#[axum::debug_handler]
pub async fn list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ListUsersResponse>, (StatusCode, Json<serde_json::Value>)> {
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "No token" }))))?;
    
    // For now, skip IP validation - just verify token exists
    let user = state.users.verify_session(token, "127.0.0.1").await
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid token" }))))?;
    
    if !user.role.can_manage_users() {
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Only admins can list users" }))));
    }

    let users = state.users.list_users().await;
    let responses: Vec<_> = users
        .into_iter()
        .map(|u| UserResponse {
            id: u.id,
            username: u.username,
            role: format!("{:?}", u.role).to_lowercase(),
            email: u.email,
            is_active: u.is_active,
        })
        .collect();

    Ok(Json(ListUsersResponse { users: responses }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateUserRequest {
    pub username: Option<String>,
    /// None = don't change, Some(null) = clear email, Some("value") = set email
    pub email: Option<Option<String>>,
    pub role: Option<String>,
    pub is_active: Option<bool>,
}

/// PUT /admin/users/{id} (admin only)
#[axum::debug_handler]
pub async fn update_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<UserResponse>, (StatusCode, Json<serde_json::Value>)> {
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "No token" }))))?;

    let user = state.users.verify_session(token, "127.0.0.1").await
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid token" }))))?;

    if !user.role.can_manage_users() {
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Only admins can update users" }))));
    }

    let role = match req.role {
        Some(ref r) => Some(match r.to_lowercase().as_str() {
            "admin" => Role::Admin,
            "teacher" => Role::Teacher,
            "assistant" => Role::Assistant,
            _ => return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "Invalid role" })))),
        }),
        None => None,
    };

    let updated = state.users.update_user(
        &user_id,
        req.username,
        req.email,
        role,
        req.is_active,
    ).await
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))))?;

    Ok(Json(UserResponse {
        id: updated.id,
        username: updated.username,
        role: format!("{:?}", updated.role).to_lowercase(),
        email: updated.email,
        is_active: updated.is_active,
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: Option<String>,
    pub new_password: String,
}

/// PUT /admin/users/{id}/password (admin or self)
#[axum::debug_handler]
pub async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "No token" }))))?;

    let requester = state.users.verify_session(token, "127.0.0.1").await
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid token" }))))?;

    let is_self = requester.id == user_id;
    let is_admin = requester.role.can_manage_users();

    if !is_self && !is_admin {
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Cannot change another user's password" }))));
    }

    // Self-service requires current password; admin can reset without it
    let current: Option<&str> = if is_self {
        Some(req.current_password.as_deref()
            .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "Current password is required" }))))?)
    } else {
        None
    };

    state.users.change_password(&user_id, current, &req.new_password).await
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))))?;

    Ok(Json(serde_json::json!({ "message": "Password updated successfully" })))
}

/// DELETE /admin/users/{id} (admin only)
#[axum::debug_handler]
pub async fn delete_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "No token" }))))?;

    let user = state.users.verify_session(token, "127.0.0.1").await
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid token" }))))?;

    if !user.role.can_manage_users() {
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Only admins can delete users" }))));
    }

    state.users.delete_user(&user_id, &user.id).await
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))))?;

    Ok(Json(serde_json::json!({ "message": "User deleted" })))
}
