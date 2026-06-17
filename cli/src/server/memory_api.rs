use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use super::{AppState, authenticated_tenant_context};

#[derive(Debug, Deserialize)]
pub struct MemorySearchRequest {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub threshold: f32,
    #[serde(default)]
    pub filters: serde_json::Map<String, serde_json::Value>,
    #[serde(rename = "contextSummary")]
    pub context_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MemoryAddRequest {
    pub content: String,
    pub layer: String,
    #[serde(default)]
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct MemoryDeleteRequest {
    pub layer: String,
}

#[derive(Debug, Deserialize)]
pub struct MemoryListRequest {
    pub layer: String,
    #[serde(default = "default_list_limit")]
    pub limit: usize,
}

#[derive(Debug, Deserialize)]
pub struct MemoryFeedbackRequest {
    #[serde(rename = "memoryId")]
    pub memory_id: String,
    pub layer: String,
    #[serde(rename = "rewardType")]
    pub reward_type: String,
    pub score: f32,
    pub reasoning: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DiscoveryResponse {
    pub service: &'static str,
    pub version: &'static str,
    pub endpoints: Vec<EndpointInfo>,
}

#[derive(Debug, Serialize)]
pub struct EndpointInfo {
    pub path: &'static str,
    pub method: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Serialize)]
pub struct MemorySearchResponse {
    pub items: Vec<mk_core::types::MemoryEntry>,
    pub total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct MemoryAddResponse {
    #[serde(rename = "memoryId")]
    pub memory_id: String,
    pub layer: String,
}

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub success: bool,
    pub message: String,
}

fn default_limit() -> usize {
    10
}

fn default_list_limit() -> usize {
    20
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/memory", get(discovery_handler))
        .route("/memory/search", post(search_handler))
        .route("/memory/add", post(add_handler))
        .route("/memory/list", post(list_handler))
        .route("/memory/feedback", post(feedback_handler))
        .route("/memory/{id}", delete(delete_handler))
        .with_state(state)
}

async fn discovery_handler() -> impl IntoResponse {
    Json(DiscoveryResponse {
        service: "aeterna-memory",
        version: env!("CARGO_PKG_VERSION"),
        endpoints: vec![
            EndpointInfo {
                path: "/api/v1/memory",
                method: "GET",
                description: "Memory service discovery",
            },
            EndpointInfo {
                path: "/api/v1/memory/search",
                method: "POST",
                description: "Search memories with optional reasoning",
            },
            EndpointInfo {
                path: "/api/v1/memory/add",
                method: "POST",
                description: "Store a new memory",
            },
            EndpointInfo {
                path: "/api/v1/memory/list",
                method: "POST",
                description: "List memories from a layer",
            },
            EndpointInfo {
                path: "/api/v1/memory/feedback",
                method: "POST",
                description: "Record memory feedback",
            },
            EndpointInfo {
                path: "/api/v1/memory/{id}",
                method: "DELETE",
                description: "Delete a memory from a layer",
            },
        ],
    })
}

async fn search_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<MemorySearchRequest>,
) -> impl IntoResponse {
    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    match state
        .memory_manager
        .search_text_with_reasoning(
            ctx,
            &req.query,
            req.limit,
            req.threshold,
            req.filters.into_iter().collect::<HashMap<_, _>>(),
            req.context_summary.as_deref(),
        )
        .await
    {
        Ok((items, reasoning_trace)) => {
            // Touch last_accessed_at for returned entries so importance decay
            // knows when a memory was last useful.
            if !items.is_empty() {
                let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
                let now_epoch = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                let pool = state.postgres.pool().clone();
                tokio::spawn(async move {
                    let res = sqlx::query(
                        "UPDATE memory_entries SET last_accessed_at = $1 WHERE id = ANY($2)",
                    )
                    .bind(now_epoch)
                    .bind(&ids)
                    .execute(&pool)
                    .await;
                    if let Err(e) = res {
                        tracing::debug!(error = %e, "Failed to update last_accessed_at");
                    }
                });
            }

            let reasoning = reasoning_trace.map(|trace| {
                serde_json::json!({
                    "strategy": trace.strategy.to_string(),
                    "refinedQuery": trace.refined_query,
                    "thoughtProcess": trace.thought_process,
                    "durationMs": trace.duration_ms,
                    "timedOut": trace.timed_out,
                })
            });

            (
                StatusCode::OK,
                Json(MemorySearchResponse {
                    total: items.len(),
                    items,
                    reasoning,
                }),
            )
                .into_response()
        }
        Err(err) => {
            tracing::error!(error = %err, "memory search failed");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_search_failed",
                &err.to_string(),
            )
        }
    }
}

