use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use futures_util::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;
use std::str::FromStr;
use std::sync::Arc;

use super::plugin_auth::validate_plugin_bearer;
use super::{AppState, authenticated_tenant_context};

#[derive(Debug, Deserialize)]
pub struct KnowledgeQueryRequest {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    pub layer: Option<String>,
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub resolve: bool,
}

fn default_limit() -> usize {
    10
}

#[derive(Debug, Deserialize)]
pub struct CreateKnowledgeRequest {
    pub content: String,
    pub path: String,
    pub layer: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct UpdateKnowledgeRequest {
    pub content: Option<String>,
    pub path: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct PromotionPreviewRequest {
    pub target_layer: String,
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePromotionRequest {
    pub target_layer: String,
    pub mode: Option<String>,
    pub shared_content: String,
    pub residual_content: Option<String>,
    pub residual_role: Option<String>,
    pub justification: Option<String>,
    pub source_version: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PromotionsQuery {
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ApprovePromotionRequest {
    pub decision: String,
    /// Task 8.1: Optimistic concurrency token — set to the `updatedAt` value from
    /// the most recently read promotion; if the request has been updated since, the
    /// server returns 409 Conflict.
    #[serde(default)]
    pub client_version: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RejectPromotionRequest {
    pub reason: String,
    /// Task 8.1: Optimistic concurrency token (see ApprovePromotionRequest).
    #[serde(default)]
    pub client_version: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RetargetPromotionRequest {
    pub target_layer: String,
    /// Task 8.1: Optimistic concurrency token (see ApprovePromotionRequest).
    #[serde(default)]
    pub client_version: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRelationRequest {
    pub target_id: String,
    pub relation_type: String,
}

#[derive(Debug, Deserialize)]
pub struct BatchRequest {
    pub operations: Vec<BatchOperation>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum BatchOperation {
    Create(CreateKnowledgeRequest),
    Delete { id: String },
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
pub struct KnowledgeItem {
    pub id: String,
    pub content: String,
    pub path: String,
    pub layer: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    /// The variant role of this entry (Canonical, Specialization, etc.).
    /// Absent means Canonical (default for legacy items).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant_role: Option<String>,
    /// Semantic relations for this entry.  Empty for entries with no relations.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub relations: Vec<mk_core::types::KnowledgeRelation>,
}

/// An enriched query result grouping a primary entry (typically Canonical) with
/// its locally-related residual entries (Specialization / Applicability /
/// Exception / Clarification).  Returned by the enriched query endpoint.
#[derive(Debug, Serialize)]
pub struct KnowledgeQueryResultItem {
    pub primary: KnowledgeItem,
    pub local_residuals: Vec<KnowledgeResidualItem>,
}

/// A residual entry related to a canonical primary result.
#[derive(Debug, Serialize)]
pub struct KnowledgeResidualItem {
    pub relation_type: String,
    pub item: KnowledgeItem,
}

#[derive(Debug, Serialize)]
pub struct QueryResponse {
    pub items: Vec<KnowledgeItem>,
    pub total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved: Option<Vec<KnowledgeQueryResultItem>>,
}

#[derive(Debug, Serialize)]
pub struct BatchResponse {
    pub results: Vec<BatchOperationResult>,
}

#[derive(Debug, Serialize)]
pub struct BatchOperationResult {
    pub index: usize,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MetadataResponse {
    pub id: String,
    pub layer: String,
    pub path: String,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
    message: String,
}

#[derive(Debug, Deserialize)]
pub struct StreamQuery {
    #[serde(default)]
    pub layer: Option<String>,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/knowledge", get(discovery_handler))
        .route("/knowledge/promotions", get(list_promotions_handler))
        .route("/knowledge/promotions/{id}", get(get_promotion_handler))
        .route(
            "/knowledge/promotions/{id}/approve",
            post(approve_promotion_handler),
        )
        .route(
            "/knowledge/promotions/{id}/reject",
            post(reject_promotion_handler),
        )
        .route(
            "/knowledge/promotions/{id}/retarget",
            post(retarget_promotion_handler),
        )
        .route("/knowledge/query", post(query_handler))
        .route("/knowledge/create", post(create_handler))
        .route(
            "/knowledge/{id}/promotions/preview",
            post(promotion_preview_handler),
        )
        .route("/knowledge/{id}/promotions", post(create_promotion_handler))
        .route("/knowledge/{id}/relations", post(create_relation_handler))
        .route("/knowledge/{id}", put(update_handler))
        .route("/knowledge/{id}", delete(delete_handler))
        .route("/knowledge/batch", post(batch_handler))
        .route("/knowledge/stream", get(stream_handler))
        .route("/knowledge/{id}/metadata", get(metadata_handler))
        .with_state(state)
}

#[tracing::instrument(skip_all)]
async fn discovery_handler() -> impl IntoResponse {
    Json(DiscoveryResponse {
        service: "aeterna-knowledge",
        version: env!("CARGO_PKG_VERSION"),
        endpoints: vec![
            EndpointInfo {
                path: "/api/v1/knowledge",
                method: "GET",
                description: "Service discovery",
            },
            EndpointInfo {
                path: "/api/v1/knowledge/query",
                method: "POST",
                description: "Search knowledge with relevance ranking",
            },
            EndpointInfo {
                path: "/api/v1/knowledge/promotions",
                method: "GET",
                description: "List promotion requests",
            },
            EndpointInfo {
                path: "/api/v1/knowledge/promotions/{id}",
                method: "GET",
                description: "Get a promotion request",
            },
            EndpointInfo {
                path: "/api/v1/knowledge/promotions/{id}/approve",
                method: "POST",
                description: "Approve a promotion request",
            },
            EndpointInfo {
                path: "/api/v1/knowledge/promotions/{id}/reject",
                method: "POST",
                description: "Reject a promotion request",
            },
            EndpointInfo {
                path: "/api/v1/knowledge/promotions/{id}/retarget",
                method: "POST",
                description: "Retarget a promotion request",
            },
            EndpointInfo {
                path: "/api/v1/knowledge/create",
                method: "POST",
                description: "Create a new knowledge item",
            },
            EndpointInfo {
                path: "/api/v1/knowledge/{id}/promotions/preview",
                method: "POST",
                description: "Preview a knowledge promotion",
            },
            EndpointInfo {
                path: "/api/v1/knowledge/{id}/promotions",
                method: "POST",
                description: "Create a promotion request for a knowledge item",
            },
            EndpointInfo {
                path: "/api/v1/knowledge/{id}/relations",
                method: "POST",
                description: "Create an explicit relation from a knowledge item",
            },
            EndpointInfo {
                path: "/api/v1/knowledge/{id}",
                method: "PUT",
                description: "Update an existing knowledge item",
            },
            EndpointInfo {
                path: "/api/v1/knowledge/{id}",
                method: "DELETE",
                description: "Delete a knowledge item",
            },
            EndpointInfo {
                path: "/api/v1/knowledge/batch",
                method: "POST",
                description: "Batch operations on knowledge items",
            },
            EndpointInfo {
                path: "/api/v1/knowledge/stream",
                method: "GET",
                description: "SSE stream of knowledge updates",
            },
            EndpointInfo {
                path: "/api/v1/knowledge/{id}/metadata",
                method: "GET",
                description: "Get metadata for a knowledge item",
            },
        ],
    })
}

#[tracing::instrument(skip_all, fields(query = %req.query, limit = req.limit))]
async fn query_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<KnowledgeQueryRequest>,
) -> impl IntoResponse {
    if let Some(response) = reject_invalid_plugin_bearer(&state, &headers) {
        return response;
    }

    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    // Determine which layers to search.  When a single layer is provided we
    // search only that layer; otherwise we search all layers and apply
    // canonical-vs-residual precedence (task 6.1).
    let layers: Vec<mk_core::types::KnowledgeLayer> = match req.layer.as_deref() {
        Some(l) => parse_layer(Some(l)).map_or_else(
            || vec![mk_core::types::KnowledgeLayer::Project],
            |kl| vec![kl],
        ),
        None => vec![
            mk_core::types::KnowledgeLayer::Tenant,
            mk_core::types::KnowledgeLayer::Org,
            mk_core::types::KnowledgeLayer::Team,
            mk_core::types::KnowledgeLayer::Project,
        ],
    };

    match state
        .knowledge_manager
        .query_enriched(ctx, &req.query, layers, req.limit)
        .await
    {
        Ok(query_results) => {
            let resolved: Vec<KnowledgeQueryResultItem> = query_results
                .into_iter()
                .map(|qr| KnowledgeQueryResultItem {
                    primary: knowledge_entry_with_relations_to_item(qr.primary),
                    local_residuals: qr
                        .local_residuals
                        .into_iter()
                        .map(|(rel_type, ewr)| KnowledgeResidualItem {
                            relation_type: rel_type.to_string(),
                            item: knowledge_entry_with_relations_to_item(ewr),
                        })
                        .collect(),
                })
                .collect();
            let mut items = Vec::new();
            for group in &resolved {
                items.push(KnowledgeItem {
                    id: group.primary.id.clone(),
                    content: group.primary.content.clone(),
                    path: group.primary.path.clone(),
                    layer: group.primary.layer.clone(),
                    tags: group.primary.tags.clone(),
                    metadata: group.primary.metadata.clone(),
                    variant_role: group.primary.variant_role.clone(),
                    relations: group.primary.relations.clone(),
                });
                for residual in &group.local_residuals {
                    items.push(KnowledgeItem {
                        id: residual.item.id.clone(),
                        content: residual.item.content.clone(),
                        path: residual.item.path.clone(),
                        layer: residual.item.layer.clone(),
                        tags: residual.item.tags.clone(),
                        metadata: residual.item.metadata.clone(),
                        variant_role: residual.item.variant_role.clone(),
                        relations: residual.item.relations.clone(),
                    });
                }
            }
            let total = items.len();
            (
                StatusCode::OK,
                Json(QueryResponse {
                    items,
                    total,
                    resolved: req.resolve.then_some(resolved),
                }),
            )
                .into_response()
        }
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "query_failed",
            &e.to_string(),
        ),
    }
}

#[tracing::instrument(skip_all, fields(path = %req.path, layer = %req.layer))]
async fn create_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateKnowledgeRequest>,
) -> impl IntoResponse {
    if let Some(response) = reject_invalid_plugin_bearer(&state, &headers) {
        return response;
    }

    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let layer = match parse_layer(Some(&req.layer)) {
        Some(l) => l,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_layer",
                &format!("Unknown layer: {}", req.layer),
            );
        }
    };

    let entry = mk_core::types::KnowledgeEntry {
        content: req.content,
        path: req.path,
        layer,
        kind: mk_core::types::KnowledgeType::Spec,
        status: mk_core::types::KnowledgeStatus::Draft,
        metadata: if req.metadata.is_null() {
            metadata_with_tags(req.tags, serde_json::Map::new())
        } else {
            match req.metadata {
                serde_json::Value::Object(map) => metadata_with_tags(req.tags, map),
                _ => metadata_with_tags(req.tags, serde_json::Map::new()),
            }
        },
        summaries: Default::default(),
        commit_hash: None,
        author: None,
        updated_at: 0,
    };

    match state
        .knowledge_repository
        .store(ctx, entry.clone(), "Created via Knowledge API")
        .await
    {
        Ok(commit_hash) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "id": entry.path,
                "commit": commit_hash,
            })),
        )
            .into_response(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "create_failed",
            &e.to_string(),
        ),
    }
}

#[tracing::instrument(skip_all, fields(id = %id))]
async fn update_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<UpdateKnowledgeRequest>,
) -> impl IntoResponse {
    if let Some(response) = reject_invalid_plugin_bearer(&state, &headers) {
        return response;
    }

    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let existing = find_entry_by_id(&state, &ctx, &id).await;
    let mut entry = match existing {
        Some(e) => e,
        None => {
            return error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "Knowledge item not found",
            );
        }
    };

