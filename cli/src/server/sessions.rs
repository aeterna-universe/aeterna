use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use mk_core::types::SYSTEM_USER_ID;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::AppState;
use super::plugin_auth::validate_plugin_bearer;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionStartRequest {
    project: Option<String>,
    directory: Option<String>,
    team: Option<String>,
    org: Option<String>,
    user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionEndRequest {
    summary: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionContextResponse {
    session_id: String,
    user_id: String,
    project: Option<String>,
    team: Option<String>,
    org: Option<String>,
    tenant: Option<String>,
    started_at: String,
}

#[derive(Debug, Serialize)]
struct SessionEndResponse {
    session_id: String,
    ended: bool,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
    message: String,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/sessions", post(start_session_handler))
        .route("/sessions/{id}/end", post(end_session_handler))
        .with_state(state)
}

async fn start_session_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<SessionStartRequest>,
) -> impl IntoResponse {
    let user_id = match authenticated_user_id(&state, &headers, req.user_id.as_deref()) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };

    let _ = req.directory;

    (
        StatusCode::OK,
        Json(SessionContextResponse {
            session_id: Uuid::new_v4().to_string(),
            user_id,
            project: req.project,
            team: req.team,
            org: req.org,
            tenant: None,
            started_at: chrono::Utc::now().to_rfc3339(),
        }),
    )
        .into_response()
}

async fn end_session_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(req): Json<SessionEndRequest>,
) -> impl IntoResponse {
    if let Err(response) = authenticated_user_id(&state, &headers, None) {
        return response;
    }

    let _ = req.summary;

    (
        StatusCode::OK,
        Json(SessionEndResponse {
            session_id,
            ended: true,
        }),
    )
        .into_response()
}

fn authenticated_user_id(
    state: &AppState,
    headers: &HeaderMap,
    fallback_user_id: Option<&str>,
) -> Result<String, axum::response::Response> {
    if state.plugin_auth_state.config.enabled {
        let secret = state
            .plugin_auth_state
            .config
            .jwt_secret
            .as_deref()
            .ok_or_else(|| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "configuration_error",
                    "Plugin auth JWT secret is not configured",
                )
            })?;

        let identity = validate_plugin_bearer(headers, secret).ok_or_else(|| {
            error_response(
                StatusCode::UNAUTHORIZED,
                "invalid_plugin_token",
                "Valid plugin bearer token required",
            )
        })?;

        return Ok(identity.github_login);
    }

    let user_id = fallback_user_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(SYSTEM_USER_ID);
    Ok(user_id.chars().take(100).collect())
}

fn error_response(status: StatusCode, error: &str, message: &str) -> axum::response::Response {
    (
        status,
        Json(ErrorBody {
            error: error.to_string(),
            message: message.to_string(),
        }),
    )
        .into_response()
}