async fn add_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<MemoryAddRequest>,
) -> impl IntoResponse {
    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let layer = match parse_memory_layer(&req.layer) {
        Ok(layer) => layer,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, "invalid_layer", message),
    };

    match state.memory_manager.add(ctx, &req.content, layer).await {
        Ok(memory_id) => (
            StatusCode::OK,
            Json(MemoryAddResponse {
                memory_id,
                layer: req.layer,
            }),
        )
            .into_response(),
        Err(err) => {
            tracing::error!(error = %err, layer = %req.layer, "memory add failed");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_add_failed",
                &err.to_string(),
            )
        }
    }
}

async fn list_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<MemoryListRequest>,
) -> impl IntoResponse {
    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let layer = match parse_memory_layer(&req.layer) {
        Ok(layer) => layer,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, "invalid_layer", message),
    };

    match state.memory_manager.list_all_from_layer(ctx, layer).await {
        Ok(mut items) => {
            items.truncate(req.limit);
            (
                StatusCode::OK,
                Json(MemorySearchResponse {
                    total: items.len(),
                    items,
                    reasoning: None,
                }),
            )
                .into_response()
        }
        Err(err) => {
            tracing::error!(error = %err, layer = %req.layer, "memory list failed");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_list_failed",
                &err.to_string(),
            )
        }
    }
}

async fn feedback_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<MemoryFeedbackRequest>,
) -> impl IntoResponse {
    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let layer = match parse_memory_layer(&req.layer) {
        Ok(layer) => layer,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, "invalid_layer", message),
    };

    let reward_type = match parse_reward_type(&req.reward_type) {
        Ok(reward_type) => reward_type,
        Err(message) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_feedback_type", message);
        }
    };

    let reward = mk_core::types::RewardSignal {
        reward_type,
        score: req.score,
        reasoning: req.reasoning,
        agent_id: ctx.agent_id.clone(),
        timestamp: chrono::Utc::now().timestamp(),
    };

    match state
        .memory_manager
        .record_reward(ctx, layer, &req.memory_id, reward)
        .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(MessageResponse {
                success: true,
                message: "Reward recorded successfully".to_string(),
            }),
        )
            .into_response(),
        Err(err) => {
            tracing::error!(
                error = %err,
                memory_id = %req.memory_id,
                layer = %req.layer,
                "memory feedback failed"
            );
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_feedback_failed",
                &err.to_string(),
            )
        }
    }
}

async fn delete_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(memory_id): Path<String>,
    Json(req): Json<MemoryDeleteRequest>,
) -> impl IntoResponse {
    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let layer = match parse_memory_layer(&req.layer) {
        Ok(layer) => layer,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, "invalid_layer", message),
    };

    match state
        .memory_manager
        .delete_from_layer(ctx, layer, &memory_id)
        .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(MessageResponse {
                success: true,
                message: "Memory deleted successfully".to_string(),
            }),
        )
            .into_response(),
        Err(err) => {
            tracing::error!(
                error = %err,
                memory_id = %memory_id,
                layer = %req.layer,
                "memory delete failed"
            );
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_delete_failed",
                &err.to_string(),
            )
        }
    }
}

fn parse_memory_layer(layer: &str) -> Result<mk_core::types::MemoryLayer, &'static str> {
    match layer.to_lowercase().as_str() {
        "agent" => Ok(mk_core::types::MemoryLayer::Agent),
        "user" => Ok(mk_core::types::MemoryLayer::User),
        "session" => Ok(mk_core::types::MemoryLayer::Session),
        "project" => Ok(mk_core::types::MemoryLayer::Project),
        "team" => Ok(mk_core::types::MemoryLayer::Team),
        "org" => Ok(mk_core::types::MemoryLayer::Org),
        "tenant" => Ok(mk_core::types::MemoryLayer::Tenant),
        _ => Err("Unknown memory layer"),
    }
}

fn parse_reward_type(reward_type: &str) -> Result<mk_core::types::RewardType, &'static str> {
    match reward_type.to_lowercase().as_str() {
        "helpful" => Ok(mk_core::types::RewardType::Helpful),
        "irrelevant" => Ok(mk_core::types::RewardType::Irrelevant),
        "outdated" => Ok(mk_core::types::RewardType::Outdated),
        "inaccurate" => Ok(mk_core::types::RewardType::Inaccurate),
        "duplicate" => Ok(mk_core::types::RewardType::Duplicate),
        _ => Err("Unknown feedback type"),
    }
}

pub(crate) async fn tenant_context_from_request(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<mk_core::types::TenantContext, axum::response::Response> {
    authenticated_tenant_context(state, headers).await
}

fn error_response(status: StatusCode, error: &str, message: &str) -> axum::response::Response {
    (
        status,
        Json(serde_json::json!({
            "error": error,
            "message": message,
        })),
    )
        .into_response()
}