    if let Some(content) = req.content {
        entry.content = content;
    }
    if let Some(path) = req.path {
        entry.path = path;
    }
    if let Some(tags) = req.tags {
        set_tags(&mut entry.metadata, tags);
    }
    if let Some(metadata) = req.metadata {
        if metadata.get("status").is_some() || metadata.get("variant_role").is_some() {
            return error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "unsafe_mutation_blocked",
                "Direct status and variant_role mutations are not allowed; use promotion lifecycle endpoints",
            );
        }
        if let Ok(m) = serde_json::from_value(metadata) {
            entry.metadata = m;
        }
    }

    match state
        .knowledge_repository
        .store(ctx, entry, "Updated via Knowledge API")
        .await
    {
        Ok(commit_hash) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": id,
                "commit": commit_hash,
            })),
        )
            .into_response(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "update_failed",
            &e.to_string(),
        ),
    }
}

#[tracing::instrument(skip_all, fields(id = %id))]
async fn delete_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Some(response) = reject_invalid_plugin_bearer(&state, &headers) {
        return response;
    }

    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let existing = find_entry_by_id(&state, &ctx, &id).await;
    match existing {
        Some(entry) => {
            match state
                .knowledge_repository
                .delete(ctx, entry.layer, &entry.path, "Deleted via Knowledge API")
                .await
            {
                Ok(_) => StatusCode::NO_CONTENT.into_response(),
                Err(e) => error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "delete_failed",
                    &e.to_string(),
                ),
            }
        }
        None => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Knowledge item not found",
        ),
    }
}

#[tracing::instrument(skip_all, fields(operation_count = req.operations.len()))]
async fn batch_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<BatchRequest>,
) -> impl IntoResponse {
    if let Some(response) = reject_invalid_plugin_bearer(&state, &headers) {
        return response;
    }

    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let mut results = Vec::with_capacity(req.operations.len());

    for (index, op) in req.operations.into_iter().enumerate() {
        let result = match op {
            BatchOperation::Create(create_req) => {
                let layer = parse_layer(Some(&create_req.layer))
                    .unwrap_or(mk_core::types::KnowledgeLayer::Project);
                let entry = mk_core::types::KnowledgeEntry {
                    content: create_req.content,
                    path: create_req.path,
                    layer,
                    kind: mk_core::types::KnowledgeType::Spec,
                    status: mk_core::types::KnowledgeStatus::Draft,
                    metadata: metadata_with_tags(create_req.tags, serde_json::Map::new()),
                    summaries: Default::default(),
                    commit_hash: None,
                    author: None,
                    updated_at: 0,
                };
                let id = entry.path.clone();
                match state
                    .knowledge_repository
                    .store(ctx.clone(), entry, "Batch create via Knowledge API")
                    .await
                {
                    Ok(_) => BatchOperationResult {
                        index,
                        success: true,
                        id: Some(id),
                        error: None,
                    },
                    Err(e) => BatchOperationResult {
                        index,
                        success: false,
                        id: None,
                        error: Some(e.to_string()),
                    },
                }
            }
            BatchOperation::Delete { id } => match find_entry_by_id(&state, &ctx, &id).await {
                Some(entry) => {
                    match state
                        .knowledge_repository
                        .delete(
                            ctx.clone(),
                            entry.layer,
                            &entry.path,
                            "Batch delete via Knowledge API",
                        )
                        .await
                    {
                        Ok(_) => BatchOperationResult {
                            index,
                            success: true,
                            id: Some(id),
                            error: None,
                        },
                        Err(e) => BatchOperationResult {
                            index,
                            success: false,
                            id: None,
                            error: Some(e.to_string()),
                        },
                    }
                }
                None => BatchOperationResult {
                    index,
                    success: false,
                    id: None,
                    error: Some(format!("Item not found: {id}")),
                },
            },
        };
        results.push(result);
    }

    Json(BatchResponse { results }).into_response()
}

#[tracing::instrument(skip_all)]
async fn stream_handler(
    State(_state): State<Arc<AppState>>,
    Query(_query): Query<StreamQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = stream::once(async {
        Ok::<_, Infallible>(
            Event::default()
                .event("connected")
                .data(serde_json::json!({"status": "connected"}).to_string()),
        )
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[tracing::instrument(skip_all, fields(id = %id))]
async fn metadata_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Some(response) = reject_invalid_plugin_bearer(&state, &headers) {
        return response;
    }

    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    match find_entry_by_id(&state, &ctx, &id).await {
        Some(entry) => {
            let layer_str = match entry.layer {
                mk_core::types::KnowledgeLayer::Tenant => "tenant",
                mk_core::types::KnowledgeLayer::Org => "org",
                mk_core::types::KnowledgeLayer::Team => "team",
                mk_core::types::KnowledgeLayer::Project => "project",
            };
            (
                StatusCode::OK,
                Json(MetadataResponse {
                    id: entry.path.clone(),
                    layer: layer_str.to_string(),
                    path: entry.path,
                    created_at: entry
                        .metadata
                        .get("created_at")
                        .and_then(serde_json::Value::as_i64),
                    updated_at: entry
                        .metadata
                        .get("updated_at")
                        .and_then(serde_json::Value::as_i64),
                    tags: if tags_from_metadata(&entry.metadata).is_empty() {
                        None
                    } else {
                        Some(tags_from_metadata(&entry.metadata))
                    },
                }),
            )
                .into_response()
        }
        None => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Knowledge item not found",
        ),
    }
}

#[tracing::instrument(skip_all, fields(id = %id, target_layer = %req.target_layer))]
async fn promotion_preview_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<PromotionPreviewRequest>,
) -> impl IntoResponse {
    if let Some(response) = reject_invalid_plugin_bearer(&state, &headers) {
        return response;
    }

    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let target_layer = match parse_layer(Some(&req.target_layer)) {
        Some(layer) => layer,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_layer",
                &format!("Unknown layer: {}", req.target_layer),
            );
        }
    };

    let mode = match parse_promotion_mode(req.mode.as_deref()) {
        Ok(mode) => mode,
        Err(message) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_promotion_mode", &message);
        }
    };

    match state
        .knowledge_manager
        .preview_promotion(ctx, &id, target_layer, mode)
        .await
    {
        Ok(preview) => (StatusCode::OK, Json(preview)).into_response(),
        Err(knowledge::manager::KnowledgeManagerError::SourceNotFound(_)) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Knowledge item not found",
        ),
        Err(knowledge::manager::KnowledgeManagerError::Validation(message)) => {
            error_response(StatusCode::BAD_REQUEST, "invalid_promotion", &message)
        }
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "promotion_preview_failed",
            &err.to_string(),
        ),
    }
}

#[tracing::instrument(skip_all, fields(id = %id, target_layer = %req.target_layer))]
async fn create_promotion_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<CreatePromotionRequest>,
) -> impl IntoResponse {
    if let Some(response) = reject_invalid_plugin_bearer(&state, &headers) {
        return response;
    }

    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let existing = match find_entry_by_id(&state, &ctx, &id).await {
        Some(entry) => entry,
        None => {
            return error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "Knowledge item not found",
            );
        }
    };

    let target_layer = match parse_layer(Some(&req.target_layer)) {
        Some(layer) => layer,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_layer",
                &format!("Unknown layer: {}", req.target_layer),
            );
        }
    };
    let mode = match parse_promotion_mode(req.mode.as_deref()) {
        Ok(mode) => mode,
        Err(message) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_promotion_mode", &message);
        }
    };
    let residual_role = match parse_residual_role(req.residual_role.as_deref()) {
        Ok(role) => role,
        Err(message) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_residual_role", &message);
        }
    };

    let now = chrono::Utc::now().timestamp();
    let promotion = mk_core::types::PromotionRequest {
        id: String::new(),
        source_item_id: id,
        source_layer: existing.layer,
        source_status: existing.status,
        target_layer,
        promotion_mode: mode,
        shared_content: req.shared_content,
        residual_content: req.residual_content,
        residual_role,
        justification: req.justification,
        status: mk_core::types::PromotionRequestStatus::Draft,
        requested_by: ctx.user_id.clone(),
        tenant_id: ctx.tenant_id.clone(),
        source_version: req
            .source_version
            .unwrap_or_else(|| existing.commit_hash.clone().unwrap_or_default()),
        latest_decision: None,
        promoted_item_id: None,
        residual_item_id: None,
        created_at: now,
        updated_at: now,
    };

    match state
        .knowledge_manager
        .create_promotion_request(ctx, promotion)
        .await
    {
        Ok(request) => (StatusCode::CREATED, Json(request)).into_response(),
        Err(knowledge::manager::KnowledgeManagerError::ConfidentialContent) => error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "sensitive_content_detected",
            "Promotion content appears to contain sensitive material",
        ),
        Err(
            knowledge::manager::KnowledgeManagerError::TenantMismatch(_)
            | knowledge::manager::KnowledgeManagerError::ForbiddenCrossTenant,
        ) => error_response(
            StatusCode::FORBIDDEN,
            "cross_tenant_forbidden",
            "Promotion belongs to a different tenant; cross-tenant promotion is not permitted",
        ),
        Err(knowledge::manager::KnowledgeManagerError::Governance(message)) => error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "policy_violation",
            &format!("Promotion blocked by governance policy: {message}"),
        ),
        Err(
            knowledge::manager::KnowledgeManagerError::Validation(message)
            | knowledge::manager::KnowledgeManagerError::StalePromotion(message),
        ) => error_response(StatusCode::BAD_REQUEST, "invalid_promotion", &message),
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "promotion_create_failed",
            &err.to_string(),
        ),
    }
}

#[tracing::instrument(skip_all)]
async fn list_promotions_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(req): Query<PromotionsQuery>,
) -> impl IntoResponse {
    if let Some(response) = reject_invalid_plugin_bearer(&state, &headers) {
        return response;
    }

    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let status = match parse_promotion_status(req.status.as_deref()) {
        Ok(status) => status,
        Err(message) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_promotion_status",
                &message,
            );
        }
    };

    match state
        .knowledge_manager
        .list_promotion_requests(ctx, status)
        .await
    {
        Ok(requests) => (StatusCode::OK, Json(requests)).into_response(),
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "promotion_list_failed",
            &err.to_string(),
        ),
    }
}

#[tracing::instrument(skip_all, fields(id = %id))]
async fn get_promotion_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Some(response) = reject_invalid_plugin_bearer(&state, &headers) {
        return response;
    }

    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    match state
        .knowledge_manager
        .get_promotion_request(ctx, &id)
        .await
    {
        Ok(Some(request)) => (StatusCode::OK, Json(request)).into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Promotion request not found",
        ),
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "promotion_get_failed",
            &err.to_string(),
        ),
    }
}

#[tracing::instrument(skip_all, fields(id = %id))]
async fn approve_promotion_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<ApprovePromotionRequest>,
) -> impl IntoResponse {
    if let Some(response) = reject_invalid_plugin_bearer(&state, &headers) {
        return response;
    }

    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    if let Some(r) = check_reviewer_permission(&state, &ctx, &id).await {
        return r;
    }

    let decision = match mk_core::types::PromotionDecision::from_str(&req.decision) {
        Ok(decision) => decision,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_decision",
                &format!("Unknown decision: {}", req.decision),
            );
        }
    };

    match state
        .knowledge_manager
        .approve_promotion(ctx, &id, decision, req.client_version)
        .await
    {
        Ok(request) => (StatusCode::OK, Json(request)).into_response(),
        Err(knowledge::manager::KnowledgeManagerError::PromotionNotFound(_)) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Promotion request not found",
        ),
        Err(knowledge::manager::KnowledgeManagerError::ConfidentialContent) => error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "sensitive_content_detected",
            "Promotion content appears to contain sensitive material",
        ),
        Err(
            knowledge::manager::KnowledgeManagerError::TenantMismatch(_)
            | knowledge::manager::KnowledgeManagerError::ForbiddenCrossTenant,
        ) => error_response(
            StatusCode::FORBIDDEN,
            "cross_tenant_forbidden",
            "Promotion belongs to a different tenant; cross-tenant promotion is not permitted",
        ),
        Err(knowledge::manager::KnowledgeManagerError::Authorization(message)) => error_response(
            StatusCode::FORBIDDEN,
            "reviewer_authorization_failed",
            &message,
        ),
        Err(knowledge::manager::KnowledgeManagerError::Governance(message)) => error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "policy_violation",
            &format!("Promotion blocked by governance policy: {message}"),
        ),
        Err(knowledge::manager::KnowledgeManagerError::InvalidPromotionTransition(message)) => {
            error_response(StatusCode::CONFLICT, "invalid_transition", &message)
        }
        Err(knowledge::manager::KnowledgeManagerError::OptimisticConflict(server_version)) => {
            error_response(
                StatusCode::CONFLICT,
                "optimistic_conflict",
                &format!(
                    "Promotion was modified since your version; server version is {server_version}"
                ),
            )
        }
        Err(knowledge::manager::KnowledgeManagerError::StalePromotion(version)) => error_response(
            StatusCode::CONFLICT,
            "stale_promotion",
            &format!(
                "Source item has changed since promotion was submitted (version {version}); resubmit promotion"
            ),
        ),
        Err(knowledge::manager::KnowledgeManagerError::ConflictingPromotion(source, state_str)) => {
            error_response(
                StatusCode::CONFLICT,
                "conflicting_promotion",
                &format!("Another promotion for source {source} is already {state_str}"),
            )
        }
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "promotion_approve_failed",
            &err.to_string(),
        ),
    }
}

#[tracing::instrument(skip_all, fields(id = %id))]
async fn reject_promotion_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<RejectPromotionRequest>,
) -> impl IntoResponse {
    if let Some(response) = reject_invalid_plugin_bearer(&state, &headers) {
        return response;
    }

    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    if let Some(r) = check_reviewer_permission(&state, &ctx, &id).await {
        return r;
    }

    match state
        .knowledge_manager
        .reject_promotion(ctx, &id, &req.reason, req.client_version)
        .await
    {
        Ok(request) => (StatusCode::OK, Json(request)).into_response(),
        Err(knowledge::manager::KnowledgeManagerError::PromotionNotFound(_)) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Promotion request not found",
        ),
        Err(
            knowledge::manager::KnowledgeManagerError::TenantMismatch(_)
            | knowledge::manager::KnowledgeManagerError::ForbiddenCrossTenant,
        ) => error_response(
            StatusCode::FORBIDDEN,
            "cross_tenant_forbidden",
            "Promotion belongs to a different tenant; cross-tenant promotion is not permitted",
        ),
        Err(knowledge::manager::KnowledgeManagerError::Authorization(message)) => error_response(
            StatusCode::FORBIDDEN,
            "reviewer_authorization_failed",
            &message,
        ),
        Err(knowledge::manager::KnowledgeManagerError::InvalidPromotionTransition(message)) => {
            error_response(StatusCode::CONFLICT, "invalid_transition", &message)
        }
        Err(knowledge::manager::KnowledgeManagerError::OptimisticConflict(server_version)) => {
            error_response(
                StatusCode::CONFLICT,
                "optimistic_conflict",
                &format!(
                    "Promotion was modified since your version; server version is {server_version}"
                ),
            )
        }
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "promotion_reject_failed",
            &err.to_string(),
        ),
    }
}

#[tracing::instrument(skip_all, fields(id = %id, target_layer = %req.target_layer))]
async fn retarget_promotion_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<RetargetPromotionRequest>,
) -> impl IntoResponse {
    if let Some(response) = reject_invalid_plugin_bearer(&state, &headers) {
        return response;
    }

    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let new_target_layer = match parse_layer(Some(&req.target_layer)) {
        Some(layer) => layer,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_layer",
                &format!("Unknown layer: {}", req.target_layer),
            );
        }
    };

    if let Some(r) = check_reviewer_permission(&state, &ctx, &id).await {
        return r;
    }

    match state
        .knowledge_manager
        .retarget_promotion(ctx, &id, new_target_layer, req.client_version)
        .await
    {
        Ok(request) => (StatusCode::OK, Json(request)).into_response(),
        Err(knowledge::manager::KnowledgeManagerError::PromotionNotFound(_)) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "Promotion request not found",
        ),
        Err(
            knowledge::manager::KnowledgeManagerError::TenantMismatch(_)
            | knowledge::manager::KnowledgeManagerError::ForbiddenCrossTenant,
        ) => error_response(
            StatusCode::FORBIDDEN,
            "cross_tenant_forbidden",
            "Promotion belongs to a different tenant; cross-tenant promotion is not permitted",
        ),
        Err(knowledge::manager::KnowledgeManagerError::Authorization(message)) => error_response(
            StatusCode::FORBIDDEN,
            "reviewer_authorization_failed",
            &message,
        ),
        Err(knowledge::manager::KnowledgeManagerError::Validation(message)) => {
            error_response(StatusCode::BAD_REQUEST, "invalid_promotion", &message)
        }
        Err(knowledge::manager::KnowledgeManagerError::InvalidPromotionTransition(message)) => {
            error_response(StatusCode::CONFLICT, "invalid_transition", &message)
        }
        Err(knowledge::manager::KnowledgeManagerError::OptimisticConflict(server_version)) => {
            error_response(
                StatusCode::CONFLICT,
                "optimistic_conflict",
                &format!(
                    "Promotion was modified since your version; server version is {server_version}"
                ),
            )
        }
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "promotion_retarget_failed",
            &err.to_string(),
        ),
    }
}

#[tracing::instrument(skip_all, fields(id = %id, relation_type = %req.relation_type))]
async fn create_relation_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<CreateRelationRequest>,
) -> impl IntoResponse {
    if let Some(response) = reject_invalid_plugin_bearer(&state, &headers) {
        return response;
    }

    let ctx = match tenant_context_from_request(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let relation_type = match mk_core::types::KnowledgeRelationType::from_str(&req.relation_type) {
        Ok(relation_type) => relation_type,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_relation_type",
                &format!("Unknown relation type: {}", req.relation_type),
            );
        }
    };

    let relation = mk_core::types::KnowledgeRelation {
        id: uuid::Uuid::new_v4().to_string(),
        source_id: id,
        target_id: req.target_id,
        relation_type,
        tenant_id: ctx.tenant_id.clone(),
        created_by: ctx.user_id.clone(),
        created_at: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
    };

    match state.knowledge_manager.create_relation(ctx, relation).await {
        Ok(relation) => (StatusCode::CREATED, Json(relation)).into_response(),
        Err(knowledge::manager::KnowledgeManagerError::DuplicateRelation(_, _)) => error_response(
            StatusCode::CONFLICT,
            "duplicate_relation",
            "Relation already exists",
        ),
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "relation_create_failed",
            &err.to_string(),
        ),
    }
}

// ── Task 7: Security, isolation, and policy helpers ──────────────────────────

/// Task 7.4: Verify the caller has the `ApproveKnowledge` Cedar action.
///
/// Returns a `403 Forbidden` response when the caller lacks permission.
/// Used by approve, reject, and retarget handlers (all reviewer actions).
async fn check_reviewer_permission(
    state: &AppState,
    ctx: &mk_core::types::TenantContext,
    promotion_id: &str,
) -> Option<axum::response::Response> {
    let resource = format!("Aeterna::KnowledgePromotion::\"{promotion_id}\"");
    match state
        .auth_service
        .check_permission(ctx, "ApproveKnowledge", &resource)
        .await
    {
        Ok(true) => None,
        Ok(false) => Some(error_response(
            StatusCode::FORBIDDEN,
            "reviewer_authorization_failed",
            "Caller is not authorized to approve, reject, or retarget promotion requests at the target layer",
        )),
        Err(e) => {
            tracing::error!(error = %e, "Authorization service error during reviewer permission check");
            Some(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "authorization_error",
                "Authorization check failed",
            ))
        }
    }
}

/// Derive the caller's `TenantContext` from a validated Aeterna plugin bearer
/// token.
///
/// When plugin auth is **enabled**: requires a valid bearer token.  Because
/// `reject_invalid_plugin_bearer` has already blocked requests without a valid
/// token, the `None` branch here is a defensive guard only.  If it somehow
/// fires, the request is rejected rather than silently promoted to the default
/// tenant.
///
/// When plugin auth is **disabled** (development / service-to-service mode):
/// falls back to the synthetic `default/system` context with an explicit debug
/// log.  This fallback is intentional and must NOT be used in production.
async fn tenant_context_from_request(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<mk_core::types::TenantContext, axum::response::Response> {
    authenticated_tenant_context(state, headers).await
}

fn reject_invalid_plugin_bearer(
    state: &AppState,
    headers: &HeaderMap,
) -> Option<axum::response::Response> {
    if !state.plugin_auth_state.config.enabled {
        return None;
    }

    let Some(secret) = state.plugin_auth_state.config.jwt_secret.as_deref() else {
        return Some(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "configuration_error",
            "Plugin auth JWT secret is not configured",
        ));
    };

    if validate_plugin_bearer(headers, secret).is_none() {
        return Some(error_response(
            StatusCode::UNAUTHORIZED,
            "invalid_plugin_token",
            "Valid plugin bearer token required",
        ));
    }

    None
}

fn parse_layer(s: Option<&str>) -> Option<mk_core::types::KnowledgeLayer> {
    match s {
        Some("tenant") => Some(mk_core::types::KnowledgeLayer::Tenant),
        Some("org" | "organization") => Some(mk_core::types::KnowledgeLayer::Org),
        Some("team") => Some(mk_core::types::KnowledgeLayer::Team),
        Some("project") => Some(mk_core::types::KnowledgeLayer::Project),
        _ => None,
    }
}

fn parse_promotion_mode(mode: Option<&str>) -> Result<mk_core::types::PromotionMode, String> {
    match mode {
        None => Ok(mk_core::types::PromotionMode::Partial),
        Some(value) => mk_core::types::PromotionMode::from_str(value)
            .map_err(|_| format!("Unknown promotion mode: {value}")),
    }
}

fn parse_promotion_status(
    status: Option<&str>,
) -> Result<Option<mk_core::types::PromotionRequestStatus>, String> {
    match status {
        None => Ok(None),
        Some(value) => mk_core::types::PromotionRequestStatus::from_str(value)
            .map(Some)
            .map_err(|_| format!("Unknown promotion status: {value}")),
    }
}

fn parse_residual_role(
    role: Option<&str>,
) -> Result<Option<mk_core::types::KnowledgeVariantRole>, String> {
    match role {
        None => Ok(None),
        Some(value) => mk_core::types::KnowledgeVariantRole::from_str(value)
            .map(Some)
            .map_err(|_| format!("Unknown residual role: {value}")),
    }
}

fn knowledge_entry_to_item(entry: mk_core::types::KnowledgeEntry) -> KnowledgeItem {
    let layer_str = match entry.layer {
        mk_core::types::KnowledgeLayer::Tenant => "tenant",
        mk_core::types::KnowledgeLayer::Org => "org",
        mk_core::types::KnowledgeLayer::Team => "team",
        mk_core::types::KnowledgeLayer::Project => "project",
    };
    let variant_role = entry
        .metadata
        .get("variant_role")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    KnowledgeItem {
        id: entry.path.clone(),
        content: entry.content,
        path: entry.path,
        layer: layer_str.to_string(),
        tags: tags_from_metadata(&entry.metadata),
        metadata: if entry.metadata.is_empty() {
            None
        } else {
            serde_json::to_value(&entry.metadata).ok()
        },
        variant_role,
        relations: vec![],
    }
}

/// Converts a `KnowledgeEntryWithRelations` to a `KnowledgeItem`, including
/// relation context (task 6.3).
fn knowledge_entry_with_relations_to_item(
    ewr: mk_core::types::KnowledgeEntryWithRelations,
) -> KnowledgeItem {
    let layer_str = match ewr.entry.layer {
        mk_core::types::KnowledgeLayer::Tenant => "tenant",
        mk_core::types::KnowledgeLayer::Org => "org",
        mk_core::types::KnowledgeLayer::Team => "team",
        mk_core::types::KnowledgeLayer::Project => "project",
    };
    let variant_role = ewr
        .entry
        .metadata
        .get("variant_role")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    KnowledgeItem {
        id: ewr.entry.path.clone(),
        content: ewr.entry.content,
        path: ewr.entry.path,
        layer: layer_str.to_string(),
        tags: tags_from_metadata(&ewr.entry.metadata),
        metadata: if ewr.entry.metadata.is_empty() {
            None
        } else {
            serde_json::to_value(&ewr.entry.metadata).ok()
        },
        variant_role,
        relations: ewr.relations,
    }
}

async fn find_entry_by_id(
    state: &AppState,
    ctx: &mk_core::types::TenantContext,
    id: &str,
) -> Option<mk_core::types::KnowledgeEntry> {
    let layers = [
        mk_core::types::KnowledgeLayer::Project,
        mk_core::types::KnowledgeLayer::Team,
        mk_core::types::KnowledgeLayer::Org,
        mk_core::types::KnowledgeLayer::Tenant,
    ];
    for layer in &layers {
        if let Ok(entries) = state
            .knowledge_repository
            .list(ctx.clone(), *layer, "")
            .await
            && let Some(entry) = entries.into_iter().find(|entry| entry.path == id)
        {
            return Some(entry);
        }
    }
    None
}

fn metadata_with_tags(
    tags: Vec<String>,
    map: serde_json::Map<String, serde_json::Value>,
) -> std::collections::HashMap<String, serde_json::Value> {
    let mut metadata: std::collections::HashMap<String, serde_json::Value> =
        map.into_iter().collect();
    set_tags(&mut metadata, tags);
    metadata
}

fn set_tags(
    metadata: &mut std::collections::HashMap<String, serde_json::Value>,
    tags: Vec<String>,
) {
    if tags.is_empty() {
        metadata.remove("tags");
    } else {
        metadata.insert("tags".to_string(), serde_json::json!(tags));
    }
}

fn tags_from_metadata(
    metadata: &std::collections::HashMap<String, serde_json::Value>,
) -> Vec<String> {
    metadata
        .get("tags")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::PluginAuthState;
    use crate::server::plugin_auth::{RefreshTokenStore, RefreshTokenStoreBackend};
    use agent_a2a::config::TrustedIdentityConfig;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::Request;
    use knowledge::api::GovernanceDashboardApi;
    use knowledge::governance::GovernanceEngine;
    use knowledge::manager::KnowledgeManager;
    use knowledge::repository::RepositoryError;
    use knowledge::tenant_repo_resolver::TenantRepositoryResolver;
    use memory::manager::MemoryManager;
    use memory::reasoning::ReflectiveReasoner;
    use mk_core::traits::{AuthorizationService, KnowledgeRepository};
    use mk_core::types::{
        KnowledgeEntry, KnowledgeLayer, KnowledgeRelation, KnowledgeStatus, KnowledgeType,
        PromotionRequest, PromotionRequestStatus, ReasoningStrategy, ReasoningTrace, Role,
        RoleIdentifier, TenantContext, UserId,
    };
    use std::collections::HashMap;
    use storage::postgres::PostgresBackend;
    use storage::secret_provider::LocalSecretProvider;
    use storage::tenant_config_provider::KubernetesTenantConfigProvider;
    use storage::tenant_store::{TenantRepositoryBindingStore, TenantStore};
    use sync::bridge::SyncManager;
    use sync::websocket::{AuthToken, TokenValidator, WsResult, WsServer};
    use testing::postgres;
    use tools::server::McpServer;
    use tower::ServiceExt;

    struct MockAuth;

    #[async_trait]
    impl AuthorizationService for MockAuth {
        type Error = anyhow::Error;

        async fn check_permission(
            &self,
            _ctx: &TenantContext,
            _action: &str,
            _resource: &str,
        ) -> Result<bool, Self::Error> {
            Ok(true)
        }

        async fn get_user_roles(
            &self,
            _ctx: &TenantContext,
        ) -> Result<Vec<RoleIdentifier>, Self::Error> {
            Ok(vec![Role::Developer.into()])
        }

        async fn assign_role(
            &self,
            _ctx: &TenantContext,
            _user_id: &UserId,
            _role: RoleIdentifier,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn remove_role(
            &self,
            _ctx: &TenantContext,
            _user_id: &UserId,
            _role: RoleIdentifier,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    struct MockRepo {
        items: tokio::sync::RwLock<HashMap<(KnowledgeLayer, String), KnowledgeEntry>>,
        promotions: tokio::sync::RwLock<HashMap<String, PromotionRequest>>,
        relations: tokio::sync::RwLock<Vec<KnowledgeRelation>>,
    }

    impl MockRepo {
        fn new() -> Self {
            Self {
                items: tokio::sync::RwLock::new(HashMap::new()),
                promotions: tokio::sync::RwLock::new(HashMap::new()),
                relations: tokio::sync::RwLock::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl KnowledgeRepository for MockRepo {
        type Error = RepositoryError;

        async fn get(
            &self,
            _ctx: TenantContext,
            layer: KnowledgeLayer,
            path: &str,
        ) -> Result<Option<KnowledgeEntry>, Self::Error> {
            Ok(self
                .items
                .read()
                .await
                .get(&(layer, path.to_string()))
                .cloned())
        }

        async fn store(
            &self,
            _ctx: TenantContext,
            entry: KnowledgeEntry,
            _message: &str,
        ) -> Result<String, Self::Error> {
            self.items
                .write()
                .await
                .insert((entry.layer, entry.path.clone()), entry);
            Ok("mock-commit".to_string())
        }

        async fn list(
            &self,
            _ctx: TenantContext,
            layer: KnowledgeLayer,
            prefix: &str,
        ) -> Result<Vec<KnowledgeEntry>, Self::Error> {
            Ok(self
                .items
                .read()
                .await
                .iter()
                .filter(|((entry_layer, path), _)| {
                    *entry_layer == layer && path.starts_with(prefix)
                })
                .map(|(_, value)| value.clone())
                .collect())
        }

        async fn delete(
            &self,
            _ctx: TenantContext,
            layer: KnowledgeLayer,
            path: &str,
            _message: &str,
        ) -> Result<String, Self::Error> {
            self.items.write().await.remove(&(layer, path.to_string()));
            Ok("mock-commit".to_string())
        }

        async fn get_head_commit(
            &self,
            _ctx: TenantContext,
        ) -> Result<Option<String>, Self::Error> {
            Ok(Some("mock-commit".to_string()))
        }

        async fn get_affected_items(
            &self,
            _ctx: TenantContext,
            _since_commit: &str,
        ) -> Result<Vec<(KnowledgeLayer, String)>, Self::Error> {
            Ok(vec![])
        }

        async fn search(
            &self,
            _ctx: TenantContext,
            query: &str,
            layers: Vec<KnowledgeLayer>,
            limit: usize,
        ) -> Result<Vec<KnowledgeEntry>, Self::Error> {
            Ok(self
                .items
                .read()
                .await
                .values()
                .filter(|entry| layers.contains(&entry.layer) && entry.content.contains(query))
                .take(limit)
                .cloned()
                .collect())
        }

        async fn store_promotion_request(
            &self,
            _ctx: TenantContext,
            request: PromotionRequest,
        ) -> Result<PromotionRequest, Self::Error> {
            self.promotions
                .write()
                .await
                .insert(request.id.clone(), request.clone());
            Ok(request)
        }

        async fn get_promotion_request(
            &self,
            _ctx: TenantContext,
            id: &str,
        ) -> Result<Option<PromotionRequest>, Self::Error> {
            Ok(self.promotions.read().await.get(id).cloned())
        }

        async fn update_promotion_request(
            &self,
            _ctx: TenantContext,
            request: PromotionRequest,
        ) -> Result<PromotionRequest, Self::Error> {
            self.promotions
                .write()
                .await
                .insert(request.id.clone(), request.clone());
            Ok(request)
        }

        async fn list_promotion_requests(
            &self,
            _ctx: TenantContext,
            status: Option<PromotionRequestStatus>,
        ) -> Result<Vec<PromotionRequest>, Self::Error> {
            Ok(self
                .promotions
                .read()
                .await
                .values()
                .filter(|request| status.as_ref().is_none_or(|value| &request.status == value))
                .cloned()
                .collect())
        }

        async fn store_relation(
            &self,
            _ctx: TenantContext,
            relation: KnowledgeRelation,
        ) -> Result<KnowledgeRelation, Self::Error> {
            self.relations.write().await.push(relation.clone());
            Ok(relation)
        }

        async fn get_relations_for_item(
            &self,
            _ctx: TenantContext,
            item_id: &str,
        ) -> Result<Vec<KnowledgeRelation>, Self::Error> {
            Ok(self
                .relations
                .read()
                .await
                .iter()
                .filter(|relation| relation.source_id == item_id || relation.target_id == item_id)
                .cloned()
                .collect())
        }

        fn root_path(&self) -> Option<std::path::PathBuf> {
            None
        }
    }

    struct MockTokenValidator;

    #[async_trait]
    impl TokenValidator for MockTokenValidator {
        async fn validate(&self, token: &str) -> WsResult<AuthToken> {
            Ok(AuthToken {
                user_id: token.to_string(),
                tenant_id: "default".to_string(),
                permissions: vec![],
                expires_at: 0,
            })
        }
    }

    struct TestNoopReasoner;

    #[async_trait]
    impl ReflectiveReasoner for TestNoopReasoner {
        async fn reason(
            &self,
            query: &str,
            _context_summary: Option<&str>,
        ) -> anyhow::Result<ReasoningTrace> {
            let now = chrono::Utc::now();
            Ok(ReasoningTrace {
                strategy: ReasoningStrategy::SemanticOnly,
                thought_process: "test noop".to_string(),
                refined_query: Some(query.to_string()),
                start_time: now,
                end_time: now,
                timed_out: false,
                duration_ms: 0,
                metadata: HashMap::new(),
            })
        }
    }

    fn discovery_router() -> Router {
        Router::new().route("/knowledge", get(discovery_handler))
    }

    fn sample_entry(path: &str) -> KnowledgeEntry {
        KnowledgeEntry {
            path: path.to_string(),
            content: "sample content".to_string(),
            layer: KnowledgeLayer::Project,
            kind: KnowledgeType::Spec,
            status: KnowledgeStatus::Draft,
            summaries: HashMap::new(),
            metadata: HashMap::new(),
            commit_hash: None,
            author: None,
            updated_at: 0,
        }
    }

    async fn app_with_repo(repo: Arc<MockRepo>) -> Option<Router> {
        let auth_service: Arc<dyn AuthorizationService<Error = anyhow::Error> + Send + Sync> =
            Arc::new(MockAuth);
        let fixture = postgres().await?;
        let postgres = Arc::new(PostgresBackend::new(fixture.url()).await.ok()?);
        postgres.initialize_schema().await.ok()?;
        let governance_engine = Arc::new(GovernanceEngine::new());
        let knowledge_manager = Arc::new(KnowledgeManager::new(
            Arc::new(
                knowledge::repository::GitRepository::new(tempfile::tempdir().unwrap().path())
                    .unwrap(),
            ),
            governance_engine.clone(),
        ));
        let memory_manager = Arc::new(MemoryManager::new());
        let sync_manager = Arc::new(
            SyncManager::new(
                memory_manager.clone(),
                knowledge_manager.clone(),
                config::config::DeploymentConfig::default(),
                None,
                Arc::new(sync::state_persister::FilePersister::new(
                    std::env::temp_dir(),
                )),
                None,
            )
            .await
            .unwrap(),
        );
        let dashboard = Arc::new(GovernanceDashboardApi::new(
            governance_engine.clone(),
            postgres.clone(),
            config::config::DeploymentConfig::default(),
        ));
        let mcp_server = Arc::new(McpServer::new(
            memory_manager.clone(),
            sync_manager.clone(),
            knowledge_manager.clone(),
            repo.clone(),
            postgres.clone(),
            governance_engine.clone(),
            Arc::new(TestNoopReasoner),
            auth_service.clone(),
            None,
            None,
            None,
        ));
        let (shutdown_tx, _) = tokio::sync::watch::channel(false);
        let tenant_store = Arc::new(TenantStore::new(postgres.pool().clone()));
        let tenant_repository_binding_store =
            Arc::new(TenantRepositoryBindingStore::new(postgres.pool().clone()));
        let git_provider_connection_registry = Arc::new(
            storage::git_provider_connection_store::InMemoryGitProviderConnectionStore::new(),
        );
        let tenant_repo_resolver = Arc::new(
            TenantRepositoryResolver::new(
                tenant_repository_binding_store.clone(),
                std::env::temp_dir(),
                Arc::new(LocalSecretProvider::new(HashMap::new())),
            )
            .with_connection_registry(git_provider_connection_registry.clone()),
        );

        Some(router(Arc::new(AppState {
            config: Arc::new(config::Config::default()),
            postgres: postgres.clone(),
            memory_manager,
            knowledge_manager,
            knowledge_repository: repo,
            governance_engine,
            governance_dashboard: dashboard,
            auth_service,
            mcp_server,
            sync_manager,
            git_provider: None,
            webhook_secret: None,
            event_publisher: None,
            graph_store: None,
            governance_storage: None,
            reasoner: None,
            ws_server: Arc::new(WsServer::new(Arc::new(MockTokenValidator))),
            a2a_config: Arc::new(agent_a2a::Config::default()),
            a2a_auth_state: Arc::new(agent_a2a::AuthState {
                api_key: None,
                jwt_secret: None,
                enabled: false,
                trusted_identity: TrustedIdentityConfig::default(),
            }),
            plugin_auth_state: Arc::new(PluginAuthState {
                config: config::PluginAuthConfig::default(),
                postgres: Some(postgres.clone()),
                refresh_store: RefreshTokenStoreBackend::InMemory(RefreshTokenStore::new()),
            }),
            revocation_cache: Default::default(),
            k8s_auth_config: config::KubernetesAuthConfig::default(),
            idp_config: None,
            idp_sync_service: None,
            idp_client: None,
            shutdown_tx: Arc::new(shutdown_tx),
            tenant_store,
            tenant_repository_binding_store,
            tenant_repo_resolver,
            tenant_config_provider: Arc::new(
                KubernetesTenantConfigProvider::new_in_memory_for_tests("default".to_string()),
            ),
            provider_registry: Arc::new(memory::provider_registry::TenantProviderRegistry::new(
                None, None,
            )),
            git_provider_connection_registry,
            redis_conn: None,
            redis_url: None,
            tenant_runtime_state: std::sync::Arc::new(
                crate::server::tenant_runtime_state::TenantRuntimeRegistry::new(),
            ),
            bootstrap_tracker: std::sync::Arc::new(
                crate::server::bootstrap_tracker::BootstrapTracker::new(),
            ),
        })))
    }

    #[tokio::test]
    async fn test_discovery_returns_endpoints() {
        let app = discovery_router();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/knowledge")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["service"], "aeterna-knowledge");
        assert!(json["endpoints"].as_array().unwrap().len() == 16);
    }

    #[test]
    fn test_parse_layer_valid() {
        assert!(matches!(
            parse_layer(Some("tenant")),
            Some(mk_core::types::KnowledgeLayer::Tenant)
        ));
        assert!(matches!(
            parse_layer(Some("team")),
            Some(mk_core::types::KnowledgeLayer::Team)
        ));
        assert!(matches!(
            parse_layer(Some("org")),
            Some(mk_core::types::KnowledgeLayer::Org)
        ));
        assert!(matches!(
            parse_layer(Some("project")),
            Some(mk_core::types::KnowledgeLayer::Project)
        ));
    }

    #[test]
    fn test_parse_layer_invalid() {
        assert!(parse_layer(None).is_none());
        assert!(parse_layer(Some("unknown")).is_none());
    }

    #[test]
    fn tags_round_trip_via_metadata() {
        let metadata = metadata_with_tags(
            vec!["a".to_string(), "b".to_string()],
            serde_json::Map::new(),
        );
        assert_eq!(tags_from_metadata(&metadata), vec!["a", "b"]);
    }

    #[tokio::test]
    async fn create_query_metadata_and_delete_flow_works() {
        let repo = Arc::new(MockRepo::new());
        let Some(app) = app_with_repo(repo.clone()).await else {
            eprintln!("Skipping OpenSpec test: Docker not available");
            return;
        };

        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/knowledge/create")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "content": "hello world",
                            "path": "specs/new.md",
                            "layer": "project",
                            "tags": ["alpha"]
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_response.status(), StatusCode::CREATED);

        let query_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/knowledge/query")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "query": "hello",
                            "limit": 10,
                            "layer": "project"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(query_response.status(), StatusCode::OK);
        let query_body = axum::body::to_bytes(query_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let query_json: serde_json::Value = serde_json::from_slice(&query_body).unwrap();
        assert_eq!(query_json["total"], 1);
        assert_eq!(query_json["items"][0]["path"], "specs/new.md");
        assert_eq!(query_json["items"][0]["tags"][0], "alpha");

        let update_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/knowledge/specs/new.md")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "content": "updated content",
                            "tags": ["beta"],
                            "metadata": { "owner": "platform" }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(update_response.status(), StatusCode::OK);

        let metadata_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/knowledge/specs/new.md/metadata")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(metadata_response.status(), StatusCode::OK);
        let metadata_body = axum::body::to_bytes(metadata_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let metadata_json: serde_json::Value = serde_json::from_slice(&metadata_body).unwrap();
        assert_eq!(metadata_json["id"], "specs/new.md");
        assert_eq!(metadata_json["path"], "specs/new.md");
        assert_eq!(metadata_json["layer"], "project");
        assert_eq!(metadata_json["tags"][0], "beta");

        let get_updated = repo
            .get(
                TenantContext::default(),
                KnowledgeLayer::Project,
                "specs/new.md",
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(get_updated.content, "updated content");
        assert_eq!(tags_from_metadata(&get_updated.metadata), vec!["beta"]);
        assert_eq!(get_updated.metadata["owner"], "platform");

        let delete_response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/knowledge/specs/new.md")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);
        assert!(
            repo.get(
                TenantContext::default(),
                KnowledgeLayer::Project,
                "specs/new.md"
            )
            .await
            .unwrap()
            .is_none()
        );
    }

    #[tokio::test]
    async fn batch_and_stream_endpoints_work() {
        let repo = Arc::new(MockRepo::new());
        let Some(app) = app_with_repo(repo).await else {
            eprintln!("Skipping OpenSpec test: Docker not available");
            return;
        };

        let batch_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/knowledge/batch")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "operations": [{
                                "action": "create",
                                "content": "batched",
                                "path": "specs/batch.md",
                                "layer": "project",
                                "tags": []
                            }]
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(batch_response.status(), StatusCode::OK);
        let batch_body = axum::body::to_bytes(batch_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let batch_json: serde_json::Value = serde_json::from_slice(&batch_body).unwrap();
        assert_eq!(batch_json["results"][0]["success"], true);
        assert_eq!(batch_json["results"][0]["id"], "specs/batch.md");

        let stream_response = app
            .oneshot(
                Request::builder()
                    .uri("/knowledge/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stream_response.status(), StatusCode::OK);
        let content_type = stream_response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert!(content_type.contains("text/event-stream"));
    }

    #[tokio::test]
    async fn promotion_endpoints_work() {
        let repo = Arc::new(MockRepo::new());
        let Some(app) = app_with_repo(repo.clone()).await else {
            eprintln!("Skipping OpenSpec test: Docker not available");
            return;
        };

        repo.store(
            TenantContext::default(),
            KnowledgeEntry {
                path: "specs/promo.md".to_string(),
                content: "shared content".to_string(),
                layer: KnowledgeLayer::Project,
                kind: KnowledgeType::Pattern,
                status: KnowledgeStatus::Accepted,
                summaries: HashMap::new(),
                metadata: HashMap::new(),
                commit_hash: Some("sha-1".to_string()),
                author: None,
                updated_at: 1,
            },
            "seed",
        )
        .await
        .unwrap();

        let preview_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/knowledge/specs/promo.md/promotions/preview")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "target_layer": "team",
                            "mode": "partial"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(preview_response.status(), StatusCode::OK);

        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/knowledge/specs/promo.md/promotions")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "target_layer": "team",
                            "mode": "partial",
                            "shared_content": "shared content",
                            "residual_content": "local details",
                            "residual_role": "specialization",
                            "source_version": "sha-1"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_response.status(), StatusCode::CREATED);
        let create_body = axum::body::to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let create_json: serde_json::Value = serde_json::from_slice(&create_body).unwrap();
        let promotion_id = create_json["id"].as_str().unwrap().to_string();

        let list_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/knowledge/promotions?status=pendingReview")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list_response.status(), StatusCode::OK);

        let get_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/knowledge/promotions/{promotion_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_response.status(), StatusCode::OK);

        let retarget_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/knowledge/promotions/{promotion_id}/retarget"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "target_layer": "org"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(retarget_response.status(), StatusCode::OK);

        let approve_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/knowledge/promotions/{promotion_id}/approve"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "decision": "approveAsSpecialization"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(approve_response.status(), StatusCode::OK);

        let reject_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/knowledge/specs/other.md/promotions")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "target_layer": "team",
                            "mode": "partial",
                            "shared_content": "other",
                            "source_version": "sha-2"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reject_response.status(), StatusCode::NOT_FOUND);

        let relation_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/knowledge/specs/promo.md/relations")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "target_id": "shared/team/promo",
                            "relation_type": "specializes"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(relation_response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn update_blocks_direct_status_and_variant_role_mutations() {
        let repo = Arc::new(MockRepo::new());
        let Some(app) = app_with_repo(repo.clone()).await else {
            eprintln!("Skipping OpenSpec test: Docker not available");
            return;
        };

        repo.store(
            TenantContext::default(),
            sample_entry("specs/mutate.md"),
            "seed",
        )
        .await
        .unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/knowledge/specs/mutate.md")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "metadata": {
                                "status": "accepted",
                                "variant_role": "canonical"
                            }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    async fn app_with_plugin_auth_enabled(repo: Arc<MockRepo>) -> Option<Router> {
        let auth_service: Arc<dyn AuthorizationService<Error = anyhow::Error> + Send + Sync> =
            Arc::new(MockAuth);
        let fixture = postgres().await?;
        let postgres = Arc::new(PostgresBackend::new(fixture.url()).await.ok()?);
        postgres.initialize_schema().await.ok()?;
        let governance_engine = Arc::new(GovernanceEngine::new());
        let knowledge_manager = Arc::new(KnowledgeManager::new(
            Arc::new(
                knowledge::repository::GitRepository::new(tempfile::tempdir().unwrap().path())
                    .unwrap(),
            ),
            governance_engine.clone(),
        ));
        let memory_manager = Arc::new(MemoryManager::new());
        let sync_manager = Arc::new(
            SyncManager::new(
                memory_manager.clone(),
                knowledge_manager.clone(),
                config::config::DeploymentConfig::default(),
                None,
                Arc::new(sync::state_persister::FilePersister::new(
                    std::env::temp_dir(),
                )),
                None,
            )
            .await
            .unwrap(),
        );
        let dashboard = Arc::new(GovernanceDashboardApi::new(
            governance_engine.clone(),
            postgres.clone(),
            config::config::DeploymentConfig::default(),
        ));
        let mcp_server = Arc::new(McpServer::new(
            memory_manager.clone(),
            sync_manager.clone(),
            knowledge_manager.clone(),
            repo.clone(),
            postgres.clone(),
            governance_engine.clone(),
            Arc::new(TestNoopReasoner),
            auth_service.clone(),
            None,
            None,
            None,
        ));
        let (shutdown_tx, _) = tokio::sync::watch::channel(false);
        let tenant_store = Arc::new(TenantStore::new(postgres.pool().clone()));
        let tenant_repository_binding_store =
            Arc::new(TenantRepositoryBindingStore::new(postgres.pool().clone()));
        let git_provider_connection_registry = Arc::new(
            storage::git_provider_connection_store::InMemoryGitProviderConnectionStore::new(),
        );
        let tenant_repo_resolver = Arc::new(
            TenantRepositoryResolver::new(
                tenant_repository_binding_store.clone(),
                std::env::temp_dir(),
                Arc::new(LocalSecretProvider::new(HashMap::new())),
            )
            .with_connection_registry(git_provider_connection_registry.clone()),
        );

        Some(router(Arc::new(AppState {
            config: Arc::new(config::Config::default()),
            postgres: postgres.clone(),
            memory_manager,
            knowledge_manager,
            knowledge_repository: repo,
            governance_engine,
            governance_dashboard: dashboard,
            auth_service,
            mcp_server,
            sync_manager,
            git_provider: None,
            webhook_secret: None,
            event_publisher: None,
            graph_store: None,
            governance_storage: None,
            reasoner: None,
            ws_server: Arc::new(WsServer::new(Arc::new(MockTokenValidator))),
            a2a_config: Arc::new(agent_a2a::Config::default()),
            a2a_auth_state: Arc::new(agent_a2a::AuthState {
                api_key: None,
                jwt_secret: None,
                enabled: false,
                trusted_identity: TrustedIdentityConfig::default(),
            }),
            plugin_auth_state: Arc::new(PluginAuthState {
                config: config::PluginAuthConfig {
                    enabled: true,
                    jwt_secret: Some("test-jwt-secret-at-least-32-chars-long!!".to_string()),
                    ..Default::default()
                },
                postgres: Some(postgres.clone()),
                refresh_store: RefreshTokenStoreBackend::InMemory(RefreshTokenStore::new()),
            }),
            revocation_cache: Default::default(),
            k8s_auth_config: config::KubernetesAuthConfig::default(),
            idp_config: None,
            idp_sync_service: None,
            idp_client: None,
            shutdown_tx: Arc::new(shutdown_tx),
            tenant_store,
            tenant_repository_binding_store,
            tenant_repo_resolver,
            tenant_config_provider: Arc::new(
                KubernetesTenantConfigProvider::new_in_memory_for_tests("default".to_string()),
            ),
            provider_registry: Arc::new(memory::provider_registry::TenantProviderRegistry::new(
                None, None,
            )),
            git_provider_connection_registry,
            redis_conn: None,
            redis_url: None,
            tenant_runtime_state: std::sync::Arc::new(
                crate::server::tenant_runtime_state::TenantRuntimeRegistry::new(),
            ),
            bootstrap_tracker: std::sync::Arc::new(
                crate::server::bootstrap_tracker::BootstrapTracker::new(),
            ),
        })))
    }

    // ── Task 4.1: knowledge API fail-closed when plugin auth enabled ──────────

    #[tokio::test]
    async fn knowledge_query_rejected_without_bearer_when_plugin_auth_enabled() {
        let repo = Arc::new(MockRepo::new());
        let Some(app) = app_with_plugin_auth_enabled(repo).await else {
            eprintln!("Skipping: Docker not available");
            return;
        };

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/knowledge/query")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "query": "anything",
                            "limit": 5
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "Knowledge query MUST be rejected (401) when plugin auth is enabled and no bearer is present"
        );
    }

    #[tokio::test]
    async fn knowledge_create_rejected_without_bearer_when_plugin_auth_enabled() {
        let repo = Arc::new(MockRepo::new());
        let Some(app) = app_with_plugin_auth_enabled(repo).await else {
            eprintln!("Skipping: Docker not available");
            return;
        };

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/knowledge/create")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "content": "test",
                            "path": "specs/t.md",
                            "layer": "project"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "Knowledge create MUST be rejected (401) when plugin auth is enabled and no bearer is present"
        );
    }

    // ── Task 12.2 — API tests: explicit reject and approve body shape ─────────

    #[tokio::test]
    async fn promotion_reject_endpoint_returns_200_with_rejected_status() {
        let repo = Arc::new(MockRepo::new());
        let Some(app) = app_with_repo(repo.clone()).await else {
            eprintln!("Skipping: Docker not available");
            return;
        };

        // Seed source item and create a pending promotion
        repo.store(
            TenantContext::default(),
            KnowledgeEntry {
                path: "specs/reject-test.md".to_string(),
                content: "reject test content".to_string(),
                layer: KnowledgeLayer::Project,
                kind: KnowledgeType::Spec,
                status: KnowledgeStatus::Accepted,
                summaries: HashMap::new(),
                metadata: HashMap::new(),
                commit_hash: Some("sha-rej".to_string()),
                author: None,
                updated_at: 1,
            },
            "seed",
        )
        .await
        .unwrap();

        let create_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/knowledge/specs/reject-test.md/promotions")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "target_layer": "team",
                            "mode": "full",
                            "shared_content": "reject test content",
                            "source_version": "sha-rej"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_resp.status(), StatusCode::CREATED);
        let create_body = axum::body::to_bytes(create_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let create_json: serde_json::Value = serde_json::from_slice(&create_body).unwrap();
        let promo_id = create_json["id"].as_str().unwrap().to_string();

        // Reject it
        let reject_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/knowledge/promotions/{promo_id}/reject"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "reason": "not ready for broader audience yet"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reject_resp.status(), StatusCode::OK);
        let reject_body = axum::body::to_bytes(reject_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let reject_json: serde_json::Value = serde_json::from_slice(&reject_body).unwrap();
        assert_eq!(
            reject_json["status"], "rejected",
            "status must be 'rejected' after reject endpoint"
        );
        assert_eq!(reject_json["id"], promo_id);
    }

    #[tokio::test]
    async fn promotion_approve_endpoint_returns_200_with_approved_status_and_id() {
        let repo = Arc::new(MockRepo::new());
        let Some(app) = app_with_repo(repo.clone()).await else {
            eprintln!("Skipping: Docker not available");
            return;
        };

        repo.store(
            TenantContext::default(),
            KnowledgeEntry {
                path: "specs/approve-shape.md".to_string(),
                content: "approve shape content".to_string(),
                layer: KnowledgeLayer::Project,
                kind: KnowledgeType::Spec,
                status: KnowledgeStatus::Accepted,
                summaries: HashMap::new(),
                metadata: HashMap::new(),
                commit_hash: Some("sha-app".to_string()),
                author: None,
                updated_at: 1,
            },
            "seed",
        )
        .await
        .unwrap();

        let create_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/knowledge/specs/approve-shape.md/promotions")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "target_layer": "team",
                            "mode": "full",
                            "shared_content": "approve shape content",
                            "source_version": "sha-app"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_resp.status(), StatusCode::CREATED);
        let create_body = axum::body::to_bytes(create_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let create_json: serde_json::Value = serde_json::from_slice(&create_body).unwrap();
        let promo_id = create_json["id"].as_str().unwrap().to_string();

        let approve_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/knowledge/promotions/{promo_id}/approve"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "decision": "approveAsReplacement"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(approve_resp.status(), StatusCode::OK);
        let approve_body = axum::body::to_bytes(approve_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let approve_json: serde_json::Value = serde_json::from_slice(&approve_body).unwrap();
        assert_eq!(
            approve_json["id"], promo_id,
            "response must include the promotion id"
        );
        assert_eq!(
            approve_json["status"], "approved",
            "status must be 'approved' after approve endpoint"
        );
    }

    // ── Task 12.6 — Tenant isolation tests ───────────────────────────────────

    #[tokio::test]
    async fn promotion_approve_forbidden_for_different_tenant() {
        let repo = Arc::new(MockRepo::new());
        let Some(app) = app_with_repo(repo.clone()).await else {
            eprintln!("Skipping: Docker not available");
            return;
        };

        // Seed a promotion that belongs to "tenant-a".
        // The app (plugin auth disabled) defaults callers to tenant "default",
        // so any promotion owned by a different tenant must be blocked.
        let other_tenant = mk_core::types::TenantId::new("tenant-a".to_string()).unwrap();
        let other_user = mk_core::types::UserId::new("user-a".to_string()).unwrap();
        let other_ctx = mk_core::types::TenantContext::new(other_tenant.clone(), other_user);

        let promo = PromotionRequest {
            id: uuid::Uuid::new_v4().to_string(),
            source_item_id: "specs/cross-tenant.md".to_string(),
            source_layer: KnowledgeLayer::Project,
            source_status: KnowledgeStatus::Accepted,
            target_layer: KnowledgeLayer::Team,
            promotion_mode: mk_core::types::PromotionMode::Full,
            shared_content: "cross tenant shared".to_string(),
            residual_content: None,
            residual_role: None,
            justification: None,
            status: PromotionRequestStatus::PendingReview,
            requested_by: other_ctx.user_id.clone(),
            tenant_id: other_tenant,
            source_version: "sha-x".to_string(),
            latest_decision: None,
            promoted_item_id: None,
            residual_item_id: None,
            created_at: 0,
            updated_at: 0,
        };
        let promo_id = promo.id.clone();
        repo.store_promotion_request(other_ctx, promo)
            .await
            .unwrap();

        // Caller context is "default" tenant (no bearer token, plugin auth disabled)
        let approve_resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/knowledge/promotions/{promo_id}/approve"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "decision": "approveAsReplacement"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            approve_resp.status(),
            StatusCode::FORBIDDEN,
            "Cross-tenant promotion approve must be blocked with 403"
        );
        let body = axum::body::to_bytes(approve_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "cross_tenant_forbidden");
    }

    #[tokio::test]
    async fn promotion_reject_forbidden_for_different_tenant() {
        let repo = Arc::new(MockRepo::new());
        let Some(app) = app_with_repo(repo.clone()).await else {
            eprintln!("Skipping: Docker not available");
            return;
        };

        let other_tenant = mk_core::types::TenantId::new("tenant-b".to_string()).unwrap();
        let other_user = mk_core::types::UserId::new("user-b".to_string()).unwrap();
        let other_ctx = mk_core::types::TenantContext::new(other_tenant.clone(), other_user);

        let promo = PromotionRequest {
            id: uuid::Uuid::new_v4().to_string(),
            source_item_id: "specs/tenant-b-item.md".to_string(),
            source_layer: KnowledgeLayer::Project,
            source_status: KnowledgeStatus::Accepted,
            target_layer: KnowledgeLayer::Team,
            promotion_mode: mk_core::types::PromotionMode::Full,
            shared_content: "tenant-b shared".to_string(),
            residual_content: None,
            residual_role: None,
            justification: None,
            status: PromotionRequestStatus::PendingReview,
            requested_by: other_ctx.user_id.clone(),
            tenant_id: other_tenant,
            source_version: "sha-b".to_string(),
            latest_decision: None,
            promoted_item_id: None,
            residual_item_id: None,
            created_at: 0,
            updated_at: 0,
        };
        let promo_id = promo.id.clone();
        repo.store_promotion_request(other_ctx, promo)
            .await
            .unwrap();

        let reject_resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/knowledge/promotions/{promo_id}/reject"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "reason": "not applicable"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            reject_resp.status(),
            StatusCode::FORBIDDEN,
            "Cross-tenant promotion reject must be blocked with 403"
        );
    }

    // ── Task 12.7 — Confidentiality/redaction tests ───────────────────────────

    #[tokio::test]
    async fn promotion_create_blocked_when_shared_content_contains_secret() {
        let repo = Arc::new(MockRepo::new());
        let Some(app) = app_with_repo(repo.clone()).await else {
            eprintln!("Skipping: Docker not available");
            return;
        };

        repo.store(
            TenantContext::default(),
            KnowledgeEntry {
                path: "specs/secret-item.md".to_string(),
                content: "db_password=supersecret123".to_string(),
                layer: KnowledgeLayer::Project,
                kind: KnowledgeType::Spec,
                status: KnowledgeStatus::Accepted,
                summaries: HashMap::new(),
                metadata: HashMap::new(),
                commit_hash: Some("sha-sec".to_string()),
                author: None,
                updated_at: 1,
            },
            "seed",
        )
        .await
        .unwrap();

        // Attempt to promote with shared_content containing "password="
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/knowledge/specs/secret-item.md/promotions")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "target_layer": "team",
                            "mode": "full",
                            "shared_content": "db_password=supersecret123",
                            "source_version": "sha-sec"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "Promotion with sensitive shared_content must be blocked"
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "sensitive_content_detected");
    }

    #[tokio::test]
    async fn promotion_create_blocked_when_shared_content_contains_api_key() {
        let repo = Arc::new(MockRepo::new());
        let Some(app) = app_with_repo(repo.clone()).await else {
            eprintln!("Skipping: Docker not available");
            return;
        };

        repo.store(
            TenantContext::default(),
            KnowledgeEntry {
                path: "specs/api-key-item.md".to_string(),
                content: "api_key: sk-1234567890abcdef".to_string(),
                layer: KnowledgeLayer::Project,
                kind: KnowledgeType::Spec,
                status: KnowledgeStatus::Accepted,
                summaries: HashMap::new(),
                metadata: HashMap::new(),
                commit_hash: Some("sha-apikey".to_string()),
                author: None,
                updated_at: 1,
            },
            "seed",
        )
        .await
        .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/knowledge/specs/api-key-item.md/promotions")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "target_layer": "team",
                            "mode": "full",
                            "shared_content": "api_key: sk-1234567890abcdef",
                            "source_version": "sha-apikey"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "Promotion with api_key in shared_content must be blocked"
        );
    }

    // ── Task 12.8 — Idempotency and retry tests ───────────────────────────────

    #[tokio::test]
    async fn promotion_approve_is_idempotent_on_retry() {
        let repo = Arc::new(MockRepo::new());
        let Some(app) = app_with_repo(repo.clone()).await else {
            eprintln!("Skipping: Docker not available");
            return;
        };

        repo.store(
            TenantContext::default(),
            KnowledgeEntry {
                path: "specs/idempotent.md".to_string(),
                content: "idempotent content".to_string(),
                layer: KnowledgeLayer::Project,
                kind: KnowledgeType::Spec,
                status: KnowledgeStatus::Accepted,
                summaries: HashMap::new(),
                metadata: HashMap::new(),
                commit_hash: Some("sha-idem".to_string()),
                author: None,
                updated_at: 1,
            },
            "seed",
        )
        .await
        .unwrap();

        let create_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/knowledge/specs/idempotent.md/promotions")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "target_layer": "team",
                            "mode": "full",
                            "shared_content": "idempotent content",
                            "source_version": "sha-idem"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_resp.status(), StatusCode::CREATED);
        let create_body = axum::body::to_bytes(create_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let create_json: serde_json::Value = serde_json::from_slice(&create_body).unwrap();
        let promo_id = create_json["id"].as_str().unwrap().to_string();

        // First approve
        let approve_body = serde_json::to_vec(&serde_json::json!({
            "decision": "approveAsReplacement"
        }))
        .unwrap();
        let resp1 = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/knowledge/promotions/{promo_id}/approve"))
                    .header("content-type", "application/json")
                    .body(Body::from(approve_body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);
        let body1 = axum::body::to_bytes(resp1.into_body(), usize::MAX)
            .await
            .unwrap();
        let json1: serde_json::Value = serde_json::from_slice(&body1).unwrap();

        // Second approve (retry) — already approved, should return the same record idempotently
        let resp2 = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/knowledge/promotions/{promo_id}/approve"))
                    .header("content-type", "application/json")
                    .body(Body::from(approve_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        // Either 200 (idempotent) or 409 (invalid transition) — both are valid
        // per the spec; what must NOT happen is a 500.
        assert_ne!(
            resp2.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "Retry of approve must not cause 500"
        );
        let body2 = axum::body::to_bytes(resp2.into_body(), usize::MAX)
            .await
            .unwrap();
        let json2: serde_json::Value = serde_json::from_slice(&body2).unwrap();

        // If idempotent 200, the id must match; if 409, error field must be present
        if json2.get("id").is_some() {
            assert_eq!(
                json2["id"], json1["id"],
                "Idempotent approve must return same id"
            );
        } else {
            assert!(
                json2.get("error").is_some(),
                "Non-200 retry must include an error field"
            );
        }
    }

    // ── Task 12.9 — Concurrency / stale-review tests ─────────────────────────

    #[tokio::test]
    async fn promotion_approve_returns_409_on_optimistic_conflict() {
        let repo = Arc::new(MockRepo::new());
        let Some(app) = app_with_repo(repo.clone()).await else {
            eprintln!("Skipping: Docker not available");
            return;
        };

        repo.store(
            TenantContext::default(),
            KnowledgeEntry {
                path: "specs/conflict.md".to_string(),
                content: "conflict content".to_string(),
                layer: KnowledgeLayer::Project,
                kind: KnowledgeType::Spec,
                status: KnowledgeStatus::Accepted,
                summaries: HashMap::new(),
                metadata: HashMap::new(),
                commit_hash: Some("sha-conf".to_string()),
                author: None,
                updated_at: 2,
            },
            "seed",
        )
        .await
        .unwrap();

        let create_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/knowledge/specs/conflict.md/promotions")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "target_layer": "team",
                            "mode": "full",
                            "shared_content": "conflict content",
                            "source_version": "sha-conf"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_resp.status(), StatusCode::CREATED);
        let create_body = axum::body::to_bytes(create_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let create_json: serde_json::Value = serde_json::from_slice(&create_body).unwrap();
        let promo_id = create_json["id"].as_str().unwrap().to_string();

        // Approve with a stale client_version (0), but the promotion has updated_at = 0
        // at creation so this should conflict when updated_at != provided version.
        // We simulate a stale version by providing client_version = 9999 (far ahead).
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/knowledge/promotions/{promo_id}/approve"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "decision": "approveAsReplacement",
                            "client_version": 9999
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::CONFLICT,
            "Stale client_version must return 409 Conflict"
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "optimistic_conflict");
    }

    #[tokio::test]
    async fn promotion_approve_returns_409_when_source_version_stale() {
        let repo = Arc::new(MockRepo::new());
        let Some(app) = app_with_repo(repo.clone()).await else {
            eprintln!("Skipping: Docker not available");
            return;
        };

        // Seed item at sha-v1, then "update" it to sha-v2
        repo.store(
            TenantContext::default(),
            KnowledgeEntry {
                path: "specs/stale-src.md".to_string(),
                content: "original content".to_string(),
                layer: KnowledgeLayer::Project,
                kind: KnowledgeType::Spec,
                status: KnowledgeStatus::Accepted,
                summaries: HashMap::new(),
                metadata: HashMap::new(),
                commit_hash: Some("sha-v1".to_string()),
                author: None,
                updated_at: 1,
            },
            "seed v1",
        )
        .await
        .unwrap();

        let create_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/knowledge/specs/stale-src.md/promotions")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "target_layer": "team",
                            "mode": "full",
                            "shared_content": "original content",
                            "source_version": "sha-v1"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_resp.status(), StatusCode::CREATED);
        let create_body = axum::body::to_bytes(create_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let create_json: serde_json::Value = serde_json::from_slice(&create_body).unwrap();
        let promo_id = create_json["id"].as_str().unwrap().to_string();

        // Now update the source item to sha-v2 — promotion source_version is now stale
        repo.store(
            TenantContext::default(),
            KnowledgeEntry {
                path: "specs/stale-src.md".to_string(),
                content: "updated content — breaks sha-v1 reference".to_string(),
                layer: KnowledgeLayer::Project,
                kind: KnowledgeType::Spec,
                status: KnowledgeStatus::Accepted,
                summaries: HashMap::new(),
                metadata: HashMap::new(),
                commit_hash: Some("sha-v2".to_string()),
                author: None,
                updated_at: 2,
            },
            "update to v2",
        )
        .await
        .unwrap();

        // Attempt to approve — source is now at sha-v2, promotion locked to sha-v1
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/knowledge/promotions/{promo_id}/approve"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "decision": "approveAsReplacement"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::CONFLICT,
            "Stale source version must return 409 Conflict"
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "stale_promotion");
    }
}
