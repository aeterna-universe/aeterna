use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use adapters::auth::matrix::{ALL_ACTIONS, role_permission_matrix};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use knowledge::tenant_repo_resolver::RepoResolutionError;
use memory::provider_registry::config_keys;
use mk_core::traits::{StorageBackend, TenantConfigProvider};
use mk_core::types::{
    BranchPolicy, CredentialKind, GitProviderConnection, GitProviderKind, GovernanceEvent,
    OrganizationalUnit, PersistentEvent, RecordSource, RepositoryKind, Role, RoleIdentifier,
    TenantConfigDocument, TenantConfigField, TenantConfigOwnership, TenantContext,
    TenantSecretEntry, TenantSecretReference, UnitType,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use storage::PrincipalType;
use storage::git_provider_connection_store::GitProviderConnectionError;
use storage::governance::AuditExtensions;
use storage::tenant_config_provider::TenantConfigProviderError;
use storage::tenant_store::UpsertTenantRepositoryBinding;
use uuid::Uuid;

use super::{
    AppState, authenticated_tenant_context, manifest_render, tenant_diff, tenant_scoped_context,
};

const OWNERSHIP_PLATFORM: &str = "platform";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTenantRequest {
    pub slug: String,
    pub name: String,
    pub account_id: Option<String>,
    pub environment: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTenantRequest {
    pub slug: Option<String>,
    pub name: Option<String>,
    pub environment: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TenantListQuery {
    #[serde(default)]
    pub include_inactive: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateTenantDomainMappingRequest {
    pub domain: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertTenantConfigRequest {
    #[serde(default)]
    pub fields: BTreeMap<String, TenantConfigField>,
    #[serde(default)]
    pub secret_references: BTreeMap<String, TenantSecretReference>,
}

/// Inbound API body for `PUT /tenants/:id/config/secrets/:name`.
///
/// `secret_value` is a [`mk_core::SecretBytes`] rather than a `String`: the
/// plaintext is wrapped into the zeroize-on-drop container at the serde
/// boundary (see the `Deserialize` impl on `SecretBytes`) so it never
/// reaches normal logging or `Debug` output.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTenantSecretRequest {
    #[serde(default = "default_tenant_ownership")]
    pub ownership: TenantConfigOwnership,
    pub secret_value: mk_core::SecretBytes,
}

// ---------------------------------------------------------------------------
// Git provider connection request/response types (task 3.4)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGitProviderConnectionRequest {
    /// Optional stable connection identifier. When omitted, the server generates a UUID.
    pub id: Option<String>,
    pub name: String,
    pub provider_kind: GitProviderKind,
    pub app_id: u64,
    pub installation_id: u64,
    /// Secret-provider reference to the PEM private key (must use local/, secret/, arn:aws: prefix).
    pub pem_secret_ref: String,
    /// Optional secret-provider reference to the webhook secret.
    pub webhook_secret_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTenantRepositoryBindingRequest {
    pub kind: RepositoryKind,
    pub local_path: Option<String>,
    pub remote_url: Option<String>,
    pub branch: String,
    pub branch_policy: BranchPolicy,
    pub credential_kind: CredentialKind,
    pub credential_ref: Option<String>,
    pub github_owner: Option<String>,
    pub github_repo: Option<String>,
    #[serde(default)]
    pub source_owner: RecordSource,
    /// Reference a platform-owned Git provider connection by ID.
    /// When set, `credential_ref` is not required for GitHubApp bindings.
    pub git_provider_connection_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct HierarchyListQuery {
    pub parent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateHierarchyUnitRequest {
    pub name: String,
    pub unit_type: UnitType,
    pub parent_id: Option<String>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
    /// Source ownership: defaults to `admin` for API-created units.
    #[serde(default)]
    pub source_owner: RecordSource,
}

#[derive(Debug, Deserialize)]
pub struct UpdateHierarchyUnitRequest {
    pub name: Option<String>,
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
pub struct AssignUnitRoleRequest {
    pub user_id: String,
    pub role: RoleIdentifier,
}

#[derive(Debug, Deserialize)]
pub struct UserRoleListQuery {
    pub user_id: String,
}

#[derive(Debug, Serialize)]
struct UnitMemberRoleResponse {
    user_id: String,
    role: RoleIdentifier,
}

#[derive(Debug, Serialize)]
struct UserScopedRoleResponse {
    unit_id: String,
    role: RoleIdentifier,
}

#[derive(Debug, Serialize)]
struct TenantResponse<T> {
    success: bool,
    tenant: T,
}

#[derive(Debug, Serialize)]
struct AccountResponse<T> {
    success: bool,
    account: T,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateAccountRequest {
    slug: String,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateAccountRequest {
    slug: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachAccountRequest {
    account_id: String,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/admin/accounts", get(list_accounts).post(create_account))
        .route(
            "/admin/accounts/{account}",
            get(show_account).patch(update_account).delete(delete_account),
        )
        .route("/admin/accounts/{account}/tenants", get(list_account_tenants))
        .route("/admin/tenants", get(list_tenants).post(create_tenant))
        .route(
            "/admin/tenants/{tenant}",
            get(show_tenant).patch(update_tenant),
        )
        .route(
            "/admin/tenants/{tenant}/deactivate",
            post(deactivate_tenant),
        )
        .route(
            "/admin/tenants/{tenant}/account",
            post(attach_tenant_account).delete(detach_tenant_account),
        )
        .route(
            "/admin/tenants/{tenant}/purge",
            post(purge_tenant),
        )
        .route(
            "/admin/tenants/{tenant}/domain-mappings",
            post(add_domain_mapping),
        )
        .route(
            "/admin/tenants/{tenant}/repository-binding",
            get(show_tenant_repository_binding).put(set_tenant_repository_binding),
        )
        .route(
            "/admin/tenants/{tenant}/repository-binding/validate",
            post(validate_tenant_repository_binding),
        )
        .route(
            "/admin/tenants/{tenant}/config",
            get(inspect_tenant_config).put(upsert_tenant_config),
        )
        .route(
            "/admin/tenants/{tenant}/config/validate",
            post(validate_tenant_config),
        )
        .route(
            "/admin/tenants/{tenant}/secrets/{logical_name}",
            put(set_tenant_secret).delete(delete_tenant_secret),
        )
        .route(
            "/admin/tenant-config",
            get(inspect_my_tenant_config).put(upsert_my_tenant_config),
        )
        .route(
            "/admin/tenant-config/validate",
            post(validate_my_tenant_config),
        )
        .route(
            "/admin/tenant-config/secrets/{logical_name}",
            put(set_my_tenant_secret).delete(delete_my_tenant_secret),
        )
        .route(
            "/admin/hierarchy",
            get(list_hierarchy_units).post(create_hierarchy_unit),
        )
        .route(
            "/admin/hierarchy/{unit}",
            get(show_hierarchy_unit).patch(update_hierarchy_unit),
        )
        .route(
            "/admin/hierarchy/{unit}/ancestors",
            get(list_hierarchy_ancestors),
        )
        .route(
            "/admin/hierarchy/{unit}/descendants",
            get(list_hierarchy_descendants),
        )
        .route(
            "/admin/hierarchy/{unit}/members",
            get(list_unit_members).post(assign_unit_role),
        )
        .route(
            "/admin/hierarchy/{unit}/members/{user_id}/roles/{role}",
            delete(remove_unit_role),
        )
        .route("/admin/memberships", get(list_user_memberships))
        .route("/admin/permissions/matrix", get(get_permission_matrix))
        .route(
            "/admin/permissions/effective",
            get(get_effective_permissions),
        )
        // Git provider connection routes (task 3.4)
        .route(
            "/admin/git-provider-connections",
            get(list_git_provider_connections).post(create_git_provider_connection),
        )
        .route(
            "/admin/git-provider-connections/{connection_id}",
            get(show_git_provider_connection),
        )
        .route(
            "/admin/git-provider-connections/{connection_id}/tenants/{tenant}",
            post(grant_git_provider_connection_to_tenant)
                .delete(revoke_git_provider_connection_from_tenant),
        )
        .route(
            "/admin/tenants/{tenant}/git-provider-connections",
            get(list_tenant_git_provider_connections),
        )
        .route(
            "/admin/tenants/provision",
            post(provision_tenant),
        )
        .route(
            "/admin/tenants/diff",
            post(diff_tenant),
        )
        // Provider configuration routes
        .route(
            "/admin/tenants/{tenant}/providers",
            get(get_tenant_providers),
        )
        .route(
            "/admin/tenants/{tenant}/providers/llm",
            put(set_tenant_llm_provider).delete(delete_tenant_llm_provider),
        )
        .route(
            "/admin/tenants/{tenant}/providers/embedding",
            put(set_tenant_embedding_provider).delete(delete_tenant_embedding_provider),
        )
        .route(
            "/admin/tenants/{tenant}/providers/status",
            get(test_tenant_provider_connectivity),
        )
        .with_state(state)
}

fn default_tenant_ownership() -> TenantConfigOwnership {
    TenantConfigOwnership::Tenant
}

fn tenant_config_from_request(
    tenant_id: mk_core::types::TenantId,
    req: UpsertTenantConfigRequest,
) -> TenantConfigDocument {
    TenantConfigDocument {
        tenant_id,
        fields: req.fields,
        secret_references: req.secret_references,
    }
}

/// Flatten `manifest.providers.{llm,embedding}` into the in-progress
/// config document so the provider registry can resolve them the same
/// way it resolves state written by `PUT /admin/tenants/{t}/providers/{llm,embedding}`.
///
/// Closes the forward half of FINDINGS-2-2 §2.2-A: before this helper,
/// `validate_manifest` would accept a `providers:` block and
/// `provision_tenant` would silently drop it, producing a tenant with
/// no provider config even though the apply returned success.
///
/// What gets written (mirroring the dedicated `set_provider_llm` /
/// `set_provider_embedding` handlers at lines 2441+ / 2580+):
///
/// - `llm_provider` / `embedding_provider`  ← `provider.kind`
/// - `llm_model` / `embedding_model`        ← `provider.model` (skipped if absent)
/// - `{llm,embedding}_google_project_id`    ← `provider.config["projectId"]`
///   when `kind == "google"`
/// - `{llm,embedding}_google_location`      ← `provider.config["location"]`
///   when `kind == "google"`
/// - `{llm,embedding}_bedrock_region`       ← `provider.config["region"]`
///   when `kind == "bedrock"`
///
/// All fields are written with `TenantConfigOwnership::Platform` —
/// consistent with the dedicated PUT handler (operators can't override
/// provider selection via TenantAdmin config writes), and consistent
/// with the ownership filter in the manifest validator.
///
/// Secret aliasing: if `provider.secret_ref` is set and resolves inside
/// `doc.secret_references`, the resolved `TenantSecretReference` is
/// cloned into the map under the canonical key the provider registry
/// reads (`llm_api_key` / `embedding_api_key`). This gives us a
/// functional API-key pointer without duplicating secret bytes — both
/// the operator-friendly name (e.g. `openai_key`) and the canonical
/// name point at the same backing `SecretReference` variant.
///
/// Memory layers are *not* handled here — see FINDINGS-2-2 §2.2-D;
/// `providers.memory_layers` has no config-key convention in the
/// codebase at all, and both forward and reverse sides need that
/// decision before wiring.
///
/// `validate_manifest` is assumed to have run already; this helper
/// does not re-validate `kind` non-empty or `secret_ref` resolution.
fn apply_manifest_providers_to_config(
    providers: &ManifestProviders,
    doc: &mut TenantConfigDocument,
) {
    // Helper closure: write a single provider block under a given key
    // prefix (e.g. "llm_" or "embedding_").
    let mut apply_one = |prefix: &str, provider: &ManifestProvider| {
        let pf = |suffix: &str| format!("{prefix}{suffix}");
        // `TenantConfigOwnership` isn't `Copy`, so we construct a fresh
        // `Platform` marker at each insertion site rather than
        // cloning a local. The enum has no payload — construction is
        // free and clearer than `.clone()` noise.
        let platform = || TenantConfigOwnership::Platform;

        doc.fields.insert(
            pf("provider"),
            TenantConfigField {
                ownership: platform(),
                value: serde_json::json!(provider.kind),
            },
        );

        if let Some(model) = &provider.model {
            doc.fields.insert(
                pf("model"),
                TenantConfigField {
                    ownership: platform(),
                    value: serde_json::json!(model),
                },
            );
        }

        // Google Vertex: project_id + location. Manifest uses camelCase
        // in the `provider.config` map (projectId / location) to stay
        // consistent with the rest of the manifest JSON convention;
        // storage uses snake_case keys under `config_keys`.
        if provider.kind == "google" {
            if let Some(project_id) = provider.config.get("projectId") {
                doc.fields.insert(
                    pf("google_project_id"),
                    TenantConfigField {
                        ownership: platform(),
                        value: serde_json::json!(project_id),
                    },
                );
            }
            if let Some(location) = provider.config.get("location") {
                doc.fields.insert(
                    pf("google_location"),
                    TenantConfigField {
                        ownership: platform(),
                        value: serde_json::json!(location),
                    },
                );
            }
        }

        // Bedrock: region.
        if provider.kind == "bedrock"
            && let Some(region) = provider.config.get("region")
        {
            doc.fields.insert(
                pf("bedrock_region"),
                TenantConfigField {
                    ownership: platform(),
                    value: serde_json::json!(region),
                },
            );
        }

        // Secret alias: canonical key → same reference as operator name.
        if let Some(secret_ref) = &provider.secret_ref
            && let Some(src) = doc.secret_references.get(secret_ref).cloned()
        {
            doc.secret_references.insert(pf("api_key"), src);
        }
    };

    if let Some(llm) = &providers.llm {
        apply_one("llm_", llm);
    }
    if let Some(embedding) = &providers.embedding {
        apply_one("embedding_", embedding);
    }
    // B3 §2.2-D — memoryLayers storage convention.
    //
    // Each entry `providers.memoryLayers.{layer}` writes under the
    // prefix `memory_layer.{layer}.`. The dot at the prefix boundary
    // guarantees non-collision with the flat snake_case namespace used
    // by llm/embedding/… (`llm_provider` ≠ `memory_layer.episodic.provider`
    // and no existing key contains `.`).
    //
    // Layer-key validity is enforced upstream by
    // `is_valid_memory_layer_key` in `validate_manifest`; that guard
    // runs before any write, so we can assume the key is safe here.
    // A defensive re-check would add no value and would only paper
    // over a validator regression.
    for (layer, provider) in &providers.memory_layers {
        let prefix = format!("memory_layer.{layer}.");
        apply_one(&prefix, provider);
    }
}

fn reject_non_tenant_owned_config(
    doc: &TenantConfigDocument,
) -> Result<(), axum::response::Response> {
    let platform_field = doc
        .fields
        .iter()
        .find(|(_, field)| field.ownership == TenantConfigOwnership::Platform)
        .map(|(key, _)| key.clone());
    if let Some(key) = platform_field {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            &format!("TenantAdmin cannot mutate platform-owned config field '{key}'"),
        ));
    }

    let platform_secret_ref = doc
        .secret_references
        .iter()
        .find(|(_, value)| value.ownership == TenantConfigOwnership::Platform)
        .map(|(key, _)| key.clone());
    if let Some(key) = platform_secret_ref {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            &format!("TenantAdmin cannot mutate platform-owned secret reference '{key}'"),
        ));
    }

    Ok(())
}

fn map_tenant_config_provider_error(
    operation: &str,
    err: TenantConfigProviderError,
) -> axum::response::Response {
    match err {
        TenantConfigProviderError::Validation(message) => error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!("tenant_config_{operation}_invalid"),
            &message,
        ),
        TenantConfigProviderError::InvalidTenantId(message) => error_response(
            StatusCode::BAD_REQUEST,
            &format!("tenant_config_{operation}_failed"),
            &message,
        ),
        // Secret-backend failures (KMS unreachable, AEAD tampering, DB
        // error) are internal. The message is intentionally generic so we
        // do not leak KMS ARNs, key ids, or row shapes to API clients;
        // the root cause lands in the structured log via `err`.
        TenantConfigProviderError::Secret(err) => {
            tracing::error!(operation, error = ?err, "tenant secret backend failure");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("tenant_config_{operation}_failed"),
                "tenant secret backend is unavailable",
            )
        }
    }
}

async fn resolve_tenant_record_or_404(
    state: &AppState,
    tenant_ref: &str,
    operation_error: &str,
) -> Result<mk_core::types::TenantRecord, axum::response::Response> {
    match state.tenant_store.get_tenant(tenant_ref).await {
        Ok(Some(record)) => Ok(record),
        Ok(None) => Err(error_response(
            StatusCode::NOT_FOUND,
            "tenant_not_found",
            "Tenant not found",
        )),
        Err(err) => Err(error_response(
            StatusCode::BAD_REQUEST,
            operation_error,
            &err.to_string(),
        )),
    }
}

async fn inspect_tenant_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
) -> impl IntoResponse {
    if let Err(response) = require_platform_admin(&state, &headers).await {
        return response;
    }

    let tenant_record =
        match resolve_tenant_record_or_404(&state, &tenant, "tenant_config_inspect_failed").await {
            Ok(record) => record,
            Err(response) => return response,
        };

    match state
        .tenant_config_provider
        .get_config(&tenant_record.id)
        .await
    {
        Ok(config) => (
            StatusCode::OK,
            Json(json!({ "success": true, "config": config })),
        )
            .into_response(),
        Err(err) => map_tenant_config_provider_error("inspect", err),
    }
}

async fn upsert_tenant_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
    Json(req): Json<UpsertTenantConfigRequest>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let tenant_record =
        match resolve_tenant_record_or_404(&state, &tenant, "tenant_config_upsert_failed").await {
            Ok(record) => record,
            Err(response) => return response,
        };
    let document = tenant_config_from_request(tenant_record.id.clone(), req);

    match state.tenant_config_provider.upsert_config(document).await {
        Ok(config) => {
            state.provider_registry.invalidate_tenant(&tenant_record.id);
            audit_tenant_action(
                state.as_ref(),
                &ctx,
                "tenant_config_upsert",
                Some(tenant_record.id.as_str()),
                json!({
                    "tenantId": tenant_record.id.as_str(),
                    "fieldCount": config.fields.len(),
                    "secretReferenceCount": config.secret_references.len()
                }),
            )
            .await;
            (
                StatusCode::OK,
                Json(json!({ "success": true, "config": config })),
            )
                .into_response()
        }
        Err(err) => map_tenant_config_provider_error("upsert", err),
    }
}

async fn validate_tenant_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
    Json(req): Json<UpsertTenantConfigRequest>,
) -> impl IntoResponse {
    if let Err(response) = require_platform_admin(&state, &headers).await {
        return response;
    }

    let tenant_record = match resolve_tenant_record_or_404(
        &state,
        &tenant,
        "tenant_config_validate_failed",
    )
    .await
    {
        Ok(record) => record,
        Err(response) => return response,
    };
    let document = tenant_config_from_request(tenant_record.id.clone(), req);

    match state.tenant_config_provider.validate(&document).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "success": true, "valid": true, "config": document })),
        )
            .into_response(),
        Err(TenantConfigProviderError::Validation(message)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "success": false,
                "valid": false,
                "error": "tenant_config_invalid",
                "message": message,
            })),
        )
            .into_response(),
        Err(err) => map_tenant_config_provider_error("validate", err),
    }
}

async fn set_tenant_secret(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((tenant, logical_name)): Path<(String, String)>,
    Json(req): Json<SetTenantSecretRequest>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let tenant_record =
        match resolve_tenant_record_or_404(&state, &tenant, "tenant_secret_set_failed").await {
            Ok(record) => record,
            Err(response) => return response,
        };

    let secret = TenantSecretEntry {
        logical_name: logical_name.clone(),
        ownership: req.ownership,
        secret_value: req.secret_value,
    };
    match state
        .tenant_config_provider
        .set_secret_entry(&tenant_record.id, secret)
        .await
    {
        Ok(reference) => {
            state.provider_registry.invalidate_tenant(&tenant_record.id);
            audit_tenant_action(
                state.as_ref(),
                &ctx,
                "tenant_config_secret_set",
                Some(tenant_record.id.as_str()),
                json!({
                    "tenantId": tenant_record.id.as_str(),
                    "logicalName": reference.logical_name,
                    "ownership": reference.ownership,
                }),
            )
            .await;
            (
                StatusCode::OK,
                Json(json!({ "success": true, "secretReference": reference })),
            )
                .into_response()
        }
        Err(err) => map_tenant_config_provider_error("secret_set", err),
    }
}

async fn delete_tenant_secret(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((tenant, logical_name)): Path<(String, String)>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let tenant_record =
        match resolve_tenant_record_or_404(&state, &tenant, "tenant_secret_delete_failed").await {
            Ok(record) => record,
            Err(response) => return response,
        };

    // Guard: TenantAdmin cannot delete a platform-owned secret entry.
    // Resolve the existing config to check ownership before proceeding.
    if !ctx.has_known_role(&Role::PlatformAdmin) {
        match state
            .tenant_config_provider
            .get_config(&tenant_record.id)
            .await
        {
            Ok(Some(existing)) => {
                if let Some(ref_entry) = existing.secret_references.get(&logical_name)
                    && ref_entry.ownership == TenantConfigOwnership::Platform
                {
                    return error_response(
                        StatusCode::FORBIDDEN,
                        "forbidden",
                        "TenantAdmin cannot delete a platform-owned secret entry",
                    );
                }
            }
            Ok(None) => {}
            Err(err) => return map_tenant_config_provider_error("secret_delete", err),
        }
    }

    match state
        .tenant_config_provider
        .delete_secret_entry(&tenant_record.id, &logical_name)
        .await
    {
        Ok(deleted) => {
            state.provider_registry.invalidate_tenant(&tenant_record.id);
            audit_tenant_action(
                state.as_ref(),
                &ctx,
                "tenant_config_secret_delete",
                Some(tenant_record.id.as_str()),
                json!({
                    "tenantId": tenant_record.id.as_str(),
                    "logicalName": logical_name,
                    "deleted": deleted,
                }),
            )
            .await;
            (
                StatusCode::OK,
                Json(json!({ "success": true, "deleted": deleted })),
            )
                .into_response()
        }
        Err(err) => map_tenant_config_provider_error("secret_delete", err),
    }
}

async fn inspect_my_tenant_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_tenant_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let tenant_ref = ctx
        .target_tenant_id
        .as_ref()
        .unwrap_or(&ctx.tenant_id)
        .as_str();
    let tenant_record = match resolve_tenant_record_or_404(
        &state,
        tenant_ref,
        "tenant_config_inspect_failed",
    )
    .await
    {
        Ok(record) => record,
        Err(response) => return response,
    };

    match state
        .tenant_config_provider
        .get_config(&tenant_record.id)
        .await
    {
        Ok(config) => (
            StatusCode::OK,
            Json(json!({ "success": true, "config": config })),
        )
            .into_response(),
        Err(err) => map_tenant_config_provider_error("inspect", err),
    }
}

async fn upsert_my_tenant_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<UpsertTenantConfigRequest>,
) -> impl IntoResponse {
    let ctx = match require_tenant_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let tenant_ref = ctx
        .target_tenant_id
        .as_ref()
        .unwrap_or(&ctx.tenant_id)
        .as_str();
    let tenant_record =
        match resolve_tenant_record_or_404(&state, tenant_ref, "tenant_config_upsert_failed").await
        {
            Ok(record) => record,
            Err(response) => return response,
        };
    let document = tenant_config_from_request(tenant_record.id.clone(), req);
    if let Err(response) = reject_non_tenant_owned_config(&document) {
        return response;
    }

    match state.tenant_config_provider.upsert_config(document).await {
        Ok(config) => {
            state.provider_registry.invalidate_tenant(&tenant_record.id);
            audit_tenant_action(
                state.as_ref(),
                &ctx,
                "tenant_config_upsert",
                Some(tenant_record.id.as_str()),
                json!({
                    "tenantId": tenant_record.id.as_str(),
                    "fieldCount": config.fields.len(),
                    "secretReferenceCount": config.secret_references.len()
                }),
            )
            .await;
            (
                StatusCode::OK,
                Json(json!({ "success": true, "config": config })),
            )
                .into_response()
        }
        Err(err) => map_tenant_config_provider_error("upsert", err),
    }
}

async fn validate_my_tenant_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<UpsertTenantConfigRequest>,
) -> impl IntoResponse {
    let ctx = match require_tenant_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let tenant_ref = ctx
        .target_tenant_id
        .as_ref()
        .unwrap_or(&ctx.tenant_id)
        .as_str();
    let tenant_record =
        match resolve_tenant_record_or_404(&state, tenant_ref, "tenant_config_validate_failed")
            .await
        {
            Ok(record) => record,
            Err(response) => return response,
        };
    let document = tenant_config_from_request(tenant_record.id.clone(), req);
    if let Err(response) = reject_non_tenant_owned_config(&document) {
        return response;
    }

    match state.tenant_config_provider.validate(&document).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "success": true, "valid": true, "config": document })),
        )
            .into_response(),
        Err(TenantConfigProviderError::Validation(message)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "success": false,
                "valid": false,
                "error": "tenant_config_invalid",
                "message": message,
            })),
        )
            .into_response(),
        Err(err) => map_tenant_config_provider_error("validate", err),
    }
}

async fn set_my_tenant_secret(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(logical_name): Path<String>,
    Json(req): Json<SetTenantSecretRequest>,
) -> impl IntoResponse {
    let ctx = match require_tenant_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    if req.ownership != TenantConfigOwnership::Tenant {
        return error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "TenantAdmin cannot mutate platform-owned secret entries",
        );
    }

    let tenant_ref = ctx
        .target_tenant_id
        .as_ref()
        .unwrap_or(&ctx.tenant_id)
        .as_str();
    let tenant_record =
        match resolve_tenant_record_or_404(&state, tenant_ref, "tenant_secret_set_failed").await {
            Ok(record) => record,
            Err(response) => return response,
        };

    let secret = TenantSecretEntry {
        logical_name: logical_name.clone(),
        ownership: req.ownership,
        secret_value: req.secret_value,
    };

    match state
        .tenant_config_provider
        .set_secret_entry(&tenant_record.id, secret)
        .await
    {
        Ok(reference) => {
            state.provider_registry.invalidate_tenant(&tenant_record.id);
            audit_tenant_action(
                state.as_ref(),
                &ctx,
                "tenant_config_secret_set",
                Some(tenant_record.id.as_str()),
                json!({
                    "tenantId": tenant_record.id.as_str(),
                    "logicalName": reference.logical_name,
                    "ownership": reference.ownership,
                }),
            )
            .await;
            (
                StatusCode::OK,
                Json(json!({ "success": true, "secretReference": reference })),
            )
                .into_response()
        }
        Err(err) => map_tenant_config_provider_error("secret_set", err),
    }
}

async fn delete_my_tenant_secret(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(logical_name): Path<String>,
) -> impl IntoResponse {
    let ctx = match require_tenant_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let tenant_ref = ctx
        .target_tenant_id
        .as_ref()
        .unwrap_or(&ctx.tenant_id)
        .as_str();
    let tenant_record =
        match resolve_tenant_record_or_404(&state, tenant_ref, "tenant_secret_delete_failed").await
        {
            Ok(record) => record,
            Err(response) => return response,
        };

    let existing_config = match state
        .tenant_config_provider
        .get_config(&tenant_record.id)
        .await
    {
        Ok(config) => config,
        Err(err) => return map_tenant_config_provider_error("secret_delete", err),
    };

    if let Some(reference) = existing_config
        .as_ref()
        .and_then(|config| config.secret_references.get(&logical_name))
        && reference.ownership == TenantConfigOwnership::Platform
        && !ctx.has_known_role(&Role::PlatformAdmin)
    {
        return error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "TenantAdmin cannot delete platform-owned secret entries",
        );
    }

    match state
        .tenant_config_provider
        .delete_secret_entry(&tenant_record.id, &logical_name)
        .await
    {
        Ok(deleted) => {
            state.provider_registry.invalidate_tenant(&tenant_record.id);
            audit_tenant_action(
                state.as_ref(),
                &ctx,
                "tenant_config_secret_delete",
                Some(tenant_record.id.as_str()),
                json!({
                    "tenantId": tenant_record.id.as_str(),
                    "logicalName": logical_name,
                    "deleted": deleted,
                }),
            )
            .await;
            (
                StatusCode::OK,
                Json(json!({ "success": true, "deleted": deleted })),
            )
                .into_response()
        }
        Err(err) => map_tenant_config_provider_error("secret_delete", err),
    }
}

async fn create_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateAccountRequest>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let store = storage::AccountStore::new(state.postgres.pool().clone());
    match store.create(&req.slug, &req.name).await {
        Ok(record) => {
            audit_tenant_action(
                &state,
                &ctx,
                "account_create",
                Some(&record.id),
                json!({ "slug": record.slug, "name": record.name }),
            )
            .await;
            (
                StatusCode::CREATED,
                Json(AccountResponse {
                    success: true,
                    account: record,
                }),
            )
                .into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "account_create_failed",
            &err.to_string(),
        ),
    }
}

async fn list_accounts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(response) = require_platform_admin_or_scope(&state, &headers, "tenants:read").await {
        return response;
    }

    let store = storage::AccountStore::new(state.postgres.pool().clone());
    match store.list().await {
        Ok(accounts) => (
            StatusCode::OK,
            Json(json!({ "success": true, "accounts": accounts })),
        )
            .into_response(),
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "account_list_failed",
            &err.to_string(),
        ),
    }
}

async fn show_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(account): Path<String>,
) -> impl IntoResponse {
    if let Err(response) = require_platform_admin_or_scope(&state, &headers, "tenants:read").await {
        return response;
    }

    let store = storage::AccountStore::new(state.postgres.pool().clone());
    match store.get(&account).await {
        Ok(Some(record)) => (
            StatusCode::OK,
            Json(AccountResponse {
                success: true,
                account: record,
            }),
        )
            .into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "account_not_found",
            "Account not found",
        ),
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "account_show_failed",
            &err.to_string(),
        ),
    }
}

async fn update_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(account): Path<String>,
    Json(req): Json<UpdateAccountRequest>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let store = storage::AccountStore::new(state.postgres.pool().clone());
    match store
        .update(&account, req.slug.as_deref(), req.name.as_deref())
        .await
    {
        Ok(Some(record)) => {
            audit_tenant_action(
                &state,
                &ctx,
                "account_update",
                Some(&record.id),
                json!({ "slug": record.slug, "name": record.name }),
            )
            .await;
            (
                StatusCode::OK,
                Json(AccountResponse {
                    success: true,
                    account: record,
                }),
            )
                .into_response()
        }
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "account_not_found",
            "Account not found",
        ),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "account_update_failed",
            &err.to_string(),
        ),
    }
}

async fn delete_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(account): Path<String>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let store = storage::AccountStore::new(state.postgres.pool().clone());
    match store.soft_delete(&account).await {
        Ok(Some(record)) => {
            audit_tenant_action(
                &state,
                &ctx,
                "account_delete",
                Some(&record.id),
                json!({ "slug": record.slug, "name": record.name }),
            )
            .await;
            (
                StatusCode::OK,
                Json(AccountResponse {
                    success: true,
                    account: record,
                }),
            )
                .into_response()
        }
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "account_not_found",
            "Account not found",
        ),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "account_delete_failed",
            &err.to_string(),
        ),
    }
}

async fn list_account_tenants(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(account): Path<String>,
) -> impl IntoResponse {
    if let Err(response) = require_platform_admin_or_scope(&state, &headers, "tenants:read").await {
        return response;
    }

    let store = storage::AccountStore::new(state.postgres.pool().clone());
    match store.list_tenants(&account).await {
        Ok(tenants) => (
            StatusCode::OK,
            Json(json!({ "success": true, "tenants": tenants })),
        )
            .into_response(),
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "account_tenant_list_failed",
            &err.to_string(),
        ),
    }
}

async fn attach_tenant_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
    Json(req): Json<AttachAccountRequest>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    match state
        .tenant_store
        .attach_account(&tenant, &req.account_id)
        .await
    {
        Ok(Some(record)) => {
            audit_tenant_action(
                &state,
                &ctx,
                "tenant_account_attach",
                Some(record.id.as_str()),
                json!({ "tenant": record.slug, "account": record.account }),
            )
            .await;
            (
                StatusCode::OK,
                Json(TenantResponse {
                    success: true,
                    tenant: record,
                }),
            )
                .into_response()
        }
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "tenant_or_account_not_found",
            "Tenant or account not found",
        ),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "tenant_account_attach_failed",
            &err.to_string(),
        ),
    }
}

async fn detach_tenant_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    match state.tenant_store.detach_account(&tenant).await {
        Ok(Some(record)) => {
            audit_tenant_action(
                &state,
                &ctx,
                "tenant_account_detach",
                Some(record.id.as_str()),
                json!({ "tenant": record.slug }),
            )
            .await;
            (
                StatusCode::OK,
                Json(TenantResponse {
                    success: true,
                    tenant: record,
                }),
            )
                .into_response()
        }
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "tenant_not_found",
            "Tenant not found",
        ),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "tenant_account_detach_failed",
            &err.to_string(),
        ),
    }
}

async fn create_tenant(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateTenantRequest>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    match state
        .tenant_store
        .create_tenant_with_metadata(
            &req.slug,
            &req.name,
            RecordSource::Admin,
            req.account_id.as_deref(),
            req.environment.as_deref(),
        )
        .await
    {
        Ok(record) => {
            let now = chrono::Utc::now().timestamp();
            let tenant_event = GovernanceEvent::TenantCreated {
                record_id: record.id.as_str().to_string(),
                slug: record.slug.clone(),
                tenant_id: record.id.clone(),
                timestamp: now,
            };
            persist_governance_event(&state, &ctx, &tenant_event).await;
            audit_tenant_action(
                &state,
                &ctx,
                "tenant_create",
                Some(record.id.as_str()),
                json!({
                    "slug": record.slug,
                    "name": record.name,
                    "account": record.account,
                    "environment": record.environment,
                }),
            )
            .await;
            (
                StatusCode::CREATED,
                Json(TenantResponse {
                    success: true,
                    tenant: record,
                }),
            )
                .into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "tenant_create_failed",
            &err.to_string(),
        ),
    }
}

async fn list_tenants(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<TenantListQuery>,
) -> impl IntoResponse {
    // B2 §10.5 — accept either a PlatformAdmin user OR a service token
    // carrying the `tenants:read` scope. Read-only endpoint, no audit
    // rows, so the principal is discarded after the gate.
    if let Err(response) = require_platform_admin_or_scope(&state, &headers, "tenants:read").await {
        return response;
    }

    match state
        .tenant_store
        .list_tenants(query.include_inactive)
        .await
    {
        Ok(tenants) => (
            StatusCode::OK,
            Json(json!({
                // `scope: "all"` marks the response as cross-tenant per the
                // RFC envelope convention. The existing `tenants` field is
                // kept for backward compatibility with pre-#44.d clients.
                "success": true,
                "scope": "all",
                "tenants": tenants,
            })),
        )
            .into_response(),
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "tenant_list_failed",
            &err.to_string(),
        ),
    }
}

async fn show_tenant(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
) -> impl IntoResponse {
    // B2 §10.5 — PA user OR service token with `tenants:read`.
    if let Err(response) = require_platform_admin_or_scope(&state, &headers, "tenants:read").await {
        return response;
    }

    match state.tenant_store.get_tenant(&tenant).await {
        Ok(Some(record)) => (
            StatusCode::OK,
            Json(TenantResponse {
                success: true,
                tenant: record,
            }),
        )
            .into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "tenant_not_found",
            "Tenant not found",
        ),
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "tenant_show_failed",
            &err.to_string(),
        ),
    }
}

async fn update_tenant(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
    Json(req): Json<UpdateTenantRequest>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    match state
        .tenant_store
        .update_tenant(
            &tenant,
            req.slug.as_deref(),
            req.name.as_deref(),
            req.environment.as_deref(),
        )
        .await
    {
        Ok(Some(record)) => {
            let now = chrono::Utc::now().timestamp();
            let tenant_event = GovernanceEvent::TenantUpdated {
                record_id: record.id.as_str().to_string(),
                tenant_id: record.id.clone(),
                timestamp: now,
            };
            persist_governance_event(&state, &ctx, &tenant_event).await;
            audit_tenant_action(
                &state,
                &ctx,
                "tenant_update",
                Some(record.id.as_str()),
                json!({
                    "slug": record.slug,
                    "name": record.name,
                    "account": record.account,
                    "environment": record.environment,
                }),
            )
            .await;
            (
                StatusCode::OK,
                Json(TenantResponse {
                    success: true,
                    tenant: record,
                }),
            )
                .into_response()
        }
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "tenant_not_found",
            "Tenant not found",
        ),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "tenant_update_failed",
            &err.to_string(),
        ),
    }
}

async fn deactivate_tenant(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    match state.tenant_store.deactivate_tenant(&tenant).await {
        Ok(Some(record)) => {
            let now = chrono::Utc::now().timestamp();
            let tenant_event = GovernanceEvent::TenantDeactivated {
                record_id: record.id.as_str().to_string(),
                tenant_id: record.id.clone(),
                timestamp: now,
            };
            persist_governance_event(&state, &ctx, &tenant_event).await;
            audit_tenant_action(
                &state,
                &ctx,
                "tenant_deactivate",
                Some(record.id.as_str()),
                json!({ "slug": record.slug, "status": record.status }),
            )
            .await;
            (
                StatusCode::OK,
                Json(TenantResponse {
                    success: true,
                    tenant: record,
                }),
            )
                .into_response()
        }
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "tenant_not_found",
            "Tenant not found",
        ),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "tenant_deactivate_failed",
            &err.to_string(),
        ),
    }
}

/// Full data purge for a deactivated tenant.
///
/// This endpoint cascades deletion across PostgreSQL, DuckDB graph, and Redis.
/// It should only be called after the quarantine period has elapsed.
async fn purge_tenant(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let cascade = storage::CascadeDeleter::new(&state.postgres, state.graph_store.clone());
    let report = cascade
        .cascade_tenant_purge::<fn(String) -> std::future::Ready<Result<(), Box<dyn std::error::Error + Send + Sync>>>, _>(
            &tenant,
            None, // Redis connection not available here; GDPR flow handles Redis separately
            None, // Qdrant callback — future: wire through MemoryManager providers
        )
        .await;

    audit_tenant_action(
        &state,
        &ctx,
        "tenant_purge",
        Some(&tenant),
        json!({
            "memories_deleted": report.memories.postgres_deleted,
            "knowledge_deleted": report.knowledge_items_deleted,
            "org_units_deleted": report.org_units_deleted,
            "user_roles_deleted": report.user_roles_deleted,
            "unit_policies_deleted": report.unit_policies_deleted,
            "errors": report.errors,
        }),
    )
    .await;

    (
        StatusCode::OK,
        Json(json!({ "success": true, "report": report })),
    )
        .into_response()
}

async fn add_domain_mapping(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
    Json(req): Json<CreateTenantDomainMappingRequest>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    match state
        .tenant_store
        .add_verified_domain_mapping(&tenant, &req.domain)
        .await
    {
        Ok(record) => {
            audit_tenant_action(
                &state,
                &ctx,
                "tenant_domain_mapping_add",
                Some(record.id.as_str()),
                json!({
                    "domain": req.domain,
                    "tenantId": record.id.as_str(),
                    "selectedTargetTenantId": ctx.target_tenant_id.as_ref().map(mk_core::TenantId::as_str)
                }),
            )
            .await;
            (
                StatusCode::CREATED,
                Json(json!({
                    "success": true,
                    "tenant": record,
                    "domain": req.domain,
                    "verified": true,
                })),
            )
                .into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "tenant_domain_mapping_failed",
            &err.to_string(),
        ),
    }
}

async fn show_tenant_repository_binding(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
) -> impl IntoResponse {
    if let Err(response) = require_platform_admin(&state, &headers).await {
        return response;
    }

    let tenant_record = match state.tenant_store.get_tenant(&tenant).await {
        Ok(Some(record)) => record,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "tenant_not_found",
                "Tenant not found",
            );
        }
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "tenant_binding_show_failed",
                &err.to_string(),
            );
        }
    };

    match state
        .tenant_repository_binding_store
        .get_binding(&tenant_record.id)
        .await
    {
        Ok(Some(binding)) => (
            StatusCode::OK,
            Json(json!({ "success": true, "binding": binding.redacted() })),
        )
            .into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "binding_not_found",
            "No repository binding configured for tenant",
        ),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "tenant_binding_show_failed",
            &err.to_string(),
        ),
    }
}

async fn set_tenant_repository_binding(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
    Json(req): Json<SetTenantRepositoryBindingRequest>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let tenant_record = match state.tenant_store.get_tenant(&tenant).await {
        Ok(Some(record)) => record,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "tenant_not_found",
                "Tenant not found",
            );
        }
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "tenant_binding_set_failed",
                &err.to_string(),
            );
        }
    };

    let previous_binding = match state
        .tenant_repository_binding_store
        .get_binding(&tenant_record.id)
        .await
    {
        Ok(binding) => binding,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "tenant_binding_set_failed",
                &err.to_string(),
            );
        }
    };

    // Validate that the credential_ref is a supported secret reference (not raw
    // secret material) before persisting (task 3.5).
    {
        let candidate_ref = mk_core::types::TenantRepositoryBinding {
            id: String::new(),
            tenant_id: tenant_record.id.clone(),
            kind: req.kind.clone(),
            local_path: req.local_path.clone(),
            remote_url: req.remote_url.clone(),
            branch: req.branch.clone(),
            branch_policy: req.branch_policy.clone(),
            credential_kind: req.credential_kind.clone(),
            credential_ref: req.credential_ref.clone(),
            github_owner: req.github_owner.clone(),
            github_repo: req.github_repo.clone(),
            source_owner: req.source_owner.clone(),
            git_provider_connection_id: req.git_provider_connection_id.clone(),
            // Sentinel timestamps for an unsaved validation candidate; storage
            // overwrites these on persistence.
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        if let Err(reason) = candidate_ref.validate_credential_ref() {
            return error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_credential_ref",
                &reason,
            );
        }
    }

    // When a connection_id is provided, validate that the tenant has visibility
    // before persisting (fail-closed: if the registry is absent, reject).
    if let Some(ref conn_id) = req.git_provider_connection_id {
        let allowed = state
            .git_provider_connection_registry
            .tenant_can_use(conn_id, &tenant_record.id)
            .await
            .map_err(|e: GitProviderConnectionError| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "connection_registry_error",
                    &e.to_string(),
                )
            });
        match allowed {
            Err(response) => return response,
            Ok(false) => {
                return error_response(
                    StatusCode::FORBIDDEN,
                    "connection_not_allowed",
                    &format!(
                        "Tenant '{}' is not in the allow-list for Git provider connection '{conn_id}'",
                        tenant_record.id.as_str()
                    ),
                );
            }
            Ok(true) => {}
        }
    }

    let binding_request = UpsertTenantRepositoryBinding {
        tenant_id: tenant_record.id.clone(),
        kind: req.kind,
        local_path: req.local_path,
        remote_url: req.remote_url,
        branch: req.branch,
        branch_policy: req.branch_policy,
        credential_kind: req.credential_kind,
        credential_ref: req.credential_ref,
        github_owner: req.github_owner,
        github_repo: req.github_repo,
        source_owner: req.source_owner,
        git_provider_connection_id: req.git_provider_connection_id,
    };

    match state
        .tenant_repository_binding_store
        .upsert_binding(binding_request)
        .await
    {
        Ok(binding) => {
            let now = chrono::Utc::now().timestamp();
            let event = if previous_binding.is_some() {
                GovernanceEvent::RepositoryBindingUpdated {
                    binding_id: binding.id.clone(),
                    tenant_id: binding.tenant_id.clone(),
                    timestamp: now,
                }
            } else {
                GovernanceEvent::RepositoryBindingCreated {
                    binding_id: binding.id.clone(),
                    tenant_id: binding.tenant_id.clone(),
                    timestamp: now,
                }
            };
            persist_governance_event(state.as_ref(), &ctx, &event).await;
            state.tenant_repo_resolver.invalidate(&binding.tenant_id);
            audit_tenant_action(
                state.as_ref(),
                &ctx,
                "tenant_repository_binding_set",
                Some(binding.id.as_str()),
                json!({
                    "tenantId": binding.tenant_id.as_str(),
                    "kind": binding.kind,
                    "branch": binding.branch,
                    "branchPolicy": binding.branch_policy,
                    "credentialKind": binding.credential_kind,
                    "sourceOwner": binding.source_owner,
                }),
            )
            .await;

            (
                StatusCode::OK,
                Json(json!({ "success": true, "binding": binding.redacted() })),
            )
                .into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "tenant_binding_set_failed",
            &err.to_string(),
        ),
    }
}

async fn validate_tenant_repository_binding(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
    Json(req): Json<SetTenantRepositoryBindingRequest>,
) -> impl IntoResponse {
    if let Err(response) = require_platform_admin(&state, &headers).await {
        return response;
    }

    let tenant_record = match state.tenant_store.get_tenant(&tenant).await {
        Ok(Some(record)) => record,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "tenant_not_found",
                "Tenant not found",
            );
        }
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "tenant_binding_validate_failed",
                &err.to_string(),
            );
        }
    };

    let candidate = mk_core::types::TenantRepositoryBinding {
        id: "validation".to_string(),
        tenant_id: tenant_record.id.clone(),
        kind: req.kind,
        local_path: req.local_path,
        remote_url: req.remote_url,
        branch: req.branch,
        branch_policy: req.branch_policy,
        credential_kind: req.credential_kind,
        credential_ref: req.credential_ref,
        github_owner: req.github_owner,
        github_repo: req.github_repo,
        source_owner: req.source_owner,
        git_provider_connection_id: None,
        // Sentinel timestamps for an unsaved validation candidate; storage
        // overwrites these on persistence.
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    match state
        .tenant_repo_resolver
        .validate_binding(&tenant_record.id, &candidate)
        .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "success": true, "valid": true, "binding": candidate.redacted() })),
        )
            .into_response(),
        Err(RepoResolutionError::InvalidBinding { reason, .. }) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "success": false,
                "valid": false,
                "error": "repository_binding_invalid",
                "message": reason,
            })),
        )
            .into_response(),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "tenant_binding_validate_failed",
            &err.to_string(),
        ),
    }
}

async fn list_hierarchy_units(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<HierarchyListQuery>,
) -> impl IntoResponse {
    let ctx = match require_tenant_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let result = if let Some(parent_id) = query.parent_id.as_deref() {
        state.postgres.list_children_scoped(&ctx, parent_id).await
    } else {
        state.postgres.list_all_units().await.map(|units| {
            let mut units: Vec<_> = units
                .into_iter()
                .filter(|unit| unit.tenant_id == ctx.tenant_id && unit.parent_id.is_none())
                .collect();
            units.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
            units
        })
    };

    match result {
        Ok(units) => (
            StatusCode::OK,
            Json(json!({ "success": true, "units": units })),
        )
            .into_response(),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "hierarchy_list_failed",
            &err.to_string(),
        ),
    }
}

async fn create_hierarchy_unit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateHierarchyUnitRequest>,
) -> impl IntoResponse {
    let ctx = match require_tenant_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let now = chrono::Utc::now().timestamp();
    let unit = OrganizationalUnit {
        id: Uuid::new_v4().to_string(),
        name: req.name,
        unit_type: req.unit_type,
        parent_id: req.parent_id,
        tenant_id: ctx.tenant_id.clone(),
        metadata: req.metadata,
        source_owner: req.source_owner,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    match state.postgres.create_unit_scoped(&ctx, &unit).await {
        Ok(()) => {
            persist_governance_event(
                state.as_ref(),
                &ctx,
                &GovernanceEvent::UnitCreated {
                    unit_id: unit.id.clone(),
                    unit_type: unit.unit_type,
                    tenant_id: ctx.tenant_id.clone(),
                    parent_id: unit.parent_id.clone(),
                    timestamp: now,
                },
            )
            .await;
            audit_hierarchy_action(
                state.as_ref(),
                &ctx,
                "hierarchy_unit_create",
                Some(unit.id.as_str()),
                json!({
                    "name": unit.name,
                    "unitType": unit.unit_type,
                    "parentId": unit.parent_id,
                }),
            )
            .await;

            (
                StatusCode::CREATED,
                Json(json!({ "success": true, "unit": unit })),
            )
                .into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "hierarchy_unit_create_failed",
            &err.to_string(),
        ),
    }
}

async fn show_hierarchy_unit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(unit): Path<String>,
) -> impl IntoResponse {
    let ctx = match require_tenant_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    match state.postgres.get_unit_scoped(&ctx, &unit).await {
        Ok(Some(unit)) => (
            StatusCode::OK,
            Json(json!({ "success": true, "unit": unit })),
        )
            .into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "hierarchy_unit_not_found",
            "Hierarchy unit not found",
        ),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "hierarchy_unit_show_failed",
            &err.to_string(),
        ),
    }
}

async fn update_hierarchy_unit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(unit): Path<String>,
    Json(req): Json<UpdateHierarchyUnitRequest>,
) -> impl IntoResponse {
    let ctx = match require_tenant_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let Some(mut existing) = (match state.postgres.get_unit_scoped(&ctx, &unit).await {
        Ok(unit) => unit,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "hierarchy_unit_update_failed",
                &err.to_string(),
            );
        }
    }) else {
        return error_response(
            StatusCode::NOT_FOUND,
            "hierarchy_unit_not_found",
            "Hierarchy unit not found",
        );
    };

    if let Some(name) = req.name {
        existing.name = name;
    }
    if let Some(metadata) = req.metadata {
        existing.metadata = metadata;
    }
    existing.updated_at = chrono::Utc::now();

    match state.postgres.update_unit_scoped(&ctx, &existing).await {
        Ok(()) => {
            persist_governance_event(
                state.as_ref(),
                &ctx,
                &GovernanceEvent::UnitUpdated {
                    unit_id: existing.id.clone(),
                    tenant_id: ctx.tenant_id.clone(),
                    // GovernanceEvent.timestamp is i64 epoch (event-bus protocol);
                    // OrganizationalUnit.updated_at is now DateTime<Utc> so we
                    // project here at the boundary.
                    timestamp: existing.updated_at.timestamp(),
                },
            )
            .await;
            audit_hierarchy_action(
                state.as_ref(),
                &ctx,
                "hierarchy_unit_update",
                Some(existing.id.as_str()),
                json!({
                    "name": existing.name,
                    "unitType": existing.unit_type,
                    "parentId": existing.parent_id,
                }),
            )
            .await;

            (
                StatusCode::OK,
                Json(json!({ "success": true, "unit": existing })),
            )
                .into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "hierarchy_unit_update_failed",
            &err.to_string(),
        ),
    }
}

async fn list_hierarchy_ancestors(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(unit): Path<String>,
) -> impl IntoResponse {
    let ctx = match require_tenant_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    match state.postgres.get_ancestors_scoped(&ctx, &unit).await {
        Ok(units) => (
            StatusCode::OK,
            Json(json!({ "success": true, "units": units })),
        )
            .into_response(),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "hierarchy_ancestors_failed",
            &err.to_string(),
        ),
    }
}

async fn list_hierarchy_descendants(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(unit): Path<String>,
) -> impl IntoResponse {
    let ctx = match require_tenant_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    match state
        .postgres
        .get_unit_descendants_scoped(&ctx, &unit)
        .await
    {
        Ok(units) => (
            StatusCode::OK,
            Json(json!({ "success": true, "units": units })),
        )
            .into_response(),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "hierarchy_descendants_failed",
            &err.to_string(),
        ),
    }
}

async fn list_unit_members(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(unit): Path<String>,
) -> impl IntoResponse {
    let ctx = match require_tenant_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    match state.postgres.get_unit_scoped(&ctx, &unit).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "hierarchy_unit_not_found",
                "Hierarchy unit not found",
            );
        }
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "membership_list_failed",
                &err.to_string(),
            );
        }
    }

    match state.postgres.list_unit_roles_scoped(&ctx, &unit).await {
        Ok(entries) => {
            let members: Vec<_> = entries
                .into_iter()
                .map(|(user_id, role)| UnitMemberRoleResponse {
                    user_id: user_id.into_inner(),
                    role,
                })
                .collect();
            (
                StatusCode::OK,
                Json(json!({ "success": true, "unitId": unit, "members": members })),
            )
                .into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "membership_list_failed",
            &err.to_string(),
        ),
    }
}

async fn assign_unit_role(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(unit): Path<String>,
    Json(req): Json<AssignUnitRoleRequest>,
) -> impl IntoResponse {
    let ctx = match require_tenant_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    // Gate: caller must have AssignRoles permission on the tenant resource.
    let authz_resource = format!("Aeterna::Tenant::\"{}\"", ctx.tenant_id.as_str());
    match state
        .auth_service
        .check_permission(&ctx, "AssignRoles", &authz_resource)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            return error_response(
                StatusCode::FORBIDDEN,
                "forbidden",
                "AssignRoles permission required for this tenant",
            );
        }
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "authz_check_failed",
                &e.to_string(),
            );
        }
    }

    if matches!(req.role, RoleIdentifier::Known(Role::PlatformAdmin)) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_role_assignment",
            "PlatformAdmin cannot be assigned as a tenant-scoped hierarchy role",
        );
    }

    match state.postgres.get_unit_scoped(&ctx, &unit).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "hierarchy_unit_not_found",
                "Hierarchy unit not found",
            );
        }
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "membership_assign_failed",
                &err.to_string(),
            );
        }
    }

    let user_id = match mk_core::types::UserId::new(req.user_id.clone()) {
        Some(user_id) => user_id,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_user_id",
                "Invalid user id",
            );
        }
    };

    match state
        .postgres
        .assign_role_scoped(&ctx, &user_id, &unit, req.role.clone())
        .await
    {
        Ok(()) => {
            let now = chrono::Utc::now().timestamp();
            persist_governance_event(
                state.as_ref(),
                &ctx,
                &GovernanceEvent::RoleAssigned {
                    user_id: user_id.clone(),
                    unit_id: unit.clone(),
                    role: req.role.clone(),
                    tenant_id: ctx.tenant_id.clone(),
                    timestamp: now,
                },
            )
            .await;
            audit_membership_action(
                state.as_ref(),
                &ctx,
                "membership_role_assign",
                Some(unit.as_str()),
                json!({
                    "unitId": unit,
                    "userId": user_id.as_str(),
                    "role": req.role,
                }),
            )
            .await;

            (
                StatusCode::CREATED,
                Json(json!({
                    "success": true,
                    "membership": {
                        "unitId": unit,
                        "userId": user_id.as_str(),
                        "role": req.role,
                    }
                })),
            )
                .into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "membership_assign_failed",
            &err.to_string(),
        ),
    }
}

async fn remove_unit_role(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((unit, user_id, role)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let ctx = match require_tenant_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    // Gate: caller must have AssignRoles permission on the tenant resource.
    let authz_resource = format!("Aeterna::Tenant::\"{}\"", ctx.tenant_id.as_str());
    match state
        .auth_service
        .check_permission(&ctx, "AssignRoles", &authz_resource)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            return error_response(
                StatusCode::FORBIDDEN,
                "forbidden",
                "AssignRoles permission required for this tenant",
            );
        }
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "authz_check_failed",
                &e.to_string(),
            );
        }
    }

    match state.postgres.get_unit_scoped(&ctx, &unit).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "hierarchy_unit_not_found",
                "Hierarchy unit not found",
            );
        }
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "membership_remove_failed",
                &err.to_string(),
            );
        }
    }

    let user_id = match mk_core::types::UserId::new(user_id) {
        Some(user_id) => user_id,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_user_id",
                "Invalid user id",
            );
        }
    };
    let role = RoleIdentifier::from_str_flexible(&role);

    if matches!(role, RoleIdentifier::Known(Role::PlatformAdmin)) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_role_assignment",
            "PlatformAdmin cannot be assigned as a tenant-scoped hierarchy role",
        );
    }

    match state
        .postgres
        .remove_role_scoped(&ctx, &user_id, &unit, role.clone())
        .await
    {
        Ok(()) => {
            let now = chrono::Utc::now().timestamp();
            persist_governance_event(
                state.as_ref(),
                &ctx,
                &GovernanceEvent::RoleRemoved {
                    user_id: user_id.clone(),
                    unit_id: unit.clone(),
                    role: role.clone(),
                    tenant_id: ctx.tenant_id.clone(),
                    timestamp: now,
                },
            )
            .await;
            audit_membership_action(
                state.as_ref(),
                &ctx,
                "membership_role_remove",
                Some(unit.as_str()),
                json!({
                    "unitId": unit,
                    "userId": user_id.as_str(),
                    "role": role,
                }),
            )
            .await;

            (StatusCode::OK, Json(json!({ "success": true }))).into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "membership_remove_failed",
            &err.to_string(),
        ),
    }
}

async fn list_user_memberships(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<UserRoleListQuery>,
) -> impl IntoResponse {
    let ctx = match require_tenant_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let user_id = match mk_core::types::UserId::new(query.user_id.clone()) {
        Some(user_id) => user_id,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_user_id",
                "Invalid user id",
            );
        }
    };

    match state
        .postgres
        .get_user_roles_scoped(&ctx, &user_id, &ctx.tenant_id)
        .await
    {
        Ok(entries) => {
            let memberships: Vec<_> = entries
                .into_iter()
                .map(|(unit_id, role)| UserScopedRoleResponse { unit_id, role })
                .collect();
            (
                StatusCode::OK,
                Json(json!({
                    "success": true,
                    "userId": user_id.as_str(),
                    "memberships": memberships,
                })),
            )
                .into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "membership_list_failed",
            &err.to_string(),
        ),
    }
}

async fn require_platform_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<TenantContext, axum::response::Response> {
    let ctx = authenticated_tenant_context(state, headers).await?;
    if !ctx.has_known_role(&Role::PlatformAdmin) {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "PlatformAdmin role required",
        ));
    }

    Ok(ctx)
}

/// B2 §10.5 per-route wiring helper.
///
/// Enforces the two-principal authorisation model documented in
/// `docs/security/tenant-provisioning-security.md` §2:
///
/// - **Service token caller** — validated via
///   [`validate_service_token_from_headers`]; the route's required
///   capability is checked with [`require_capability`]. Role checks
///   are intentionally skipped (a service principal has no user role).
/// - **User caller** — the validator returns `Ok(None)` (no bearer, or
///   a plugin token that the service-token validator classifies as a
///   user identity), and we fall through to the existing
///   [`require_platform_admin`] role check.
///
/// Either gate is sufficient: a caller holding a `tenants:provision`
/// service token and a caller holding the `PlatformAdmin` user role
/// both reach the handler body. A caller with neither fails closed.
///
/// Returns [`AuthPrincipal::Service`] on the service-token path and
/// [`AuthPrincipal::User`] on the user path. Handlers that need to
/// audit the action borrow the principal as an [`AuditActor`] via
/// [`AuthPrincipal::as_audit_actor`] — the B2 §11.4 refactor makes
/// the audit helpers accept either kind uniformly.
pub(super) async fn require_platform_admin_or_scope(
    state: &AppState,
    headers: &HeaderMap,
    required_scope: &str,
) -> Result<AuthPrincipal, axum::response::Response> {
    // 1. Service-token path — short-circuits the role check entirely.
    match crate::server::service_token_validator::validate_service_token_from_headers(
        state, headers,
    )
    .await
    {
        Ok(Some(principal)) => {
            // Valid service token — check scope, return the
            // principal so the handler can attribute its audit rows
            // to the service agent (B2 §11.4).
            crate::server::service_token_validator::require_capability(
                Some(&principal),
                required_scope,
            )?;
            Ok(AuthPrincipal::Service(principal))
        }
        Ok(None) => {
            // 2. User path — fall through to the existing role check.
            let ctx = require_platform_admin(state, headers).await?;
            Ok(AuthPrincipal::User(ctx))
        }
        Err(response) => {
            // 3. Token *claimed* to be a service token but failed validation.
            //    Fail closed — do NOT fall through to the role path, because
            //    the caller's stated identity did not authenticate.
            Err(response)
        }
    }
}

async fn require_tenant_admin_context(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<TenantContext, axum::response::Response> {
    let ctx = tenant_scoped_context(state, headers).await?;
    if ctx.has_known_role(&Role::PlatformAdmin) {
        Ok(ctx)
    } else if ctx.has_known_role(&Role::Admin) {
        // TenantAdmin is self-scoped: they MUST NOT target a different tenant.
        if let Some(ref target) = ctx.target_tenant_id
            && target != &ctx.tenant_id
        {
            return Err(error_response(
                StatusCode::FORBIDDEN,
                "forbidden",
                "TenantAdmin cannot target a different tenant",
            ));
        }
        Ok(ctx)
    } else {
        Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Admin or PlatformAdmin role required",
        ))
    }
}

/// B2 §11.4 — unified audit-attribution actor.
///
/// Before B2 §11.4 the audit helpers hard-coded `PrincipalType::User`
/// and pulled `actor_id` from `TenantContext::user_id`. That made it
/// impossible to attribute an audit row to a service-token caller
/// (§10.5), because a service token carries a [`ServicePrincipal`]
/// (agent UUID + tenant + scopes) with no user identity.
///
/// This enum is the single point of branching: each audit helper
/// translates the actor kind into the matching
/// `(PrincipalType, Option<Uuid>, acting_as_tenant_id, details.*)`
/// tuple. Call sites pass either `&TenantContext` or `&ServicePrincipal`
/// directly — the `From` impls below turn them into an `AuditActor`
/// automatically, so the existing `audit_tenant_action(&ctx, ...)`
/// call shape keeps compiling.
#[derive(Debug, Clone, Copy)]
pub(crate) enum AuditActor<'a> {
    User(&'a TenantContext),
    Service(&'a crate::server::service_token_validator::ServicePrincipal),
}

impl<'a> From<&'a TenantContext> for AuditActor<'a> {
    fn from(ctx: &'a TenantContext) -> Self {
        AuditActor::User(ctx)
    }
}

impl<'a> From<&'a crate::server::service_token_validator::ServicePrincipal> for AuditActor<'a> {
    fn from(p: &'a crate::server::service_token_validator::ServicePrincipal) -> Self {
        AuditActor::Service(p)
    }
}

/// Owned counterpart to [`AuditActor`], returned by
/// [`require_platform_admin_or_scope`] so handlers can thread the
/// authenticated principal through the full request lifetime and
/// borrow it as an `AuditActor` at each audit call site without
/// re-authenticating.
#[derive(Debug)]
pub(super) enum AuthPrincipal {
    User(TenantContext),
    Service(crate::server::service_token_validator::ServicePrincipal),
}

impl AuthPrincipal {
    /// Borrow this principal as an [`AuditActor`] for passing to the
    /// `audit_tenant_action*` helpers.
    pub(crate) fn as_audit_actor(&self) -> AuditActor<'_> {
        match self {
            AuthPrincipal::User(ctx) => AuditActor::User(ctx),
            AuthPrincipal::Service(p) => AuditActor::Service(p),
        }
    }

    /// Return a [`TenantContext`] suitable for threading into downstream
    /// tenant-scoped helpers (`get_unit`, `create_unit`, storage calls
    /// that expect a `TenantContext` arg).
    ///
    /// For the `User` variant this is the authenticated user's own
    /// context — unchanged from pre-§11.4 behaviour.
    ///
    /// For the `Service` variant we synthesize a context from the
    /// `ServicePrincipal`:
    ///   - `user_id` ← the agent UUID (as string) — every audit /
    ///     tracking column that expects a `UserId` gets a stable,
    ///     unique identifier for the service token.
    ///   - `agent_id` ← `Some(agent_uuid_string)` — distinguishes
    ///     service-attributed downstream writes from user ones.
    ///   - `roles` ← empty — downstream helpers must not re-check
    ///     role; the scope check in
    ///     [`require_platform_admin_or_scope`] is the authoritative
    ///     authorisation gate for service tokens.
    ///   - `tenant_id` / `target_tenant_id` ← both set to the supplied
    ///     target (the tenant being provisioned). Service tokens do
    ///     not carry an impersonation concept, so `target == actor`.
    ///
    /// The synthesized context is for DOWNSTREAM DATA OPS only. Any
    /// *authorisation* decision that still depends on role must be
    /// redirected through a scope-aware helper before calling this.
    pub(crate) fn to_downstream_tenant_context(
        &self,
        target_tenant_id: mk_core::TenantId,
    ) -> mk_core::types::TenantContext {
        match self {
            AuthPrincipal::User(ctx) => mk_core::types::TenantContext {
                tenant_id: target_tenant_id.clone(),
                user_id: ctx.user_id.clone(),
                roles: ctx.roles.clone(),
                target_tenant_id: Some(target_tenant_id),
                agent_id: ctx.agent_id.clone(),
            },
            AuthPrincipal::Service(p) => {
                let agent_str = p.agent_id.to_string();
                mk_core::types::TenantContext {
                    tenant_id: target_tenant_id.clone(),
                    // `UserId::new` only rejects empty / >100 chars —
                    // a UUID string always satisfies both.
                    user_id: mk_core::types::UserId::new(agent_str.clone())
                        .expect("agent UUID string fits UserId bounds"),
                    roles: Vec::new(),
                    target_tenant_id: Some(target_tenant_id),
                    agent_id: Some(agent_str),
                }
            }
        }
    }

    /// Return a [`TenantContext`] suitable for the **admin pool** write
    /// path — `with_admin_context`, `persist_governance_event`, and the
    /// `audit_tenant_action*` family when the call site already holds a
    /// `&TenantContext` from pre-§10.5 days.
    ///
    /// Differs from [`to_downstream_tenant_context`] in *which* tenant
    /// the synthesized context binds:
    ///
    ///   - For `User`: returns the user's own ctx (cloned). Behaviour
    ///     identical to pre-§11.4.
    ///   - For `Service`: binds `tenant_id = service-token's OWN
    ///     tenant`, `target_tenant_id = None`, `user_id = agent_uuid`,
    ///     `agent_id = Some(agent_uuid)`, empty roles. The caller's
    ///     audit row therefore links *cause* (which agent in which
    ///     tenant) rather than *effect* (the targeted tenant — which
    ///     is already covered by the action's `target_id`).
    ///
    /// This is the same shape `provision_tenant` synthesizes inline
    /// today; extracted here so grant/revoke connection (and any
    /// future scope-gated mutation) can stop duplicating the match.
    pub(crate) fn to_admin_pool_context(&self) -> mk_core::types::TenantContext {
        match self {
            AuthPrincipal::User(ctx) => ctx.clone(),
            AuthPrincipal::Service(sp) => {
                let agent_str = sp.agent_id.to_string();
                mk_core::types::TenantContext {
                    tenant_id: mk_core::TenantId::new(sp.tenant_id.to_string())
                        .expect("service principal tenant_id is a valid uuid"),
                    user_id: mk_core::types::UserId::new(agent_str.clone())
                        .expect("agent UUID string fits UserId bounds"),
                    roles: Vec::new(),
                    target_tenant_id: None,
                    agent_id: Some(agent_str),
                }
            }
        }
    }
}

/// B2 §11.4 — resolve an [`AuditActor`] to the audit-row fields.
///
/// Returns `(actor_type, actor_id, acting_as_tenant_id, details_json)`.
/// The first three bind directly into `log_audit_with_extensions`; the
/// fourth is merged into the existing per-action `details` payload so
/// downstream audit-log readers see the same `actorTenantId` /
/// `selectedTargetTenantId` keys regardless of actor kind (service
/// principals simply leave `selectedTargetTenantId` as `null`).
fn audit_actor_fields(
    actor: AuditActor<'_>,
) -> (
    PrincipalType,
    Option<uuid::Uuid>,
    Option<uuid::Uuid>,
    serde_json::Value,
) {
    match actor {
        AuditActor::User(ctx) => {
            let actor_id = uuid::Uuid::parse_str(ctx.user_id.as_str()).ok();
            // #44.d §2.5 — `acting_as_tenant_id` is the tenant the action
            // operated against: the impersonated tenant when set,
            // otherwise the actor's own membership. Drives
            // `/govern/audit?tenant=<slug>` filtering.
            let acting_as = uuid::Uuid::parse_str(
                ctx.target_tenant_id
                    .as_ref()
                    .map_or(ctx.tenant_id.as_str(), mk_core::TenantId::as_str),
            )
            .ok();
            let envelope = serde_json::json!({
                "actorTenantId": ctx.tenant_id.as_str(),
                "selectedTargetTenantId": ctx.target_tenant_id.as_ref().map(mk_core::TenantId::as_str),
                "actorKind": "user",
            });
            (PrincipalType::User, actor_id, acting_as, envelope)
        }
        AuditActor::Service(principal) => {
            // Service tokens are persisted as `agents` rows with
            // `agent_type='service'` (see §10.2 mint_handler), so
            // `PrincipalType::Agent` is the right type and
            // `principal.agent_id` is the FK into `agents`.
            let envelope = serde_json::json!({
                "actorTenantId": principal.tenant_id.to_string(),
                "selectedTargetTenantId": serde_json::Value::Null,
                "actorKind": "service",
                "agentId": principal.agent_id.to_string(),
            });
            (
                PrincipalType::Agent,
                Some(principal.agent_id),
                // For a service caller, `acting_as` is the service
                // token's own tenant — consistent with how other
                // agent-attributed audit rows filter today. Routes that
                // provision a *different* target tenant carry the
                // target slug in `target_id` and the full manifest hash
                // in the §11.1 extensions, so the audit trail still
                // links cause (service token) to effect (target tenant).
                Some(principal.tenant_id),
                envelope,
            )
        }
    }
}

async fn audit_tenant_action<'a>(
    state: &AppState,
    actor: impl Into<AuditActor<'a>>,
    action: &str,
    target_id: Option<&str>,
    details: serde_json::Value,
) {
    let Some(storage) = &state.governance_storage else {
        return;
    };

    let (actor_type, actor_id, acting_as, mut envelope) = audit_actor_fields(actor.into());
    // Preserve the pre-§11.4 payload shape: the per-action `details`
    // are nested under a `details` key so existing audit consumers
    // that look for `details.<action_specific_field>` keep working.
    envelope["details"] = details;

    let _ = storage
        .log_audit(
            action,
            None,
            Some("tenant"),
            target_id,
            actor_type,
            actor_id,
            None,
            envelope,
            acting_as,
        )
        .await;
}

/// Provision-path variant of [`audit_tenant_action`] that records the B2
/// §11.1 request-context columns (`via`, `client_version`, `manifest_hash`,
/// `generation`, `dry_run`) alongside the primary audit payload. Used only
/// by the provisioning handlers; every other call site keeps using the
/// zero-ceremony [`audit_tenant_action`] helper.
async fn audit_tenant_action_ext<'a>(
    state: &AppState,
    actor: impl Into<AuditActor<'a>>,
    action: &str,
    target_id: Option<&str>,
    details: serde_json::Value,
    ext: AuditExtensions,
) {
    let Some(storage) = &state.governance_storage else {
        return;
    };

    let (actor_type, actor_id, acting_as, mut envelope) = audit_actor_fields(actor.into());
    envelope["details"] = details;

    let _ = storage
        .log_audit_with_extensions(
            action,
            None,
            Some("tenant"),
            target_id,
            actor_type,
            actor_id,
            None,
            envelope,
            acting_as,
            ext,
        )
        .await;
}

/// Build the base [`AuditExtensions`] for a provision call from the request
/// headers. `via` is taken from the middleware-injected
/// [`crate::server::request_context::RequestContext`] (present as a request
/// extension) — but since this helper receives the raw `HeaderMap`, we
/// re-parse it here. Cheap: header lookup + a handful of string compares.
/// `client_version` is extracted from the standard `User-Agent` header,
/// trimmed and capped at 256 chars to keep audit rows bounded.
fn provision_audit_ext_base(headers: &HeaderMap) -> AuditExtensions {
    let rc = super::request_context::RequestContext::from_headers(headers);
    let client_version = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(256).collect::<String>());
    AuditExtensions {
        via: Some(rc.client_kind.to_string()),
        client_version,
        manifest_hash: None,
        generation: None,
        dry_run: None,
    }
}

async fn audit_hierarchy_action(
    state: &AppState,
    ctx: &TenantContext,
    action: &str,
    target_id: Option<&str>,
    details: serde_json::Value,
) {
    let Some(storage) = &state.governance_storage else {
        return;
    };

    let actor_id = uuid::Uuid::parse_str(ctx.user_id.as_str()).ok();
    let acting_as = uuid::Uuid::parse_str(
        ctx.target_tenant_id
            .as_ref()
            .map_or(ctx.tenant_id.as_str(), mk_core::TenantId::as_str),
    )
    .ok();
    let _ = storage
        .log_audit(
            action,
            None,
            Some("organizational_unit"),
            target_id,
            PrincipalType::User,
            actor_id,
            None,
            serde_json::json!({
                "actorTenantId": ctx.tenant_id.as_str(),
                "selectedTargetTenantId": ctx.target_tenant_id.as_ref().map(mk_core::TenantId::as_str),
                "details": details,
            }),
            acting_as,
        )
        .await;
}

async fn audit_membership_action(
    state: &AppState,
    ctx: &TenantContext,
    action: &str,
    target_id: Option<&str>,
    details: serde_json::Value,
) {
    let Some(storage) = &state.governance_storage else {
        return;
    };

    let actor_id = uuid::Uuid::parse_str(ctx.user_id.as_str()).ok();
    let acting_as = uuid::Uuid::parse_str(
        ctx.target_tenant_id
            .as_ref()
            .map_or(ctx.tenant_id.as_str(), mk_core::TenantId::as_str),
    )
    .ok();
    let _ = storage
        .log_audit(
            action,
            None,
            Some("membership"),
            target_id,
            PrincipalType::User,
            actor_id,
            None,
            serde_json::json!({
                "actorTenantId": ctx.tenant_id.as_str(),
                "selectedTargetTenantId": ctx.target_tenant_id.as_ref().map(mk_core::TenantId::as_str),
                "details": details,
            }),
            acting_as,
        )
        .await;
}

async fn persist_governance_event(
    state: &AppState,
    ctx: &mk_core::types::TenantContext,
    event: &GovernanceEvent,
) {
    // Tenant provisioning / admin-surface events: write via the admin pool.
    // `with_admin_context` records the admin's action in
    // `governance_audit_log` atomically with the `governance_events` row,
    // giving us a unified audit trail without requiring `app.tenant_id`
    // to match (the event's tenant_id may be the *managed* tenant, not
    // the admin's own).
    //
    // Clone so the boxed future owns the event and satisfies the `'static`
    // bound required by `with_admin_context`'s HRTB.
    let event_owned = event.clone();
    let _ = state
        .postgres
        .with_admin_context(ctx, "log_governance_event", move |tx| {
            Box::pin(async move {
                storage::postgres::PostgresBackend::log_event(tx, &event_owned).await
            })
        })
        .await;
    let _ = state
        .postgres
        .persist_event(PersistentEvent::new(event.clone()))
        .await;
}

// ---------------------------------------------------------------------------
// Provider configuration API (tenant-specific LLM/embedding providers)
// ---------------------------------------------------------------------------

/// Request body for setting an LLM or embedding provider.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetProviderRequest {
    pub provider: String,
    pub model: String,
    pub api_key: Option<String>,
    pub google_project_id: Option<String>,
    pub google_location: Option<String>,
    pub bedrock_region: Option<String>,
}

/// Provider info returned in the GET response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub configured: bool,
}

/// Response body for `GET /admin/tenants/{tenant}/providers`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TenantProvidersResponse {
    pub llm: ProviderInfo,
    pub embedding: ProviderInfo,
    pub source: String,
}

/// Status of a single provider connectivity test.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatusInfo {
    pub status: String,
    pub latency_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimension: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Response body for `GET /admin/tenants/{tenant}/providers/status`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TenantProviderStatusResponse {
    pub llm: ProviderStatusInfo,
    pub embedding: ProviderStatusInfo,
}

/// Helper to extract a provider info block from tenant config.
fn extract_provider_info(
    config: &Option<TenantConfigDocument>,
    provider_key: &str,
    model_key: &str,
    api_key_name: &str,
) -> ProviderInfo {
    let (provider, model, configured) = match config {
        Some(doc) => {
            let provider = doc
                .fields
                .get(provider_key)
                .and_then(|f| f.value.as_str().map(String::from));
            let model = doc
                .fields
                .get(model_key)
                .and_then(|f| f.value.as_str().map(String::from));
            let has_secret = doc.secret_references.contains_key(api_key_name);
            let configured =
                provider.is_some() && (has_secret || provider.as_deref() != Some("openai"));
            (provider, model, configured)
        }
        None => (None, None, false),
    };
    ProviderInfo {
        provider,
        model,
        configured,
    }
}

/// `GET /api/v1/admin/tenants/{tenant}/providers`
///
/// Returns the current LLM and embedding provider configuration for a tenant.
/// Does NOT return API keys -- only indicates whether they are configured.
#[tracing::instrument(skip_all, fields(tenant))]
async fn get_tenant_providers(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
) -> impl IntoResponse {
    if let Err(response) = require_platform_admin(&state, &headers).await {
        return response;
    }

    let tenant_record =
        match resolve_tenant_record_or_404(&state, &tenant, "provider_config_get_failed").await {
            Ok(record) => record,
            Err(response) => return response,
        };

    let config = state
        .tenant_config_provider
        .get_config(&tenant_record.id)
        .await
        .ok()
        .flatten();

    let has_tenant_config = config.as_ref().is_some_and(|c| {
        c.fields.contains_key(config_keys::LLM_PROVIDER)
            || c.fields.contains_key(config_keys::EMBEDDING_PROVIDER)
    });

    let llm = extract_provider_info(
        &config,
        config_keys::LLM_PROVIDER,
        config_keys::LLM_MODEL,
        config_keys::LLM_API_KEY,
    );
    let embedding = extract_provider_info(
        &config,
        config_keys::EMBEDDING_PROVIDER,
        config_keys::EMBEDDING_MODEL,
        config_keys::EMBEDDING_API_KEY,
    );

    let source = if has_tenant_config {
        "tenant"
    } else {
        OWNERSHIP_PLATFORM
    };

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "llm": llm,
            "embedding": embedding,
            "source": source,
        })),
    )
        .into_response()
}

/// `PUT /api/v1/admin/tenants/{tenant}/providers/llm`
///
/// Set the LLM provider configuration for a tenant.
#[tracing::instrument(skip_all, fields(tenant))]
async fn set_tenant_llm_provider(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
    Json(req): Json<SetProviderRequest>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let tenant_record =
        match resolve_tenant_record_or_404(&state, &tenant, "provider_llm_set_failed").await {
            Ok(record) => record,
            Err(response) => return response,
        };

    // Build config fields
    let mut fields = BTreeMap::new();
    fields.insert(
        config_keys::LLM_PROVIDER.to_string(),
        TenantConfigField {
            ownership: TenantConfigOwnership::Platform,
            value: serde_json::json!(req.provider),
        },
    );
    fields.insert(
        config_keys::LLM_MODEL.to_string(),
        TenantConfigField {
            ownership: TenantConfigOwnership::Platform,
            value: serde_json::json!(req.model),
        },
    );
    if let Some(project_id) = &req.google_project_id {
        fields.insert(
            config_keys::LLM_GOOGLE_PROJECT_ID.to_string(),
            TenantConfigField {
                ownership: TenantConfigOwnership::Platform,
                value: serde_json::json!(project_id),
            },
        );
    }
    if let Some(location) = &req.google_location {
        fields.insert(
            config_keys::LLM_GOOGLE_LOCATION.to_string(),
            TenantConfigField {
                ownership: TenantConfigOwnership::Platform,
                value: serde_json::json!(location),
            },
        );
    }
    if let Some(region) = &req.bedrock_region {
        fields.insert(
            config_keys::LLM_BEDROCK_REGION.to_string(),
            TenantConfigField {
                ownership: TenantConfigOwnership::Platform,
                value: serde_json::json!(region),
            },
        );
    }

    let document = TenantConfigDocument {
        tenant_id: tenant_record.id.clone(),
        fields,
        secret_references: BTreeMap::new(),
    };

    if let Err(err) = state.tenant_config_provider.upsert_config(document).await {
        return map_tenant_config_provider_error("provider_llm_set", err);
    }

    // Store API key secret if provided
    if let Some(api_key) = &req.api_key {
        let secret = TenantSecretEntry {
            logical_name: config_keys::LLM_API_KEY.to_string(),
            ownership: TenantConfigOwnership::Platform,
            secret_value: mk_core::secret::SecretBytes::from(api_key.clone()),
        };
        if let Err(err) = state
            .tenant_config_provider
            .set_secret_entry(&tenant_record.id, secret)
            .await
        {
            return map_tenant_config_provider_error("provider_llm_secret_set", err);
        }
    }

    state.provider_registry.invalidate_tenant(&tenant_record.id);

    audit_tenant_action(
        state.as_ref(),
        &ctx,
        "provider_llm_set",
        Some(tenant_record.id.as_str()),
        json!({
            "tenantId": tenant_record.id.as_str(),
            "provider": req.provider,
            "model": req.model,
        }),
    )
    .await;

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "provider": req.provider,
            "model": req.model,
        })),
    )
        .into_response()
}

/// `PUT /api/v1/admin/tenants/{tenant}/providers/embedding`
///
/// Set the embedding provider configuration for a tenant.
#[tracing::instrument(skip_all, fields(tenant))]
async fn set_tenant_embedding_provider(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
    Json(req): Json<SetProviderRequest>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let tenant_record = match resolve_tenant_record_or_404(
        &state,
        &tenant,
        "provider_embedding_set_failed",
    )
    .await
    {
        Ok(record) => record,
        Err(response) => return response,
    };

    // Check if changing embedding model when vectors already exist
    let existing_config = state
        .tenant_config_provider
        .get_config(&tenant_record.id)
        .await
        .ok()
        .flatten();
    let existing_model = existing_config.as_ref().and_then(|c| {
        c.fields
            .get(config_keys::EMBEDDING_MODEL)
            .and_then(|f| f.value.as_str().map(String::from))
    });
    let model_changed = existing_model.as_ref().is_some_and(|m| m != &req.model);

    // Build config fields
    let mut fields = BTreeMap::new();
    fields.insert(
        config_keys::EMBEDDING_PROVIDER.to_string(),
        TenantConfigField {
            ownership: TenantConfigOwnership::Platform,
            value: serde_json::json!(req.provider),
        },
    );
    fields.insert(
        config_keys::EMBEDDING_MODEL.to_string(),
        TenantConfigField {
            ownership: TenantConfigOwnership::Platform,
            value: serde_json::json!(req.model),
        },
    );
    if let Some(project_id) = &req.google_project_id {
        fields.insert(
            config_keys::EMBEDDING_GOOGLE_PROJECT_ID.to_string(),
            TenantConfigField {
                ownership: TenantConfigOwnership::Platform,
                value: serde_json::json!(project_id),
            },
        );
    }
    if let Some(location) = &req.google_location {
        fields.insert(
            config_keys::EMBEDDING_GOOGLE_LOCATION.to_string(),
            TenantConfigField {
                ownership: TenantConfigOwnership::Platform,
                value: serde_json::json!(location),
            },
        );
    }
    if let Some(region) = &req.bedrock_region {
        fields.insert(
            config_keys::EMBEDDING_BEDROCK_REGION.to_string(),
            TenantConfigField {
                ownership: TenantConfigOwnership::Platform,
                value: serde_json::json!(region),
            },
        );
    }

    let document = TenantConfigDocument {
        tenant_id: tenant_record.id.clone(),
        fields,
        secret_references: BTreeMap::new(),
    };

    if let Err(err) = state.tenant_config_provider.upsert_config(document).await {
        return map_tenant_config_provider_error("provider_embedding_set", err);
    }

    // Store API key secret if provided
    if let Some(api_key) = &req.api_key {
        let secret = TenantSecretEntry {
            logical_name: config_keys::EMBEDDING_API_KEY.to_string(),
            ownership: TenantConfigOwnership::Platform,
            secret_value: mk_core::secret::SecretBytes::from(api_key.clone()),
        };
        if let Err(err) = state
            .tenant_config_provider
            .set_secret_entry(&tenant_record.id, secret)
            .await
        {
            return map_tenant_config_provider_error("provider_embedding_secret_set", err);
        }
    }

    state.provider_registry.invalidate_tenant(&tenant_record.id);

    audit_tenant_action(
        state.as_ref(),
        &ctx,
        "provider_embedding_set",
        Some(tenant_record.id.as_str()),
        json!({
            "tenantId": tenant_record.id.as_str(),
            "provider": req.provider,
            "model": req.model,
        }),
    )
    .await;

    let mut response = json!({
        "success": true,
        "provider": req.provider,
        "model": req.model,
    });

    if model_changed {
        response["warning"] = serde_json::json!(
            "Embedding model changed. Existing vectors may have different dimensions and should be re-indexed."
        );
    }

    (StatusCode::OK, Json(response)).into_response()
}

/// `DELETE /api/v1/admin/tenants/{tenant}/providers/llm`
///
/// Remove the tenant LLM provider override, falling back to platform default.
#[tracing::instrument(skip_all, fields(tenant))]
async fn delete_tenant_llm_provider(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let tenant_record =
        match resolve_tenant_record_or_404(&state, &tenant, "provider_llm_delete_failed").await {
            Ok(record) => record,
            Err(response) => return response,
        };

    // Clear LLM-related config fields by upserting empty values
    let fields_to_clear = [
        config_keys::LLM_PROVIDER,
        config_keys::LLM_MODEL,
        config_keys::LLM_GOOGLE_PROJECT_ID,
        config_keys::LLM_GOOGLE_LOCATION,
        config_keys::LLM_BEDROCK_REGION,
    ];

    let mut fields = BTreeMap::new();
    for key in &fields_to_clear {
        fields.insert(
            (*key).to_string(),
            TenantConfigField {
                ownership: TenantConfigOwnership::Platform,
                value: serde_json::Value::Null,
            },
        );
    }
    let document = TenantConfigDocument {
        tenant_id: tenant_record.id.clone(),
        fields,
        secret_references: BTreeMap::new(),
    };
    let _ = state.tenant_config_provider.upsert_config(document).await;

    // Remove the API key secret
    let _ = state
        .tenant_config_provider
        .delete_secret_entry(&tenant_record.id, config_keys::LLM_API_KEY)
        .await;

    state.provider_registry.invalidate_tenant(&tenant_record.id);

    audit_tenant_action(
        state.as_ref(),
        &ctx,
        "provider_llm_delete",
        Some(tenant_record.id.as_str()),
        json!({ "tenantId": tenant_record.id.as_str() }),
    )
    .await;

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "message": "LLM provider override removed; tenant will use platform default",
        })),
    )
        .into_response()
}

/// `DELETE /api/v1/admin/tenants/{tenant}/providers/embedding`
///
/// Remove the tenant embedding provider override, falling back to platform default.
#[tracing::instrument(skip_all, fields(tenant))]
async fn delete_tenant_embedding_provider(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let tenant_record =
        match resolve_tenant_record_or_404(&state, &tenant, "provider_embedding_delete_failed")
            .await
        {
            Ok(record) => record,
            Err(response) => return response,
        };

    let fields_to_clear = [
        config_keys::EMBEDDING_PROVIDER,
        config_keys::EMBEDDING_MODEL,
        config_keys::EMBEDDING_GOOGLE_PROJECT_ID,
        config_keys::EMBEDDING_GOOGLE_LOCATION,
        config_keys::EMBEDDING_BEDROCK_REGION,
    ];

    let mut fields = BTreeMap::new();
    for key in &fields_to_clear {
        fields.insert(
            (*key).to_string(),
            TenantConfigField {
                ownership: TenantConfigOwnership::Platform,
                value: serde_json::Value::Null,
            },
        );
    }
    let document = TenantConfigDocument {
        tenant_id: tenant_record.id.clone(),
        fields,
        secret_references: BTreeMap::new(),
    };
    let _ = state.tenant_config_provider.upsert_config(document).await;

    let _ = state
        .tenant_config_provider
        .delete_secret_entry(&tenant_record.id, config_keys::EMBEDDING_API_KEY)
        .await;

    state.provider_registry.invalidate_tenant(&tenant_record.id);

    audit_tenant_action(
        state.as_ref(),
        &ctx,
        "provider_embedding_delete",
        Some(tenant_record.id.as_str()),
        json!({ "tenantId": tenant_record.id.as_str() }),
    )
    .await;

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "message": "Embedding provider override removed; tenant will use platform default",
        })),
    )
        .into_response()
}

/// `GET /api/v1/admin/tenants/{tenant}/providers/status`
///
/// Test provider connectivity by attempting a simple operation with both the
/// LLM and embedding services.
#[tracing::instrument(skip_all, fields(tenant))]
async fn test_tenant_provider_connectivity(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
) -> impl IntoResponse {
    if let Err(response) = require_platform_admin(&state, &headers).await {
        return response;
    }

    let tenant_record =
        match resolve_tenant_record_or_404(&state, &tenant, "provider_status_failed").await {
            Ok(record) => record,
            Err(response) => return response,
        };

    // Test LLM service
    let llm_status = {
        let start = std::time::Instant::now();
        match state
            .provider_registry
            .get_llm_service(&tenant_record.id, state.tenant_config_provider.as_ref())
            .await
        {
            Some(llm) => match llm.generate("Say hello in one word.").await {
                Ok(_) => ProviderStatusInfo {
                    status: "ok".to_string(),
                    latency_ms: Some(start.elapsed().as_millis()),
                    dimension: None,
                    error: None,
                },
                Err(e) => ProviderStatusInfo {
                    status: "error".to_string(),
                    latency_ms: Some(start.elapsed().as_millis()),
                    dimension: None,
                    error: Some(format!("{e}")),
                },
            },
            None => ProviderStatusInfo {
                status: "not_configured".to_string(),
                latency_ms: None,
                dimension: None,
                error: Some("No LLM service available for this tenant".to_string()),
            },
        }
    };

    // Test embedding service
    let embedding_status = {
        let start = std::time::Instant::now();
        match state
            .provider_registry
            .get_embedding_service(&tenant_record.id, state.tenant_config_provider.as_ref())
            .await
        {
            Some(emb) => match emb.embed("test embedding connectivity").await {
                Ok(vector) => ProviderStatusInfo {
                    status: "ok".to_string(),
                    latency_ms: Some(start.elapsed().as_millis()),
                    dimension: Some(vector.len()),
                    error: None,
                },
                Err(e) => ProviderStatusInfo {
                    status: "error".to_string(),
                    latency_ms: Some(start.elapsed().as_millis()),
                    dimension: None,
                    error: Some(format!("{e}")),
                },
            },
            None => ProviderStatusInfo {
                status: "not_configured".to_string(),
                latency_ms: None,
                dimension: None,
                error: Some("No embedding service available for this tenant".to_string()),
            },
        }
    };

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "llm": llm_status,
            "embedding": embedding_status,
        })),
    )
        .into_response()
}

fn error_response(status: StatusCode, error: &str, message: &str) -> axum::response::Response {
    (
        status,
        Json(json!({
            "error": error,
            "message": message,
        })),
    )
        .into_response()
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 2.3 — Permission inspection
// ─────────────────────────────────────────────────────────────────────────────

/// `GET /api/v1/admin/permissions/matrix`
///
/// Returns the role-to-permission matrix derived from the active Cedar RBAC policy.
/// Requires PlatformAdmin or Admin role.
async fn get_permission_matrix(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(response) = require_platform_or_admin(&state, &headers).await {
        return response;
    }

    let matrix = role_permission_matrix();
    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "matrix": matrix,
            "actions": ALL_ACTIONS,
        })),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
struct EffectivePermissionsQuery {
    user_id: String,
    resource: Option<String>,
    actions: Option<String>,
    role: Option<RoleIdentifier>,
}

/// `GET /api/v1/admin/permissions/effective?role=<role>`
///
/// Returns the set of permitted actions for the given role in the active policy bundle.
/// Requires PlatformAdmin or Admin role.
async fn get_effective_permissions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<EffectivePermissionsQuery>,
) -> impl IntoResponse {
    let admin_ctx = match require_platform_or_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let user_id = match mk_core::types::UserId::new(query.user_id.clone()) {
        Some(user_id) => user_id,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_user_id",
                "Invalid user id",
            );
        }
    };

    let resource = query
        .resource
        .unwrap_or_else(|| format!("Aeterna::Tenant::\"{}\"", admin_ctx.tenant_id.as_str()));

    let actions: Vec<String> = query
        .actions
        .as_deref()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_else(|| {
            ALL_ACTIONS
                .iter()
                .map(|value| (*value).to_string())
                .collect()
        });

    let mut principal_ctx = TenantContext::new(admin_ctx.tenant_id.clone(), user_id.clone());

    let roles = if let Some(role) = query.role.clone() {
        principal_ctx.roles = vec![role.clone()];
        vec![role]
    } else {
        match state.auth_service.get_user_roles(&principal_ctx).await {
            Ok(roles) => roles,
            Err(err) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "permission_inspection_failed",
                    &err.to_string(),
                );
            }
        }
    };

    let mut granted = Vec::new();
    let mut denied = Vec::new();

    for action in &actions {
        let allowed = if roles.is_empty() {
            match state
                .auth_service
                .check_permission(&principal_ctx, action, &resource)
                .await
            {
                Ok(value) => value,
                Err(err) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "permission_inspection_failed",
                        &err.to_string(),
                    );
                }
            }
        } else {
            let mut allowed = false;
            for role in &roles {
                let mut scoped_ctx = principal_ctx.clone();
                scoped_ctx.roles = vec![role.clone()];
                match state
                    .auth_service
                    .check_permission(&scoped_ctx, action, &resource)
                    .await
                {
                    Ok(true) => {
                        allowed = true;
                        break;
                    }
                    Ok(false) => {}
                    Err(err) => {
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "permission_inspection_failed",
                            &err.to_string(),
                        );
                    }
                }
            }
            allowed
        };

        if allowed {
            granted.push(action.clone());
        } else {
            denied.push(action.clone());
        }
    }

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "userId": user_id.as_str(),
            "resource": resource,
            "roles": roles,
            "granted": granted,
            "denied": denied,
        })),
    )
        .into_response()
}

/// Require PlatformAdmin **or** Admin (tenant-scoped) role.
async fn require_platform_or_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<TenantContext, axum::response::Response> {
    let ctx = match super::tenant_scoped_context(state, headers).await {
        Ok(ctx) => ctx,
        Err(response) => return Err(response),
    };
    if ctx.has_known_role(&Role::PlatformAdmin) || ctx.has_known_role(&Role::Admin) {
        Ok(ctx)
    } else {
        Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Admin or PlatformAdmin role required",
        ))
    }
}

// ---------------------------------------------------------------------------
// Git provider connection handlers (task 3.4)
// ---------------------------------------------------------------------------

fn map_connection_error(
    operation: &str,
    err: GitProviderConnectionError,
) -> axum::response::Response {
    match &err {
        GitProviderConnectionError::NotFound(id) => error_response(
            StatusCode::NOT_FOUND,
            &format!("git_provider_connection_{operation}_not_found"),
            &format!("Git provider connection '{id}' not found"),
        ),
        GitProviderConnectionError::Validation(msg) => error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!("git_provider_connection_{operation}_invalid"),
            msg,
        ),
    }
}

async fn create_git_provider_connection(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateGitProviderConnectionRequest>,
) -> impl IntoResponse {
    let ctx = match require_platform_admin(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let now_ts = chrono::Utc::now();
    let connection = GitProviderConnection {
        id: req.id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        name: req.name,
        provider_kind: req.provider_kind,
        app_id: req.app_id,
        installation_id: req.installation_id,
        pem_secret_ref: req.pem_secret_ref,
        webhook_secret_ref: req.webhook_secret_ref,
        allowed_tenant_ids: Vec::new(),
        created_at: now_ts,
        updated_at: now_ts,
    };

    match state
        .git_provider_connection_registry
        .create_connection(connection)
        .await
    {
        Ok(created) => {
            let event = GovernanceEvent::GitProviderConnectionCreated {
                connection_id: created.id.clone(),
                tenant_id: ctx.tenant_id.clone(),
                timestamp: now_ts.timestamp(),
            };
            persist_governance_event(state.as_ref(), &ctx, &event).await;
            audit_tenant_action(
                state.as_ref(),
                &ctx,
                "git_provider_connection_create",
                Some(created.id.as_str()),
                json!({ "connectionId": created.id, "name": created.name }),
            )
            .await;
            (
                StatusCode::CREATED,
                Json(json!({ "success": true, "connection": created.redacted() })),
            )
                .into_response()
        }
        Err(err) => map_connection_error("create", err),
    }
}

async fn list_git_provider_connections(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(response) = require_platform_admin(&state, &headers).await {
        return response;
    }

    match state
        .git_provider_connection_registry
        .list_connections()
        .await
    {
        Ok(connections) => {
            let redacted: Vec<_> = connections
                .iter()
                .map(mk_core::types::GitProviderConnection::redacted)
                .collect();
            (
                StatusCode::OK,
                Json(json!({ "success": true, "connections": redacted })),
            )
                .into_response()
        }
        Err(err) => map_connection_error("list", err),
    }
}

async fn show_git_provider_connection(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(connection_id): Path<String>,
) -> impl IntoResponse {
    if let Err(response) = require_platform_admin(&state, &headers).await {
        return response;
    }

    match state
        .git_provider_connection_registry
        .get_connection(&connection_id)
        .await
    {
        Ok(Some(conn)) => (
            StatusCode::OK,
            Json(json!({ "success": true, "connection": conn.redacted() })),
        )
            .into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "git_provider_connection_not_found",
            &format!("Git provider connection '{connection_id}' not found"),
        ),
        Err(err) => map_connection_error("show", err),
    }
}

async fn grant_git_provider_connection_to_tenant(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((connection_id, tenant)): Path<(String, String)>,
) -> impl IntoResponse {
    // B2 §10.5 — accepts PA user OR service token with
    // `connections:manage`. Mutation: must thread the authenticated
    // principal so audit rows attribute to the correct actor kind
    // (user vs agent) and so the admin-pool write path
    // (`persist_governance_event`) gets a synthesized ctx for service
    // callers — same pattern as `provision_tenant`.
    let principal =
        match require_platform_admin_or_scope(&state, &headers, "connections:manage").await {
            Ok(p) => p,
            Err(response) => return response,
        };
    let ctx = principal.to_admin_pool_context();

    let tenant_record =
        match resolve_tenant_record_or_404(&state, &tenant, "connection_grant_failed").await {
            Ok(record) => record,
            Err(response) => return response,
        };

    match state
        .git_provider_connection_registry
        .grant_tenant_visibility(&connection_id, &tenant_record.id)
        .await
    {
        Ok(()) => {
            let now = chrono::Utc::now().timestamp();
            let event = GovernanceEvent::GitProviderConnectionTenantGranted {
                connection_id: connection_id.clone(),
                tenant_id: tenant_record.id.clone(),
                timestamp: now,
            };
            persist_governance_event(state.as_ref(), &ctx, &event).await;
            audit_tenant_action(
                state.as_ref(),
                principal.as_audit_actor(),
                "git_provider_connection_grant",
                Some(connection_id.as_str()),
                json!({
                    "connectionId": connection_id,
                    "tenantId": tenant_record.id.as_str(),
                }),
            )
            .await;
            (
                StatusCode::OK,
                Json(json!({
                    "success": true,
                    "connectionId": connection_id,
                    "tenantId": tenant_record.id.as_str(),
                })),
            )
                .into_response()
        }
        Err(err) => map_connection_error("grant", err),
    }
}

async fn revoke_git_provider_connection_from_tenant(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((connection_id, tenant)): Path<(String, String)>,
) -> impl IntoResponse {
    // B2 §10.5 — see `grant_git_provider_connection_to_tenant`.
    let principal =
        match require_platform_admin_or_scope(&state, &headers, "connections:manage").await {
            Ok(p) => p,
            Err(response) => return response,
        };
    let ctx = principal.to_admin_pool_context();

    let tenant_record =
        match resolve_tenant_record_or_404(&state, &tenant, "connection_revoke_failed").await {
            Ok(record) => record,
            Err(response) => return response,
        };

    match state
        .git_provider_connection_registry
        .revoke_tenant_visibility(&connection_id, &tenant_record.id)
        .await
    {
        Ok(()) => {
            let now = chrono::Utc::now().timestamp();
            let event = GovernanceEvent::GitProviderConnectionTenantRevoked {
                connection_id: connection_id.clone(),
                tenant_id: tenant_record.id.clone(),
                timestamp: now,
            };
            persist_governance_event(state.as_ref(), &ctx, &event).await;
            audit_tenant_action(
                state.as_ref(),
                principal.as_audit_actor(),
                "git_provider_connection_revoke",
                Some(connection_id.as_str()),
                json!({
                    "connectionId": connection_id,
                    "tenantId": tenant_record.id.as_str(),
                }),
            )
            .await;
            (
                StatusCode::OK,
                Json(json!({
                    "success": true,
                    "connectionId": connection_id,
                    "tenantId": tenant_record.id.as_str(),
                })),
            )
                .into_response()
        }
        Err(err) => map_connection_error("revoke", err),
    }
}

async fn list_tenant_git_provider_connections(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tenant): Path<String>,
) -> impl IntoResponse {
    // Both PlatformAdmin (full list) and TenantAdmin (own-tenant view) are allowed.
    let ctx = match require_tenant_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let tenant_record =
        match resolve_tenant_record_or_404(&state, &tenant, "connection_list_failed").await {
            Ok(record) => record,
            Err(response) => return response,
        };

    // TenantAdmin can only list for their own tenant.
    if !ctx.has_known_role(&Role::PlatformAdmin) && tenant_record.id != ctx.tenant_id {
        return error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "TenantAdmin can only list connections for their own tenant",
        );
    }

    match state
        .git_provider_connection_registry
        .list_connections_for_tenant(&tenant_record.id)
        .await
    {
        Ok(connections) => {
            let redacted: Vec<_> = connections
                .iter()
                .map(mk_core::types::GitProviderConnection::redacted)
                .collect();
            (
                StatusCode::OK,
                Json(json!({ "success": true, "connections": redacted })),
            )
                .into_response()
        }
        Err(err) => map_connection_error("list_for_tenant", err),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 10.1–10.5: Tenant Provisioning Manifest
// ─────────────────────────────────────────────────────────────────────────────

/// Versioned manifest schema.  `apiVersion` must be `"aeterna.io/v1"` and
/// `kind` must be `"TenantManifest"`.
///
/// ### `metadata.generation`
///
/// Optional monotonic revision counter owned by the caller. When present, it
/// MUST strictly increase on every apply. `provision_tenant` rejects an apply
/// whose `generation` is `<= current generation`. When absent, the server
/// treats the apply as `current + 1` and writes that back.
///
/// ### `providers`
///
/// Optional declarative provider block. Replaces the imperative
/// `POST /admin/tenants/{slug}/providers/llm` / `.../embedding` calls with a
/// field inside the manifest. Each provider entry carries a logical
/// `secret_ref` pointing into `config.secret_references` for any sensitive
/// material (API keys, service-account JSON). Plaintext secrets never appear
/// in a provider declaration; they travel only through `secrets` (inline,
/// stripped from the canonical hash) or through a pre-existing
/// `SecretReference` in the tenant's `tenant_secrets` rows.
///
/// Fields are optional for backward compatibility: a manifest without
/// `metadata` or `providers` still parses and applies.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TenantManifest {
    pub api_version: String,
    pub kind: String,
    /// Optional metadata block. See type docs.
    #[serde(default)]
    pub metadata: Option<ManifestMetadata>,
    pub tenant: ManifestTenant,
    #[serde(default)]
    pub config: Option<ManifestConfig>,
    #[serde(default)]
    pub secrets: Option<Vec<ManifestSecret>>,
    /// Optional declarative provider block (LLM / embedding / memory layers).
    /// See type docs on [`TenantManifest`].
    #[serde(default)]
    pub providers: Option<ManifestProviders>,
    #[serde(default)]
    pub repository: Option<SetTenantRepositoryBindingRequest>,
    #[serde(default)]
    pub hierarchy: Option<Vec<ManifestOrg>>,
    #[serde(default)]
    pub roles: Option<Vec<ManifestRoleAssignment>>,
}

/// Manifest metadata. All fields optional; `generation` is the only one with
/// defined semantics today — see [`TenantManifest`].
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestMetadata {
    /// Caller-owned monotonic counter. `provision_tenant` enforces strict
    /// increase across applies; when absent, the server auto-increments
    /// `last_generation + 1`.
    #[serde(default)]
    pub generation: Option<u64>,
    /// Free-form labels for operator use. Not interpreted by the server; part
    /// of the canonical hash so label drift counts as a manifest change.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// Free-form annotations (k8s-style). Same semantics as `labels`.
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}

/// Declarative provider block inside a manifest. Every sensitive field is a
/// `secretRef` (logical name) resolved against `config.secret_references`;
/// plaintext never travels inside a provider declaration.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestProviders {
    /// LLM provider declaration. `None` means "not managed declaratively".
    #[serde(default)]
    pub llm: Option<ManifestProvider>,
    /// Embedding provider declaration.
    #[serde(default)]
    pub embedding: Option<ManifestProvider>,
    /// Memory-layer providers keyed by layer name (e.g. `"episodic"`,
    /// `"semantic"`). Each entry is the same shape as the LLM/embedding
    /// block; specific `kind` values are validated by the layer wiring code.
    #[serde(default)]
    pub memory_layers: BTreeMap<String, ManifestProvider>,
}

/// A single provider declaration (LLM, embedding, or memory-layer).
///
/// `kind` is a free-form string matched against the factory for the relevant
/// layer (e.g. `"openai"`, `"anthropic"`, `"qdrant"`). Unknown `kind` values
/// surface at wire-time, not here.
///
/// `secret_ref`, when present, MUST refer to a logical name declared in
/// `config.secret_references` of the same manifest. Unknown refs are caught
/// by [`validate_manifest`].
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestProvider {
    pub kind: String,
    #[serde(default)]
    pub model: Option<String>,
    /// Logical name into `config.secret_references`. Absent = provider takes
    /// no secret (e.g. local-only embedding model).
    #[serde(default)]
    pub secret_ref: Option<String>,
    /// Provider-specific config key/value pairs. Non-sensitive only; anything
    /// sensitive must go through `secret_ref`. Values are strings by
    /// convention (numbers / booleans encoded as strings) so the canonical
    /// hash is stable across JSON-number vs string drift.
    #[serde(default)]
    pub config: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestTenant {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub account_ref: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default)]
    pub domain_mappings: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ManifestConfig {
    #[serde(default)]
    pub fields: BTreeMap<String, TenantConfigField>,
    #[serde(default)]
    pub secret_references: BTreeMap<String, TenantSecretReference>,
}

/// Inbound manifest sub-payload carrying a single tenant secret.
///
/// Same shape rationale as [`SetTenantSecretRequest`]: `secret_value` is a
/// [`mk_core::SecretBytes`] to keep plaintext out of `Debug`, logs, and any
/// accidental `Serialize` round-trip once the manifest request is parsed.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestSecret {
    pub logical_name: String,
    #[serde(default = "default_tenant_ownership")]
    pub ownership: TenantConfigOwnership,
    pub secret_value: mk_core::SecretBytes,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestOrg {
    pub name: String,
    #[serde(default)]
    pub teams: Option<Vec<ManifestTeam>>,
    #[serde(default)]
    pub members: Option<Vec<ManifestMember>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestTeam {
    pub name: String,
    #[serde(default)]
    pub members: Option<Vec<ManifestMember>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestMember {
    pub user_id: String,
    pub role: RoleIdentifier,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestRoleAssignment {
    pub user_id: String,
    pub role: RoleIdentifier,
    /// Hierarchy unit name or ID to scope the role to (optional — tenant-wide if absent).
    pub unit: Option<String>,
}

/// Per-step status returned in the provision response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProvisionStep {
    step: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl ProvisionStep {
    fn ok(step: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            step: step.into(),
            detail: Some(detail.into()),
            ok: true,
            error: None,
        }
    }
    fn fail(step: impl Into<String>, err: impl Into<String>) -> Self {
        Self {
            step: step.into(),
            detail: None,
            ok: false,
            error: Some(err.into()),
        }
    }
}

/// B2 §7.5 — record a completed step in the response vector **and**
/// broadcast a matching `TenantChangeKind::ProvisioningStep` event on
/// the tenant-event bus so active `aeterna tenant watch` clients see
/// progress in real time.
///
/// Skipped (response-only, no broadcast) during dry-run because a
/// preview is not an apply and watchers correlating events with an
/// in-flight `apply` would be misled. Callers inside the dry-run
/// branch still push to `steps` directly.
async fn push_step_and_publish(
    state: &AppState,
    slug: &str,
    steps: &mut Vec<ProvisionStep>,
    step: ProvisionStep,
) {
    // Mirror the ProvisionStep fields onto the pubsub wire shape:
    //   ok=true  → status="ok",     detail = step.detail
    //   ok=false → status="failed", detail = step.error
    // Both map cleanly onto the ProvisioningStep variant's `detail`
    // slot; the UTF-8-safe 512 B truncator inside `publish_step`
    // handles runaway error bodies.
    let wire_status = if step.ok { "ok" } else { "failed" };
    let wire_detail = if step.ok {
        step.detail.clone()
    } else {
        step.error.clone()
    };
    super::tenant_pubsub::publish_step(state, slug, &step.step, wire_status, wire_detail).await;
    steps.push(step);
}

/// B2 §7.5 — broadcast a `started` marker for a step that is about to
/// begin. Opt-in: only the long-running steps (secrets resolution,
/// hierarchy upsert) emit these; cheap steps (domain mappings, role
/// bindings) fire only `ok`/`failed` to keep the event cardinality
/// sane (one apply → ~7–10 events, not 14–20).
async fn publish_step_started(state: &AppState, slug: &str, step: &str) {
    super::tenant_pubsub::publish_step(state, slug, step, "started", None).await;
}

/// Validate a manifest before processing any steps.
/// Returns a list of human-readable error strings; empty means valid.
/// B3 §2.2-D — memory-layer key charset check.
///
/// Layer names are operator-provided and become part of the config-field
/// key namespace. Restricting them to `[a-z][a-z0-9_-]{0,62}` gives us:
///   * zero collision risk with existing flat snake_case keys
///     (`llm_provider`, `embedding_bedrock_region`, …) because the
///     memoryLayer keys are dot-separated at the prefix boundary,
///   * deterministic, operator-readable config dumps,
///   * a shape that already round-trips through k8s label / JSON key /
///     URL-path constraints without needing escape rules.
///
/// `.` is explicitly forbidden so `memory_layer.{layer}.provider`
/// parses back unambiguously (otherwise `memory_layer.a.b.provider`
/// would be either layer `a.b` field `provider` or layer `a` field
/// `b.provider`).
fn is_valid_memory_layer_key(layer: &str) -> bool {
    let bytes = layer.as_bytes();
    if bytes.is_empty() || bytes.len() > 63 {
        return false;
    }
    if !bytes[0].is_ascii_lowercase() {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
}

fn validate_manifest(m: &TenantManifest) -> Vec<String> {
    let mut errors: Vec<String> = Vec::new();

    if m.api_version != "aeterna.io/v1" {
        errors.push(format!(
            "apiVersion must be 'aeterna.io/v1', got '{}'",
            m.api_version
        ));
    }
    if m.kind != "TenantManifest" {
        errors.push(format!("kind must be 'TenantManifest', got '{}'", m.kind));
    }

    // Validate slug is kebab-case (lowercase alphanumeric + hyphens, no leading/trailing hyphen)
    let slug = &m.tenant.slug;
    if slug.is_empty() {
        errors.push("tenant.slug is required and must not be empty".into());
    } else if !slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        || slug.starts_with('-')
        || slug.ends_with('-')
    {
        errors.push(format!(
            "tenant.slug '{slug}' must be kebab-case (lowercase letters, digits, hyphens; no leading/trailing hyphens)"
        ));
    }

    if m.tenant.name.trim().is_empty() {
        errors.push("tenant.name is required and must not be empty".into());
    }

    // Validate role names in hierarchy members
    if let Some(orgs) = &m.hierarchy {
        for org in orgs {
            if org.name.trim().is_empty() {
                errors.push("hierarchy org name must not be empty".into());
            }
            if let Some(teams) = &org.teams {
                for team in teams {
                    if team.name.trim().is_empty() {
                        errors.push("hierarchy team name must not be empty".into());
                    }
                }
            }
        }
    }

    // Validate roles section
    if let Some(roles) = &m.roles {
        for assignment in roles {
            if assignment.user_id.trim().is_empty() {
                errors.push("roles[].userId must not be empty".into());
            }
            if matches!(assignment.role, RoleIdentifier::Known(Role::PlatformAdmin)) {
                errors.push(
                    "PlatformAdmin cannot be assigned as a tenant-scoped role in a manifest".into(),
                );
            }
        }
    }

    // ── providers block: every secret_ref must resolve in config.secret_references
    //    (missing config.secret_references is fine — it just means no refs are
    //    available, so any secret_ref at all is a miss).
    if let Some(providers) = &m.providers {
        let declared_refs: std::collections::HashSet<&str> = m
            .config
            .as_ref()
            .map(|c| c.secret_references.keys().map(String::as_str).collect())
            .unwrap_or_default();

        fn check_provider(
            slot: &str,
            p: &ManifestProvider,
            declared_refs: &std::collections::HashSet<&str>,
            errors: &mut Vec<String>,
        ) {
            if p.kind.trim().is_empty() {
                errors.push(format!("providers.{slot}.kind must not be empty"));
            }
            if let Some(ref_name) = &p.secret_ref
                && !declared_refs.contains(ref_name.as_str())
            {
                let declared_list = if declared_refs.is_empty() {
                    "none".to_string()
                } else {
                    let mut v: Vec<&str> = declared_refs.iter().copied().collect();
                    v.sort();
                    v.join(", ")
                };
                errors.push(format!(
                    "providers.{slot}.secretRef '{ref_name}' does not resolve in \
                         config.secretReferences (declared: {declared_list})"
                ));
            }
        }

        if let Some(llm) = &providers.llm {
            check_provider("llm", llm, &declared_refs, &mut errors);
        }
        if let Some(emb) = &providers.embedding {
            check_provider("embedding", emb, &declared_refs, &mut errors);
        }
        for (layer, p) in &providers.memory_layers {
            // B3 §2.2-D — layer names become part of the config-field
            // key namespace (`memory_layer.{layer}.provider`, etc.).
            // Restrict to `[a-z][a-z0-9_-]{0,62}` (k8s-label shape):
            //   - starts lowercase alpha (no leading digit / underscore
            //     / dash — avoids confusion with flat existing keys
            //     like `llm_bedrock_region`),
            //   - remaining chars are lowercase alphanumeric + `_` / `-`,
            //   - max 63 so the full key stays under storage's
            //     `TenantConfigField.key` length bound with headroom for
            //     the longest suffix (`google_project_id` = 17 chars).
            // Dots are forbidden to keep the `.`-separator unambiguous
            // at the prefix boundary.
            if layer.trim().is_empty() {
                errors.push("providers.memoryLayers has an entry with an empty key".into());
            } else if !is_valid_memory_layer_key(layer) {
                errors.push(format!(
                    "providers.memoryLayers key '{layer}' is invalid: must match \
                     [a-z][a-z0-9_-]{{0,62}} (lowercase, 1–63 chars, starts with a \
                     letter, no dots)"
                ));
            }
            check_provider(
                &format!("memoryLayers.{layer}"),
                p,
                &declared_refs,
                &mut errors,
            );
        }
    }

    // ── metadata.generation must be non-zero when present (0 is a common
    //    sentinel for "unset" and rejecting it catches accidental `0` from
    //    serializers that default numbers).
    if let Some(meta) = &m.metadata
        && meta.generation == Some(0)
    {
        errors.push("metadata.generation must be >= 1 when set (use omit for auto-assign)".into());
    }

    // ── secret_references: per-variant well-formedness (B1 §1.2). The
    //    serde `tag = "kind"` union already rejects unknown kinds at parse
    //    time; here we check that the REQUIRED fields within a known kind
    //    are non-empty and well-formed. An empty `var` on Env, empty
    //    `path` on File, etc. would otherwise pass parse and blow up only
    //    at resolution time — this surfaces them at apply as a 422.
    //
    //    Postgres is not user-authored (callers supply Inline; server
    //    stores it and rewrites to Postgres before persisting) so we
    //    skip it here — `set_secret_entry`'s write path already owns
    //    the Uuid invariant.
    if let Some(cfg) = &m.config {
        for (ref_name, entry) in &cfg.secret_references {
            if ref_name.trim().is_empty() {
                errors.push("config.secretReferences has an entry with an empty key".into());
            }
            if entry.logical_name.trim().is_empty() {
                errors.push(format!(
                    "config.secretReferences.{ref_name}.logicalName must not be empty"
                ));
            }
            match &entry.reference {
                mk_core::SecretReference::Inline { plaintext } => {
                    // SecretBytes::is_empty does not expose; just length-0 check.
                    if plaintext.is_empty() {
                        errors.push(format!(
                            "config.secretReferences.{ref_name}: inline plaintext must not be empty"
                        ));
                    }
                }
                mk_core::SecretReference::Postgres { .. } => {
                    // Not expected via manifest — Inline is rewritten to
                    // Postgres server-side. Tolerate it (round-trip of a
                    // rendered manifest would include it) without error.
                }
                mk_core::SecretReference::Env { var } => {
                    if var.trim().is_empty() {
                        errors.push(format!(
                            "config.secretReferences.{ref_name}.var must not be empty"
                        ));
                    } else if var.contains('=') || var.contains('\0') {
                        errors.push(format!(
                            "config.secretReferences.{ref_name}.var must not contain '=' or null bytes"
                        ));
                    }
                }
                mk_core::SecretReference::File { path } => {
                    if path.trim().is_empty() {
                        errors.push(format!(
                            "config.secretReferences.{ref_name}.path must not be empty"
                        ));
                    } else if !path.starts_with('/') {
                        errors.push(format!(
                            "config.secretReferences.{ref_name}.path must be absolute (start with '/')"
                        ));
                    }
                }
                mk_core::SecretReference::K8s {
                    name,
                    key,
                    namespace,
                } => {
                    if name.trim().is_empty() {
                        errors.push(format!(
                            "config.secretReferences.{ref_name}.name must not be empty"
                        ));
                    }
                    if key.trim().is_empty() {
                        errors.push(format!(
                            "config.secretReferences.{ref_name}.key must not be empty"
                        ));
                    }
                    if let Some(ns) = namespace
                        && ns.trim().is_empty()
                    {
                        errors.push(format!(
                                "config.secretReferences.{ref_name}.namespace, when set, must not be empty (omit to use the server's namespace)"
                            ));
                    }
                }
                mk_core::SecretReference::Vault { mount, path, field } => {
                    for (label, value) in [("mount", mount), ("path", path), ("field", field)] {
                        if value.trim().is_empty() {
                            errors.push(format!(
                                "config.secretReferences.{ref_name}.{label} must not be empty"
                            ));
                        }
                    }
                }
            }
        }
    }

    errors
}

/// `POST /api/v1/admin/tenants/provision`
///
/// PlatformAdmin-only.  Accepts a `TenantManifest` (JSON), processes it
/// step-by-step and returns a per-step status.
///
/// ## Idempotency model (B2, tasks 1.5 + 1.6)
///
/// Every apply computes a canonical SHA-256 fingerprint of the manifest
/// (see [`crate::server::manifest_hash`]). The fingerprint is then compared
/// against the tenant row's `last_applied_manifest_hash`:
///
/// 1. **Hash match** → short-circuit return with `status = "unchanged"` and
///    `StatusCode::OK`. The apply pipeline does not run. This is the O(1)
///    re-apply path that makes `provision_tenant` safe to call on a loop
///    (GitOps reconcile, operator-written controllers, etc.) without
///    re-doing work.
///
/// 2. **Hash differs** → the manifest's `metadata.generation` is checked
///    against the row's `manifest_generation` and enforced strictly:
///    - when the caller sets `generation`, it MUST be
///      `> manifest_generation` else we return `409 Conflict`
///      (`error = "generation_conflict"`);
///    - when the caller omits `generation`, the server auto-assigns
///      `manifest_generation + 1`.
///    On success the new `(hash, generation)` pair is persisted via
///    [`TenantStore::set_manifest_state`].
///
/// First-time apply (no tenant row yet) falls through to the full pipeline
/// with auto-assigned `generation = 1`. This keeps the first-apply UX
/// trivial: the caller does not need to know the current generation to
/// bootstrap.
///
/// The handler takes `Json<serde_json::Value>` rather than
/// `Json<TenantManifest>` so the raw object is available for hashing. Typed
/// deserialization happens just after, with the same `400 Bad Request`
/// surface as the previous `Json<TenantManifest>` extractor.
///
/// ### Dry-run (`?dryRun=true`)
///
/// Task §2.1 of `harden-tenant-provisioning`. When `dryRun=true` the handler
/// runs the full **read-only** preamble — typed parse, `validate_manifest`,
/// canonical hash, manifest-state snapshot, and generation gate — then
/// returns a structured [`ProvisionPlan`] without touching the tenant store,
/// the config provider, the binding store, the governance log, or emitting
/// any `TenantCreated`/`TenantConfigChanged` events. Idempotent
/// short-circuit (hash match) and `generation_conflict` (409) surface
/// identically in dry-run mode: callers opt in to "preview only" but not
/// "skip validation".
///
/// This is the preview surface used by `aeterna tenant validate -f <file>`
/// and by operator pre-change review. Because dry-run deliberately stops
/// before any write, it cannot detect downstream failures (config provider
/// rejecting a field, repository-binding credential mismatch, etc.) — the
/// plan answers "**would this be accepted**", not "**would this succeed**".
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProvisionQuery {
    /// When `true`, return a [`ProvisionPlan`] and skip all writes.
    dry_run: Option<bool>,
    /// B2 task 4.2: caller opt-in for inline `secrets[].secretValue`
    /// plaintext. Only honored when the *server* also has
    /// `provisioning.allow_inline_secret = true` (and then, only in
    /// non-release builds — see
    /// [`config::ProvisioningConfig::effective_allow_inline_secret`]).
    ///
    /// Both switches must be on; either one missing and the endpoint
    /// rejects non-empty inline plaintext with a 422 whose body points
    /// the caller at `config.secretReferences` (task 4.3).
    allow_inline: Option<bool>,
}

/// Response body for `POST /admin/tenants/provision?dryRun=true`.
///
/// Shape is intentionally flat (no nested diff) — a full structural diff
/// is task §2.4, which is blocked on the reverse-renderer reaching
/// coverage parity with `provision_tenant`. `ProvisionPlan` answers the
/// simpler question: *what effect would a non-dry-run apply have?* via
/// the `status` classifier, the hash pair, and per-section presence
/// flags.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProvisionPlan {
    /// Always `true` for dry-run responses (explicit so clients that
    /// persist the JSON cannot later mistake it for a real apply record).
    dry_run: bool,
    /// `"unchanged"` — the incoming hash matches `last_applied_manifest_hash`;
    /// a non-dry-run apply of the same manifest would short-circuit.
    /// `"create"` — no prior manifest state exists for this slug; the row
    /// may or may not exist (tenant rows can predate the B2 manifest-state
    /// column), but the pipeline would run end-to-end.
    /// `"update"` — prior manifest state exists and the hash differs; the
    /// pipeline would run and bump the generation to `nextGeneration`.
    status: &'static str,
    slug: String,
    incoming_hash: String,
    /// `None` for the `"create"` path; `Some` (possibly with inner `None`
    /// if the row pre-dates B2) otherwise.
    current_hash: Option<String>,
    current_generation: i64,
    next_generation: i64,
    /// Config fields in the submitted manifest. Says nothing about how
    /// many of these would be new vs updated — that is §2.4 territory.
    config_field_count: usize,
    secret_reference_count: usize,
    has_repository_binding: bool,
    has_domain_mappings: bool,
    has_hierarchy: bool,
    has_roles: bool,
    has_providers: bool,
}

async fn provision_tenant(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ProvisionQuery>,
    Json(raw): Json<serde_json::Value>,
) -> impl IntoResponse {
    // B2 §10.5 + §11.4 — either a PlatformAdmin user OR a service
    // token carrying the `tenants:provision` scope is accepted. The
    // audit helpers (`audit_tenant_action`, `audit_tenant_action_ext`)
    // are actor-polymorphic since §11.4: a service-principal caller is
    // recorded as `actor_type='agent'` with `actor_id=<agent_uuid>`
    // and the original `tenant_id` of the minted service token. All
    // provisioning audit rows therefore link cause (which principal)
    // to effect (target tenant + manifest hash + generation) whether
    // the apply came from a CI pipeline or a PA operator.
    let principal =
        match require_platform_admin_or_scope(&state, &headers, "tenants:provision").await {
            Ok(p) => p,
            Err(response) => return response,
        };

    // B2 §11.4 — synthesize a `TenantContext` from the authenticated
    // principal so helpers that still require a ctx (notably
    // [`persist_governance_event`], which forwards it to the admin-pool
    // `with_admin_context` write path) accept both callers uniformly.
    // See [`AuthPrincipal::to_admin_pool_context`] for the per-variant
    // synthesis rules. Audit rows use `principal.as_audit_actor()`
    // directly (below) so they record `actor_type = 'agent'` for
    // service callers — this ctx is only for call sites that still
    // type-require `&TenantContext`.
    let ctx: mk_core::types::TenantContext = principal.to_admin_pool_context();

    // B2 §11.4 — base request-context extensions for every audit row this
    // handler writes. Cloned + amended at each call site with the values
    // that are only in scope later (manifest_hash, generation, dry_run).
    let audit_ext_base = provision_audit_ext_base(&headers);

    // ── Typed deserialization ────────────────────────────────────────────
    // We keep `raw` alive alongside `manifest` so the canonical hash can be
    // computed directly from the wire shape, not the typed re-serialization
    // (which would require Serialize impls on every sub-type).
    let manifest: TenantManifest = match serde_json::from_value(raw.clone()) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "error": "manifest_parse_failed",
                    "details": e.to_string(),
                })),
            )
                .into_response();
        }
    };

    // ── Pre-flight validation ─────────────────────────────────────────────
    let validation_errors = validate_manifest(&manifest);
    if !validation_errors.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "success": false,
                "error": "manifest_validation_failed",
                "validationErrors": validation_errors,
            })),
        )
            .into_response();
    }

    // ── Inline-secret gate (B2 tasks 4.1 / 4.2 / 4.3) ────────────────────
    // Non-empty `secrets[].secretValue` is only accepted when BOTH the
    // server flag and the caller's query param say so (and the server
    // flag itself is forced off in release builds). Empty-valued
    // entries are always allowed: they describe intent to have a logical
    // name without delivering plaintext on the wire — useful for
    // dry-runs and for manifests that bind every secret via
    // `config.secretReferences` instead.
    if let Some(secrets) = manifest.secrets.as_ref() {
        let has_inline_plaintext = secrets.iter().any(|s| !s.secret_value.is_empty());
        if has_inline_plaintext {
            let server_allows = state.config.provisioning.effective_allow_inline_secret();
            let caller_allows = query.allow_inline.unwrap_or(false);
            if !(server_allows && caller_allows) {
                // Enumerate offending logical names so the caller sees
                // exactly which entries to migrate — do not echo the
                // values themselves (they are SecretBytes, and even the
                // derived length would be a side-channel hint).
                let offending: Vec<String> = secrets
                    .iter()
                    .filter(|s| !s.secret_value.is_empty())
                    .map(|s| s.logical_name.clone())
                    .collect();
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({
                        "success": false,
                        "error": "inline_secret_not_allowed",
                        "message":
                            "Inline `secrets[].secretValue` plaintext is \
                             disabled on this server. Move these entries \
                             to `config.secretReferences` and have a \
                             SecretReference resolver deliver the bytes, \
                             or ask an operator to enable \
                             `provisioning.allowInlineSecret` (dev builds \
                             only) and re-submit with `?allowInline=true`.",
                        "offendingSecrets": offending,
                        "remediation": {
                            "preferred": "config.secretReferences",
                            "fallback": {
                                "serverConfigKey": "provisioning.allowInlineSecret",
                                "queryParameter": "allowInline=true",
                                "serverFlagEnabled": server_allows,
                                "queryFlagProvided": caller_allows,
                            }
                        }
                    })),
                )
                    .into_response();
            }
        }
    }

    // ── Canonical hash ────────────────────────────────────────────────────
    // Computed from the raw wire shape so future Serialize impls on the
    // typed structs cannot drift the hash input out from under existing
    // fingerprints. `hash_manifest_value` strips inline secret plaintext
    // from the input before hashing (see module docs).
    let incoming_hash = match crate::server::manifest_hash::hash_manifest_value(&raw) {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "error": "manifest_hash_failed",
                    "details": e.to_string(),
                })),
            )
                .into_response();
        }
    };

    // ── Idempotent short-circuit + generation gate ───────────────────────
    // Only meaningful when the tenant already exists; a missing row means
    // "first-time apply" and falls through with `new_generation = 1`. We
    // take a snapshot here and race-check via a generation-guarded UPDATE
    // at the end of the pipeline.
    let prior_state = match state
        .tenant_store
        .get_manifest_state(&manifest.tenant.slug)
        .await
    {
        Ok((hash, generation)) => Some((hash, generation)),
        Err(storage::postgres::PostgresError::NotFound(_)) => None,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "error": "manifest_state_read_failed",
                    "details": err.to_string(),
                })),
            )
                .into_response();
        }
    };

    let caller_generation = manifest.metadata.as_ref().and_then(|m| m.generation);

    let current_generation: i64 = prior_state.as_ref().map(|(_, g)| *g).unwrap_or(0);

    // Hash-match short-circuit. Keyed only on a non-NULL prior hash: a row
    // that has never been applied via the B2 path (NULL hash) always runs
    // the full pipeline, even if by coincidence the incoming hash would
    // match some stored sentinel — there is no sentinel, but the
    // `if Some(prior)` guard documents the intent.
    if let Some((Some(prior_hash), _)) = prior_state.as_ref()
        && prior_hash == &incoming_hash
    {
        let is_dry_run = query.dry_run.unwrap_or(false);
        // Dry-run on an unchanged manifest audits a preview event,
        // not a real unchanged-apply. Callers treat these separately
        // (a preview does not imply the operator decided to proceed).
        audit_tenant_action_ext(
            state.as_ref(),
            principal.as_audit_actor(),
            if is_dry_run {
                "tenant_provision_dry_run_unchanged"
            } else {
                "tenant_provision_unchanged"
            },
            None,
            json!({
                "slug": manifest.tenant.slug,
                "hash": incoming_hash,
                "generation": current_generation,
                "dryRun": is_dry_run,
            }),
            AuditExtensions {
                manifest_hash: Some(incoming_hash.clone()),
                generation: Some(current_generation),
                dry_run: Some(is_dry_run),
                ..audit_ext_base.clone()
            },
        )
        .await;
        return (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "status": "unchanged",
                "slug": manifest.tenant.slug,
                "hash": incoming_hash,
                "generation": current_generation,
                "steps": Vec::<ProvisionStep>::new(),
                "dryRun": is_dry_run,
            })),
        )
            .into_response();
    }

    // Strict-monotonic generation check. Only rejects when the caller
    // explicitly set a non-increasing value; omitted `generation` is
    // auto-assigned. A concurrent provision may still bump the row between
    // this check and the final write — the guarded UPDATE at the tail
    // catches that race (see end of handler).
    let new_generation: i64 = match caller_generation {
        Some(g) => {
            let g_i64 = g as i64;
            if g_i64 <= current_generation {
                return (
                    StatusCode::CONFLICT,
                    Json(json!({
                        "success": false,
                        "error": "generation_conflict",
                        "slug": manifest.tenant.slug,
                        "currentGeneration": current_generation,
                        "submittedGeneration": g,
                        "hint": "metadata.generation must be strictly greater than the current generation; omit it to auto-assign",
                    })),
                )
                    .into_response();
            }
            g_i64
        }
        None => current_generation.saturating_add(1),
    };

    // ── Dry-run short-circuit (§2.1) ─────────────────────────────────────
    // All read-only checks have passed at this point: typed parse,
    // `validate_manifest`, canonical hash, and the strict-monotonic
    // generation gate. The next thing the non-dry-run path does is the
    // first write (`ensure_tenant_with_source`); we stop here and return
    // a plan instead. The plan is computed from `manifest` and the
    // `prior_state` snapshot only — we do NOT re-read from the store,
    // so a concurrent apply that lands between plan and apply is not
    // reflected here (the race-guarded UPDATE at the tail of a real
    // apply is what catches that; the plan is advisory only).
    if query.dry_run.unwrap_or(false) {
        let status = if prior_state.is_none() {
            "create"
        } else {
            "update"
        };
        audit_tenant_action_ext(
            state.as_ref(),
            principal.as_audit_actor(),
            "tenant_provision_dry_run",
            None,
            json!({
                "slug": manifest.tenant.slug,
                "status": status,
                "incomingHash": incoming_hash,
                "currentGeneration": current_generation,
                "nextGeneration": new_generation,
            }),
            AuditExtensions {
                manifest_hash: Some(incoming_hash.clone()),
                generation: Some(new_generation),
                dry_run: Some(true),
                ..audit_ext_base.clone()
            },
        )
        .await;
        let plan = ProvisionPlan {
            dry_run: true,
            status,
            slug: manifest.tenant.slug.clone(),
            incoming_hash: incoming_hash.clone(),
            current_hash: prior_state.as_ref().and_then(|(h, _)| h.clone()),
            current_generation,
            next_generation: new_generation,
            config_field_count: manifest
                .config
                .as_ref()
                .map(|c| c.fields.len())
                .unwrap_or(0),
            secret_reference_count: manifest
                .config
                .as_ref()
                .map(|c| c.secret_references.len())
                .unwrap_or(0),
            has_repository_binding: manifest.repository.is_some(),
            has_domain_mappings: manifest
                .tenant
                .domain_mappings
                .as_ref()
                .is_some_and(|d| !d.is_empty()),
            has_hierarchy: manifest.hierarchy.as_ref().is_some_and(|h| !h.is_empty()),
            has_roles: manifest.roles.as_ref().is_some_and(|r| !r.is_empty()),
            has_providers: manifest.providers.is_some(),
        };
        return (StatusCode::OK, Json(plan)).into_response();
    }

    let mut steps: Vec<ProvisionStep> = Vec::new();
    let mut overall_ok = true;

    // B2 §7.5 — slug is captured once, up-front, for use by
    // `push_step_and_publish` at each step boundary. The manifest is
    // still fully owned after parse so borrowing `&manifest.tenant.slug`
    // would also work, but caching avoids a subtle aliasing risk if a
    // future refactor moves the manifest into a step handler.
    let watch_slug: String = manifest.tenant.slug.clone();

    // ── Step 1: Create or ensure tenant ──────────────────────────────────
    let tenant_record = match state
        .tenant_store
        .ensure_tenant_with_source(&manifest.tenant.slug, mk_core::types::RecordSource::Admin)
        .await
    {
        Ok(record) => {
            let now = chrono::Utc::now().timestamp();
            // Only fire TenantCreated event when this is a brand-new tenant.
            // `ensure_tenant_with_source` is idempotent; we distinguish by checking
            // whether created_at ≈ updated_at (i.e. just created).
            if record.created_at == record.updated_at {
                persist_governance_event(
                    state.as_ref(),
                    &ctx,
                    &GovernanceEvent::TenantCreated {
                        record_id: record.id.as_str().to_string(),
                        slug: record.slug.clone(),
                        tenant_id: record.id.clone(),
                        timestamp: now,
                    },
                )
                .await;
            }
            let store = storage::AccountStore::new(state.postgres.pool().clone());
            let resolved_account = match manifest.tenant.account_ref.as_deref() {
                Some(account_ref) => match store.get(account_ref).await {
                    Ok(Some(account)) => Some(account),
                    Ok(None) => {
                        push_step_and_publish(
                            state.as_ref(),
                            &watch_slug,
                            &mut steps,
                            ProvisionStep::fail(
                                "tenant",
                                format!("accountRef '{}' not found", account_ref),
                            ),
                        )
                        .await;
                        return (
                            StatusCode::NOT_FOUND,
                            Json(json!({
                                "success": false,
                                "error": "account_not_found",
                                "accountRef": account_ref,
                                "steps": steps,
                            })),
                        )
                            .into_response();
                    }
                    Err(err) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "success": false,
                                "error": "account_lookup_failed",
                                "details": err.to_string(),
                            })),
                        )
                            .into_response();
                    }
                },
                None => None,
            };

            let updated = match state
                .tenant_store
                .update_tenant(
                    record.id.as_str(),
                    None,
                    Some(&manifest.tenant.name),
                    manifest.tenant.environment.as_deref(),
                )
                .await
            {
                Ok(Some(updated)) => updated,
                Ok(None) => record.clone(),
                Err(err) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "success": false,
                            "error": "tenant_update_failed",
                            "details": err.to_string(),
                        })),
                    )
                        .into_response();
                }
            };

            let final_record = if let Some(account) = resolved_account {
                match state
                    .tenant_store
                    .attach_account(updated.id.as_str(), &account.id)
                    .await
                {
                    Ok(Some(attached)) => attached,
                    Ok(None) => updated,
                    Err(err) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "success": false,
                                "error": "tenant_account_attach_failed",
                                "details": err.to_string(),
                            })),
                        )
                            .into_response();
                    }
                }
            } else {
                updated
            };

            audit_tenant_action_ext(
                state.as_ref(),
                principal.as_audit_actor(),
                "tenant_provision_tenant",
                Some(final_record.id.as_str()),
                json!({
                    "slug": final_record.slug,
                    "name": final_record.name,
                    "account": final_record.account,
                    "environment": final_record.environment,
                }),
                AuditExtensions {
                    manifest_hash: Some(incoming_hash.clone()),
                    generation: Some(new_generation),
                    dry_run: Some(false),
                    ..audit_ext_base.clone()
                },
            )
            .await;
            push_step_and_publish(
                state.as_ref(),
                &watch_slug,
                &mut steps,
                ProvisionStep::ok(
                    "tenant",
                    format!(
                        "Tenant '{}' ensured (id={})",
                        final_record.slug,
                        final_record.id.as_str()
                    ),
                ),
            )
            .await;
            final_record
        }
        Err(err) => {
            push_step_and_publish(
                state.as_ref(),
                &watch_slug,
                &mut steps,
                ProvisionStep::fail("tenant", err.to_string()),
            )
            .await;
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "tenantId": null,
                    "steps": steps,
                })),
            )
                .into_response();
        }
    };

    let tenant_id = tenant_record.id.clone();

    // ── Step 2: Domain mappings ───────────────────────────────────────────
    if let Some(domains) = &manifest.tenant.domain_mappings {
        let mut domain_errors: Vec<String> = Vec::new();
        for domain in domains {
            match state
                .tenant_store
                .add_verified_domain_mapping(tenant_id.as_str(), domain)
                .await
            {
                Ok(_) => {}
                Err(err) => domain_errors.push(format!("{domain}: {err}")),
            }
        }
        if domain_errors.is_empty() {
            push_step_and_publish(
                state.as_ref(),
                &watch_slug,
                &mut steps,
                ProvisionStep::ok(
                    "domain_mappings",
                    format!("{} domain(s) mapped", domains.len()),
                ),
            )
            .await;
        } else {
            push_step_and_publish(
                state.as_ref(),
                &watch_slug,
                &mut steps,
                ProvisionStep::fail("domain_mappings", domain_errors.join("; ")),
            )
            .await;
            overall_ok = false;
        }
    }

    // ── Step 3: Config fields ─────────────────────────────────────────────
    // `manifest.providers` is flattened into the config document here so
    // the apply is the ONLY write path that touches `config.fields`
    // (instead of having a separate providers-upsert step that could
    // race with the main config write or leave the tenant half-applied
    // on a mid-sequence failure). See FINDINGS-2-2 §2.2-A for the bug
    // this closes (manifest providers were validated but never persisted).
    let has_providers = manifest.providers.is_some();
    if manifest.config.is_some() || has_providers {
        let cfg = manifest.config.as_ref();
        let mut doc = TenantConfigDocument {
            tenant_id: tenant_id.clone(),
            fields: cfg.map(|c| c.fields.clone()).unwrap_or_default(),
            secret_references: cfg.map(|c| c.secret_references.clone()).unwrap_or_default(),
        };
        if let Some(providers) = &manifest.providers {
            apply_manifest_providers_to_config(providers, &mut doc);
        }
        if !doc.fields.is_empty() || !doc.secret_references.is_empty() {
            match state.tenant_config_provider.upsert_config(doc).await {
                Ok(config) => {
                    audit_tenant_action_ext(
                        state.as_ref(),
                        principal.as_audit_actor(),
                        "tenant_provision_config",
                        Some(tenant_id.as_str()),
                        json!({
                            "tenantId": tenant_id.as_str(),
                            "fieldCount": config.fields.len(),
                        }),
                        AuditExtensions {
                            manifest_hash: Some(incoming_hash.clone()),
                            generation: Some(new_generation),
                            dry_run: Some(false),
                            ..audit_ext_base.clone()
                        },
                    )
                    .await;
                    push_step_and_publish(
                        state.as_ref(),
                        &watch_slug,
                        &mut steps,
                        ProvisionStep::ok(
                            "config",
                            format!("{} field(s) applied", config.fields.len()),
                        ),
                    )
                    .await;
                }
                Err(err) => {
                    push_step_and_publish(
                        state.as_ref(),
                        &watch_slug,
                        &mut steps,
                        ProvisionStep::fail("config", err.to_string()),
                    )
                    .await;
                    overall_ok = false;
                }
            }
        }
    }

    // ── Step 4: Secrets ───────────────────────────────────────────────────
    if let Some(secrets) = &manifest.secrets {
        // B2 §7.5 — secrets can take seconds to resolve (external
        // SecretReference fetches: AWS SM, Vault, etc.). Emit a
        // `started` marker so watchers see the phase enter, not a
        // ~2s gap followed by `ok`.
        publish_step_started(state.as_ref(), &watch_slug, "secrets").await;
        let mut secret_errors: Vec<String> = Vec::new();
        let mut secrets_ok: usize = 0;
        for s in secrets {
            let entry = TenantSecretEntry {
                logical_name: s.logical_name.clone(),
                ownership: s.ownership.clone(),
                secret_value: s.secret_value.clone(),
            };
            match state
                .tenant_config_provider
                .set_secret_entry(&tenant_id, entry)
                .await
            {
                Ok(_) => secrets_ok += 1,
                Err(err) => secret_errors.push(format!("{}: {}", s.logical_name, err)),
            }
        }
        if secret_errors.is_empty() {
            push_step_and_publish(
                state.as_ref(),
                &watch_slug,
                &mut steps,
                ProvisionStep::ok("secrets", format!("{secrets_ok} secret(s) stored")),
            )
            .await;
        } else {
            push_step_and_publish(
                state.as_ref(),
                &watch_slug,
                &mut steps,
                ProvisionStep::fail("secrets", secret_errors.join("; ")),
            )
            .await;
            overall_ok = false;
        }
    }

    // ── Step 5: Repository binding ────────────────────────────────────────
    if let Some(repo) = &manifest.repository {
        // Validate credential ref
        let candidate = mk_core::types::TenantRepositoryBinding {
            id: String::new(),
            tenant_id: tenant_id.clone(),
            kind: repo.kind.clone(),
            local_path: repo.local_path.clone(),
            remote_url: repo.remote_url.clone(),
            branch: repo.branch.clone(),
            branch_policy: repo.branch_policy.clone(),
            credential_kind: repo.credential_kind.clone(),
            credential_ref: repo.credential_ref.clone(),
            github_owner: repo.github_owner.clone(),
            github_repo: repo.github_repo.clone(),
            source_owner: repo.source_owner.clone(),
            git_provider_connection_id: repo.git_provider_connection_id.clone(),
            // Sentinel timestamps for an unsaved validation candidate; storage
            // overwrites these on persistence.
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let repo_step = if let Err(reason) = candidate.validate_credential_ref() {
            ProvisionStep::fail("repository", format!("invalid_credential_ref: {reason}"))
        } else {
            // Check git provider connection access
            let conn_allowed = if let Some(ref conn_id) = repo.git_provider_connection_id {
                match state
                    .git_provider_connection_registry
                    .tenant_can_use(conn_id, &tenant_id)
                    .await
                {
                    Ok(allowed) => {
                        if allowed {
                            Ok(())
                        } else {
                            Err(format!(
                                "Tenant '{}' is not in the allow-list for connection '{conn_id}'",
                                tenant_id.as_str()
                            ))
                        }
                    }
                    Err(e) => Err(e.to_string()),
                }
            } else {
                Ok(())
            };

            if let Err(msg) = conn_allowed {
                ProvisionStep::fail("repository", msg)
            } else {
                let binding_request = UpsertTenantRepositoryBinding {
                    tenant_id: tenant_id.clone(),
                    kind: repo.kind.clone(),
                    local_path: repo.local_path.clone(),
                    remote_url: repo.remote_url.clone(),
                    branch: repo.branch.clone(),
                    branch_policy: repo.branch_policy.clone(),
                    credential_kind: repo.credential_kind.clone(),
                    credential_ref: repo.credential_ref.clone(),
                    github_owner: repo.github_owner.clone(),
                    github_repo: repo.github_repo.clone(),
                    source_owner: repo.source_owner.clone(),
                    git_provider_connection_id: repo.git_provider_connection_id.clone(),
                };
                match state
                    .tenant_repository_binding_store
                    .upsert_binding(binding_request)
                    .await
                {
                    Ok(binding) => {
                        let now = chrono::Utc::now().timestamp();
                        persist_governance_event(
                            state.as_ref(),
                            &ctx,
                            &GovernanceEvent::RepositoryBindingCreated {
                                binding_id: binding.id.clone(),
                                tenant_id: tenant_id.clone(),
                                timestamp: now,
                            },
                        )
                        .await;
                        state.tenant_repo_resolver.invalidate(&tenant_id);
                        audit_tenant_action_ext(
                            state.as_ref(),
                            principal.as_audit_actor(),
                            "tenant_provision_repository",
                            Some(binding.id.as_str()),
                            json!({
                                "tenantId": tenant_id.as_str(),
                                "kind": binding.kind,
                                "branch": binding.branch,
                            }),
                            AuditExtensions {
                                manifest_hash: Some(incoming_hash.clone()),
                                generation: Some(new_generation),
                                dry_run: Some(false),
                                ..audit_ext_base.clone()
                            },
                        )
                        .await;
                        ProvisionStep::ok("repository", format!("binding id={}", binding.id))
                    }
                    Err(err) => ProvisionStep::fail("repository", err.to_string()),
                }
            }
        };
        if !repo_step.ok {
            overall_ok = false;
        }
        // Repository binding can validate connection membership + git
        // provider auth end-to-end, so it's one of the slower steps;
        // broadcast the terminal state (ok/failed) alongside the
        // response push.
        push_step_and_publish(state.as_ref(), &watch_slug, &mut steps, repo_step).await;
    }

    // ── Step 6: Organizational hierarchy ─────────────────────────────────
    // We build a TenantContext from the platform-admin ctx but scoped to the
    // newly provisioned tenant so that get_unit / create_unit work correctly.
    // B2 §11.4 — synthesize a downstream `TenantContext` from the
    // authenticated principal. For a User caller this is the user's
    // own ctx scoped to the new tenant; for a Service caller we
    // fabricate one with `user_id=agent_uuid`, empty roles, and
    // `agent_id=Some(agent_uuid)` so downstream org-hierarchy ops
    // can attribute writes without a user identity.
    let tenant_ctx = principal.to_downstream_tenant_context(tenant_id.clone());

    if let Some(org_roots) = &manifest.hierarchy {
        // B2 §7.5 — hierarchy upsert loops through every organization /
        // team and round-trips to Postgres per unit.
        // Emit a `started` marker so watchers see the phase enter.
        publish_step_started(state.as_ref(), &watch_slug, "hierarchy").await;
        let mut hierarchy_errors: Vec<String> = Vec::new();
        let mut units_created: usize = 0;
        let now = chrono::Utc::now().timestamp();

        for org in org_roots {
            let org_unit = OrganizationalUnit {
                id: Uuid::new_v4().to_string(),
                name: org.name.clone(),
                unit_type: UnitType::Organization,
                parent_id: None,
                tenant_id: tenant_id.clone(),
                metadata: HashMap::new(),
                source_owner: RecordSource::Admin,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };
            if let Err(err) = state
                .postgres
                .create_unit_scoped(&tenant_ctx, &org_unit)
                .await
            {
                hierarchy_errors.push(format!("org '{}': {err}", org_unit.name));
                continue;
            }
            persist_governance_event(
                state.as_ref(),
                &ctx,
                &GovernanceEvent::UnitCreated {
                    unit_id: org_unit.id.clone(),
                    unit_type: org_unit.unit_type,
                    tenant_id: tenant_id.clone(),
                    parent_id: None,
                    timestamp: now,
                },
            )
            .await;
            units_created += 1;

            let org_id = org_unit.id.clone();

            // Assign org-level members
            for member in org.members.iter().flatten() {
                let user_id = if let Some(id) = mk_core::types::UserId::new(member.user_id.clone())
                {
                    id
                } else {
                    hierarchy_errors.push(format!(
                        "org '{}' member: invalid user_id '{}'",
                        org.name, member.user_id
                    ));
                    continue;
                };
                if let Err(err) = state
                    .postgres
                    .assign_role_scoped(&tenant_ctx, &user_id, &org_id, member.role.clone())
                    .await
                {
                    hierarchy_errors.push(format!(
                        "org '{}' member '{}': {err}",
                        org.name, member.user_id
                    ));
                } else {
                    persist_governance_event(
                        state.as_ref(),
                        &ctx,
                        &GovernanceEvent::RoleAssigned {
                            user_id: user_id.clone(),
                            unit_id: org_id.clone(),
                            role: member.role.clone(),
                            tenant_id: tenant_id.clone(),
                            timestamp: now,
                        },
                    )
                    .await;
                }
            }

            for team in org.teams.iter().flatten() {
                let team_unit = OrganizationalUnit {
                    id: Uuid::new_v4().to_string(),
                    name: team.name.clone(),
                    unit_type: UnitType::Team,
                    parent_id: Some(org_id.clone()),
                    tenant_id: tenant_id.clone(),
                    metadata: HashMap::new(),
                    source_owner: RecordSource::Admin,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                };
                if let Err(err) = state
                    .postgres
                    .create_unit_scoped(&tenant_ctx, &team_unit)
                    .await
                {
                    hierarchy_errors.push(format!("team '{}': {err}", team_unit.name));
                    continue;
                }
                persist_governance_event(
                    state.as_ref(),
                    &ctx,
                    &GovernanceEvent::UnitCreated {
                        unit_id: team_unit.id.clone(),
                        unit_type: team_unit.unit_type,
                        tenant_id: tenant_id.clone(),
                        parent_id: Some(org_id.clone()),
                        timestamp: now,
                    },
                )
                .await;
                units_created += 1;

                let team_id = team_unit.id.clone();

                // Assign team-level members
                for member in team.members.iter().flatten() {
                    let user_id =
                        if let Some(id) = mk_core::types::UserId::new(member.user_id.clone()) {
                            id
                        } else {
                            hierarchy_errors.push(format!(
                                "team '{}' member: invalid user_id '{}'",
                                team.name, member.user_id
                            ));
                            continue;
                        };
                    if let Err(err) = state
                        .postgres
                        .assign_role_scoped(&tenant_ctx, &user_id, &team_id, member.role.clone())
                        .await
                    {
                        hierarchy_errors.push(format!(
                            "team '{}' member '{}': {err}",
                            team.name, member.user_id
                        ));
                    } else {
                        persist_governance_event(
                            state.as_ref(),
                            &ctx,
                            &GovernanceEvent::RoleAssigned {
                                user_id: user_id.clone(),
                                unit_id: team_id.clone(),
                                role: member.role.clone(),
                                tenant_id: tenant_id.clone(),
                                timestamp: now,
                            },
                        )
                        .await;
                    }
                }
            }
        }

        // ── B3/B4 §2.2-B — also write the manifest hierarchy to the
        // modern tenant-root hierarchy tables so idp-sync, OPAL's
        // `v_hierarchy` view, and the
        // reverse-render path all see manifest-declared hierarchy.
        //
        // This runs *in addition* to the legacy `organizational_units`
        // writes above — not as a replacement — because several code
        // paths (backup, GDPR export, cascade delete, storage::postgres
        // role resolution fallback) still read OU. The two trees
        // intentionally have different UUIDs: OU nodes carry randomly
        // generated ids from this function; modern rows carry
        // DB-generated ids surfaced by `ON CONFLICT DO UPDATE …
        // RETURNING id`. The link between them is logical (matching
        // name/slug under the same tenant), not physical.
        //
        // Error handling is symmetrical to the OU path: one failure
        // appends a message but does NOT short-circuit the tenant
        // provision — the rest of the steps still run so the operator
        // sees a consolidated diagnostic.
        let modern_hierarchy_summary = match Uuid::parse_str(tenant_id.as_str()) {
            Ok(tenant_uuid) => {
                let org_inputs: Vec<storage::hierarchy_store::OrgInput> = org_roots
                    .iter()
                    .map(|o| storage::hierarchy_store::OrgInput {
                        slug: storage::hierarchy_store::slugify(&o.name),
                        name: o.name.clone(),
                        teams: o
                            .teams
                            .iter()
                            .flatten()
                            .map(|t| storage::hierarchy_store::TeamInput {
                                slug: storage::hierarchy_store::slugify(&t.name),
                                name: t.name.clone(),
                            })
                            .collect(),
                    })
                    .collect();
                let store =
                    storage::hierarchy_store::HierarchyStore::new(state.postgres.pool().clone());
                match store.upsert_hierarchy(tenant_uuid, &org_inputs).await {
                    Ok(summary) => Some(summary),
                    Err(err) => {
                        hierarchy_errors.push(format!("modern hierarchy upsert: {err}"));
                        None
                    }
                }
            }
            Err(err) => {
                hierarchy_errors.push(format!(
                    "tenant id is not a valid UUID, skipping modern hierarchy upsert: {err}"
                ));
                None
            }
        };

        if hierarchy_errors.is_empty() {
            let extra = modern_hierarchy_summary
                .map(|s| {
                    format!(
                        " (modern: {} orgs, {} teams)",
                        s.orgs_upserted, s.teams_upserted
                    )
                })
                .unwrap_or_default();
            push_step_and_publish(
                state.as_ref(),
                &watch_slug,
                &mut steps,
                ProvisionStep::ok(
                    "hierarchy",
                    format!("{units_created} unit(s) created{extra}"),
                ),
            )
            .await;
        } else {
            push_step_and_publish(
                state.as_ref(),
                &watch_slug,
                &mut steps,
                ProvisionStep::fail("hierarchy", hierarchy_errors.join("; ")),
            )
            .await;
            overall_ok = false;
        }
    }

    // ── Step 7: Top-level role assignments ────────────────────────────────
    if let Some(role_assignments) = &manifest.roles {
        let mut role_errors: Vec<String> = Vec::new();
        let mut roles_ok: usize = 0;
        let now = chrono::Utc::now().timestamp();

        for assignment in role_assignments {
            let user_id = if let Some(id) = mk_core::types::UserId::new(assignment.user_id.clone())
            {
                id
            } else {
                role_errors.push(format!("invalid user_id '{}'", assignment.user_id));
                continue;
            };

            // Resolve unit: if a unit name/id is given look it up; otherwise use
            // the tenant's root by using the tenant_id string as the unit scope.
            let unit_id: String = if let Some(unit_ref) = &assignment.unit {
                match state.postgres.get_unit_scoped(&tenant_ctx, unit_ref).await {
                    Ok(Some(u)) => u.id,
                    Ok(None) => {
                        role_errors.push(format!(
                            "unit '{unit_ref}' not found for user '{}'",
                            assignment.user_id
                        ));
                        continue;
                    }
                    Err(err) => {
                        role_errors.push(format!("unit '{unit_ref}' lookup error: {err}"));
                        continue;
                    }
                }
            } else {
                // No unit specified — scope to tenant root (use tenant_id as unit)
                tenant_id.as_str().to_string()
            };

            match state
                .postgres
                .assign_role_scoped(&tenant_ctx, &user_id, &unit_id, assignment.role.clone())
                .await
            {
                Ok(()) => {
                    persist_governance_event(
                        state.as_ref(),
                        &ctx,
                        &GovernanceEvent::RoleAssigned {
                            user_id: user_id.clone(),
                            unit_id: unit_id.clone(),
                            role: assignment.role.clone(),
                            tenant_id: tenant_id.clone(),
                            timestamp: now,
                        },
                    )
                    .await;
                    roles_ok += 1;
                }
                Err(err) => {
                    role_errors.push(format!(
                        "user '{}' on unit '{unit_id}': {err}",
                        assignment.user_id
                    ));
                }
            }
        }

        if role_errors.is_empty() {
            push_step_and_publish(
                state.as_ref(),
                &watch_slug,
                &mut steps,
                ProvisionStep::ok("roles", format!("{roles_ok} role(s) assigned")),
            )
            .await;
        } else {
            push_step_and_publish(
                state.as_ref(),
                &watch_slug,
                &mut steps,
                ProvisionStep::fail("roles", role_errors.join("; ")),
            )
            .await;
            overall_ok = false;
        }
    }

    // ── Persist manifest state on full success ───────────────────────────
    // Only written when every step succeeded. A partial apply (207) leaves
    // `last_applied_manifest_hash` and `manifest_generation` untouched so
    // the next retry is forced through the full pipeline. This is the
    // conservative choice: better a redundant re-apply than an incorrect
    // short-circuit over partially-applied state.
    //
    // NOTE: there is a narrow TOCTOU window between the
    // `get_manifest_state` read near the top of this handler and this
    // UPDATE. A concurrent apply on the same slug can interleave, so the
    // final state reflects whichever writer committed last. This is
    // acceptable for the platform-admin-only provision path; a future CAS
    // variant can tighten the guarantee if we open this endpoint up.
    //
    // Runs BEFORE the pub/sub broadcast below so that a failed fingerprint
    // persist flips `overall_ok` and is reflected in the broadcast's effect
    // on downstream caches (they will see `partial` and not assume the
    // short-circuit is live).
    if overall_ok {
        if let Err(err) = state
            .tenant_store
            .set_manifest_state(&tenant_record.slug, &incoming_hash, new_generation)
            .await
        {
            // Treat a failure to persist the fingerprint as a step failure
            // so the caller does not assume the short-circuit will work on
            // the next apply. All the body mutations above are already
            // committed; returning 207 here preserves that visibility.
            push_step_and_publish(
                state.as_ref(),
                &watch_slug,
                &mut steps,
                ProvisionStep::fail(
                    "manifest_state",
                    format!("failed to persist manifest fingerprint: {err}"),
                ),
            )
            .await;
            overall_ok = false;
        } else {
            push_step_and_publish(
                state.as_ref(),
                &watch_slug,
                &mut steps,
                ProvisionStep::ok(
                    "manifest_state",
                    format!("hash={incoming_hash} generation={new_generation}"),
                ),
            )
            .await;
        }
    }

    // ── Post-apply: local re-wire + cross-pod broadcast ─────────────────
    //
    // Even a partial apply (`overall_ok == false`) can legitimately change
    // provider config/secrets, so we broadcast on every provision attempt
    // that at minimum ensured the tenant row (we returned early above if
    // that failed). The handler is idempotent; over-broadcasting is
    // cheap. The local `handle_event` call guarantees this pod's caches
    // converge immediately without waiting for the pub/sub round-trip —
    // important because the same HTTP client may hit this pod again on
    // the very next request and deserves to see the new state.
    let change = super::tenant_pubsub::TenantChangeEvent::new(
        tenant_record.slug.clone(),
        super::tenant_pubsub::TenantChangeKind::Provisioned,
    );
    super::tenant_pubsub::handle_event(state.as_ref(), change.clone()).await;
    super::tenant_pubsub::publish(state.as_ref(), &change).await;

    // ── Final response ────────────────────────────────────────────────────
    let status = if overall_ok {
        StatusCode::OK
    } else {
        StatusCode::MULTI_STATUS
    };
    (
        status,
        Json(json!({
            "success": overall_ok,
            "status": if overall_ok { "applied" } else { "partial" },
            "tenantId": tenant_id.as_str(),
            "slug": tenant_record.slug,
            "hash": incoming_hash,
            "generation": new_generation,
            "steps": steps,
        })),
    )
        .into_response()
}

/// `POST /api/v1/admin/tenants/diff` — B3 §2.4.
///
/// Compute a structured delta between the submitted manifest and the
/// tenant's current DB state (rendered via
/// [`manifest_render::render_current_manifest`]). The diff is
/// *read-only*: no rows are written, no audit row is emitted (that's
/// `provision_tenant` with `dryRun=true`'s job, §2.5), and the handler
/// never resolves secrets.
///
/// **Auth:** PlatformAdmin, same gate as `provision_tenant`.
///
/// **Response shape:** [`tenant_diff::TenantDiff`] — see that module
/// for the per-change wire format. CLI renders `-o json` directly
/// and `-o unified` by walking the structured form.
///
/// **Create vs Update vs NoOp:**
/// - Tenant missing → `operation: "create"`, every leaf of the
///   incoming manifest appears as `Added`.
/// - Tenant present, rendered-vs-incoming identical → `NoOp`.
/// - Otherwise → `Update` with the leaf list.
///
/// Unlike `provision_tenant`, this handler does NOT enforce the
/// inline-secret gate — a diff against a manifest with inline
/// plaintext is still a legitimate read operation, and refusing it
/// would force operators to hand-edit their manifests before they
/// can preview. The gate stays on the write path only.
async fn diff_tenant(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(raw): Json<serde_json::Value>,
) -> impl IntoResponse {
    // B2 §10.5 — read-side preview. Either a PlatformAdmin user OR a
    // service token carrying the `tenants:diff` scope is accepted.
    // `_ctx` is `None` on the service-token path because the diff
    // handler emits no audit rows and therefore needs no user
    // attribution. See `docs/security/tenant-provisioning-security.md` §2.
    let _ctx = match require_platform_admin_or_scope(&state, &headers, "tenants:diff").await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    let manifest: TenantManifest = match serde_json::from_value(raw.clone()) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "error": "manifest_parse_failed",
                    "details": e.to_string(),
                })),
            )
                .into_response();
        }
    };

    let validation_errors = validate_manifest(&manifest);
    if !validation_errors.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "success": false,
                "error": "manifest_validation_failed",
                "validationErrors": validation_errors,
            })),
        )
            .into_response();
    }

    let slug = manifest.tenant.slug.clone();

    // Render the current state. `NotFound` → diff against an empty
    // baseline → `operation: "create"`. Any other error is a
    // legitimate 500 — we cannot compute a trustworthy diff without a
    // consistent read of the current state.
    let current_rendered =
        match manifest_render::render_current_manifest(&state, &slug, false).await {
            Ok(rendered) => Some(rendered),
            Err(manifest_render::RenderError::NotFound(_)) => None,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "success": false,
                        "error": "current_state_render_failed",
                        "details": e.to_string(),
                    })),
                )
                    .into_response();
            }
        };

    // Serialize both sides uniformly so the diff walker operates on a
    // single shape. `serde_json::to_value` on known-good types is
    // effectively infallible, but we surface any failure as 500
    // rather than unwrap — a panic here would take the admin API
    // down on a malformed row.
    let current_json = match current_rendered {
        Some(r) => match serde_json::to_value(&r) {
            Ok(v) => Some(v),
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "success": false,
                        "error": "current_state_serialize_failed",
                        "details": e.to_string(),
                    })),
                )
                    .into_response();
            }
        },
        None => None,
    };
    // `TenantManifest` is `Deserialize`-only by design (the input
    // type never needs to re-emit wire JSON; the renderer owns that).
    // We've already validated `raw` by successfully deserializing it
    // into `TenantManifest`, so feeding `raw` directly to the diff
    // walker is sound and avoids requiring a `Serialize` impl on
    // every sub-type of the input schema.
    let diff = tenant_diff::compute_diff(slug, current_json, raw);
    (StatusCode::OK, Json(diff)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::PluginAuthState;

    // -------------------------------------------------------------------
    // B2 §11.4 — audit-actor attribution regression tests
    //
    // These tests pin the contract that the unified audit-actor
    // helpers produce the correct `(actor_type, actor_id, acting_as,
    // envelope)` tuple for both caller kinds, without requiring a
    // Postgres round-trip. A full end-to-end test that writes a
    // service-attributed audit row through `log_audit_with_extensions`
    // lives in the §13 consistency suite.
    // -------------------------------------------------------------------

    #[test]
    fn audit_actor_fields_user_matches_pre_11_4_shape() {
        let user_uuid = uuid::Uuid::new_v4();
        let tenant_uuid = uuid::Uuid::new_v4();
        let ctx = mk_core::types::TenantContext {
            tenant_id: mk_core::TenantId::new(tenant_uuid.to_string()).unwrap(),
            user_id: mk_core::types::UserId::new(user_uuid.to_string()).unwrap(),
            roles: Vec::new(),
            target_tenant_id: None,
            agent_id: None,
        };

        let (kind, actor_id, acting_as, envelope) =
            super::audit_actor_fields(super::AuditActor::User(&ctx));

        assert!(matches!(kind, storage::governance::PrincipalType::User));
        assert_eq!(actor_id, Some(user_uuid));
        // No target_tenant_id → acting_as falls back to actor's own tenant.
        assert_eq!(acting_as, Some(tenant_uuid));
        assert_eq!(envelope["actorKind"], "user");
        assert_eq!(envelope["actorTenantId"], tenant_uuid.to_string());
        assert!(envelope["selectedTargetTenantId"].is_null());
    }

    #[test]
    fn audit_actor_fields_user_with_target_tenant_prefers_target() {
        let user_uuid = uuid::Uuid::new_v4();
        let actor_tenant = uuid::Uuid::new_v4();
        let target_tenant = uuid::Uuid::new_v4();
        let ctx = mk_core::types::TenantContext {
            tenant_id: mk_core::TenantId::new(actor_tenant.to_string()).unwrap(),
            user_id: mk_core::types::UserId::new(user_uuid.to_string()).unwrap(),
            roles: Vec::new(),
            target_tenant_id: Some(mk_core::TenantId::new(target_tenant.to_string()).unwrap()),
            agent_id: None,
        };

        let (_, _, acting_as, envelope) = super::audit_actor_fields(super::AuditActor::User(&ctx));

        // `acting_as_tenant_id` is the filter key for
        // `/govern/audit?tenant=<slug>`; when the caller targeted a
        // different tenant we MUST record the target, not the actor.
        assert_eq!(acting_as, Some(target_tenant));
        assert_eq!(
            envelope["selectedTargetTenantId"],
            target_tenant.to_string()
        );
    }

    #[test]
    fn audit_actor_fields_service_records_agent_attribution() {
        let agent_uuid = uuid::Uuid::new_v4();
        let tenant_uuid = uuid::Uuid::new_v4();
        let principal = crate::server::service_token_validator::ServicePrincipal {
            agent_id: agent_uuid,
            tenant_id: tenant_uuid,
            scopes: vec!["tenants:provision".into()],
        };

        let (kind, actor_id, acting_as, envelope) =
            super::audit_actor_fields(super::AuditActor::Service(&principal));

        // Service tokens are persisted as `agents` rows — the audit
        // log attributes them with `actor_type='agent'` and
        // `actor_id=<agent_uuid>`, NOT user.
        assert!(matches!(kind, storage::governance::PrincipalType::Agent));
        assert_eq!(actor_id, Some(agent_uuid));
        // Service tokens have no impersonation concept; `acting_as` is
        // the service token's own tenant.
        assert_eq!(acting_as, Some(tenant_uuid));
        assert_eq!(envelope["actorKind"], "service");
        assert_eq!(envelope["actorTenantId"], tenant_uuid.to_string());
        assert_eq!(envelope["agentId"], agent_uuid.to_string());
        // Service callers do not set `selectedTargetTenantId`.
        assert!(envelope["selectedTargetTenantId"].is_null());
    }

    #[test]
    fn audit_actor_from_impls_coerce_through_into() {
        // Regression guard: the `impl Into<AuditActor<'_>>` on the
        // audit helpers relies on these `From` impls. If either
        // conversion breaks, every existing call site that passes
        // `&ctx` (not `AuditActor::User(&ctx)`) would stop compiling.
        let ctx = mk_core::types::TenantContext::default();
        let actor: super::AuditActor<'_> = (&ctx).into();
        assert!(matches!(actor, super::AuditActor::User(_)));

        let principal = crate::server::service_token_validator::ServicePrincipal {
            agent_id: uuid::Uuid::new_v4(),
            tenant_id: uuid::Uuid::new_v4(),
            scopes: Vec::new(),
        };
        let actor: super::AuditActor<'_> = (&principal).into();
        assert!(matches!(actor, super::AuditActor::Service(_)));
    }

    #[test]
    fn auth_principal_to_downstream_ctx_service_synthesizes_agent_identity() {
        let agent_uuid = uuid::Uuid::new_v4();
        let principal_tenant = uuid::Uuid::new_v4();
        let target_tenant = uuid::Uuid::new_v4();
        let principal = super::AuthPrincipal::Service(
            crate::server::service_token_validator::ServicePrincipal {
                agent_id: agent_uuid,
                tenant_id: principal_tenant,
                scopes: vec!["tenants:provision".into()],
            },
        );
        let target = mk_core::TenantId::new(target_tenant.to_string()).unwrap();

        let ctx = principal.to_downstream_tenant_context(target.clone());

        // Downstream ctx is scoped to the TARGET tenant (not the
        // principal's own) — org-hierarchy ops target the new tenant.
        assert_eq!(ctx.tenant_id, target);
        assert_eq!(ctx.target_tenant_id, Some(target));
        // `user_id` / `agent_id` both carry the agent UUID so the
        // downstream write path can attribute ownership.
        assert_eq!(ctx.user_id.as_str(), agent_uuid.to_string());
        assert_eq!(
            ctx.agent_id.as_deref(),
            Some(agent_uuid.to_string().as_str())
        );
        // Roles are empty — downstream MUST NOT re-check role for a
        // service caller; scope was already validated at the gate.
        assert!(ctx.roles.is_empty());
    }

    #[test]
    fn auth_principal_to_admin_pool_ctx_user_returns_clone() {
        // User variant: identical to pre-§10.5 `require_platform_admin`
        // ctx. No synthesis, no field changes — the admin pool path
        // sees exactly what the authenticated user supplied.
        let mut original = mk_core::types::TenantContext::default();
        original.tenant_id =
            mk_core::TenantId::new("11111111-1111-1111-1111-111111111111".to_string())
                .expect("uuid string parses as TenantId");
        original.user_id = mk_core::types::UserId::new("the-user".to_string()).unwrap();
        original.roles = vec![Role::PlatformAdmin.into()];
        let principal = super::AuthPrincipal::User(original.clone());

        let ctx = principal.to_admin_pool_context();

        assert_eq!(ctx.tenant_id, original.tenant_id);
        assert_eq!(ctx.user_id, original.user_id);
        assert_eq!(ctx.roles, original.roles);
        assert_eq!(ctx.target_tenant_id, original.target_tenant_id);
        assert_eq!(ctx.agent_id, original.agent_id);
    }

    #[test]
    fn auth_principal_to_admin_pool_ctx_service_binds_principal_tenant() {
        // Service variant binds the SERVICE-TOKEN'S OWN tenant_id —
        // NOT the URL path's tenant_id (which `provision_tenant`,
        // `grant_connection`, etc. read separately). This is what the
        // admin pool's `with_admin_context` records as the *cause*
        // tenant; the *effect* tenant is recorded via the action's
        // `target_id`.
        let agent_uuid = uuid::Uuid::new_v4();
        let principal_tenant = uuid::Uuid::new_v4();
        let principal = super::AuthPrincipal::Service(
            crate::server::service_token_validator::ServicePrincipal {
                agent_id: agent_uuid,
                tenant_id: principal_tenant,
                scopes: vec!["connections:manage".into()],
            },
        );

        let ctx = principal.to_admin_pool_context();

        // tenant bound to the service token's tenant, NOT some target.
        assert_eq!(ctx.tenant_id.as_str(), principal_tenant.to_string());
        // No impersonation — service tokens don't carry that concept.
        assert!(ctx.target_tenant_id.is_none());
        // Stable identity: user_id and agent_id both carry the agent UUID.
        assert_eq!(ctx.user_id.as_str(), agent_uuid.to_string());
        assert_eq!(
            ctx.agent_id.as_deref(),
            Some(agent_uuid.to_string().as_str())
        );
        // Empty roles — scope check at the gate is the auth gate.
        assert!(ctx.roles.is_empty());
    }

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
        KnowledgeEntry, KnowledgeLayer, ReasoningStrategy, ReasoningTrace, UserId,
    };
    use std::path::PathBuf;
    use storage::secret_provider::LocalSecretProvider;
    use storage::tenant_config_provider::KubernetesTenantConfigProvider;
    use storage::tenant_store::{TenantRepositoryBindingStore, TenantStore};
    use sync::bridge::SyncManager;
    use sync::state_persister::FilePersister;
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

    struct MockRepo;

    #[async_trait]
    impl KnowledgeRepository for MockRepo {
        type Error = RepositoryError;

        async fn get(
            &self,
            _ctx: TenantContext,
            _layer: KnowledgeLayer,
            _path: &str,
        ) -> Result<Option<KnowledgeEntry>, Self::Error> {
            Ok(None)
        }

        async fn store(
            &self,
            _ctx: TenantContext,
            _entry: KnowledgeEntry,
            _message: &str,
        ) -> Result<String, Self::Error> {
            Ok("mock-commit".to_string())
        }

        async fn list(
            &self,
            _ctx: TenantContext,
            _layer: KnowledgeLayer,
            _prefix: &str,
        ) -> Result<Vec<KnowledgeEntry>, Self::Error> {
            Ok(Vec::new())
        }

        async fn delete(
            &self,
            _ctx: TenantContext,
            _layer: KnowledgeLayer,
            _path: &str,
            _message: &str,
        ) -> Result<String, Self::Error> {
            Ok("mock-commit".to_string())
        }

        async fn get_head_commit(
            &self,
            _ctx: TenantContext,
        ) -> Result<Option<String>, Self::Error> {
            Ok(None)
        }

        async fn get_affected_items(
            &self,
            _ctx: TenantContext,
            _since_commit: &str,
        ) -> Result<Vec<(KnowledgeLayer, String)>, Self::Error> {
            Ok(Vec::new())
        }

        async fn search(
            &self,
            _ctx: TenantContext,
            _query: &str,
            _layers: Vec<KnowledgeLayer>,
            _limit: usize,
        ) -> Result<Vec<KnowledgeEntry>, Self::Error> {
            Ok(Vec::new())
        }

        fn root_path(&self) -> Option<PathBuf> {
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
                thought_process: "tenant_api test noop".to_string(),
                refined_query: Some(query.to_string()),
                start_time: now,
                end_time: now,
                timed_out: false,
                duration_ms: 0,
                metadata: HashMap::new(),
            })
        }
    }

    async fn app_with_tenant() -> Option<(Router, mk_core::types::TenantRecord)> {
        let fixture = postgres().await?;
        let postgres = Arc::new(
            storage::postgres::PostgresBackend::new(fixture.url())
                .await
                .ok()?,
        );
        postgres.initialize_schema().await.ok()?;

        let auth_service: Arc<dyn AuthorizationService<Error = anyhow::Error> + Send + Sync> =
            Arc::new(MockAuth);
        let git_repo = Arc::new(
            knowledge::repository::GitRepository::new(tempfile::tempdir().ok()?.path()).ok()?,
        );
        let governance_engine = Arc::new(GovernanceEngine::new());
        let knowledge_manager = Arc::new(KnowledgeManager::new(
            git_repo.clone(),
            governance_engine.clone(),
        ));
        let memory_manager = Arc::new(MemoryManager::new());
        let sync_manager = Arc::new(
            SyncManager::new(
                memory_manager.clone(),
                knowledge_manager.clone(),
                config::config::DeploymentConfig::default(),
                None,
                Arc::new(FilePersister::new(std::env::temp_dir())),
                None,
            )
            .await
            .ok()?,
        );
        let governance_dashboard = Arc::new(GovernanceDashboardApi::new(
            governance_engine.clone(),
            postgres.clone(),
            config::config::DeploymentConfig::default(),
        ));
        let mcp_server = Arc::new(McpServer::new(
            memory_manager.clone(),
            sync_manager.clone(),
            knowledge_manager.clone(),
            git_repo.clone(),
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
        let tenant_config_provider = Arc::new(
            KubernetesTenantConfigProvider::new_in_memory_for_tests("default".to_string()),
        );
        let tenant = tenant_store.create_tenant("acme", "Acme Corp").await.ok()?;

        let state = Arc::new(AppState {
            config: Arc::new(config::Config::default()),
            postgres: postgres.clone(),
            memory_manager,
            knowledge_manager,
            knowledge_repository: Arc::new(MockRepo),
            governance_engine,
            governance_dashboard,
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
            tenant_config_provider,
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
        });

        Some((router(state), tenant))
    }

    fn request_with_headers(method: &str, uri: &str, role: &str, body: Body) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", role)
            .header("x-tenant-id", "default")
            .body(body)
            .unwrap()
    }

    #[tokio::test]
    async fn tenant_config_happy_path_and_secret_redaction() {
        let Some((app, tenant)) = app_with_tenant().await else {
            eprintln!("Skipping tenant_api test: Docker not available");
            return;
        };

        let upsert_config = serde_json::json!({
            "fields": {
                "runtime.logLevel": {
                    "ownership": "Tenant",
                    "value": "info"
                }
            },
            "secretReferences": {}
        });
        let upsert_response = app
            .clone()
            .oneshot(request_with_headers(
                "PUT",
                &format!("/admin/tenants/{}/config", tenant.id.as_str()),
                "platformAdmin",
                Body::from(serde_json::to_vec(&upsert_config).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(upsert_response.status(), StatusCode::OK);

        let secret_value = "super-secret-value";
        let set_secret_response = app
            .clone()
            .oneshot(request_with_headers(
                "PUT",
                &format!("/admin/tenants/{}/secrets/repo.token", tenant.id.as_str()),
                "platformAdmin",
                Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "ownership": "Tenant",
                        "secretValue": secret_value
                    }))
                    .unwrap(),
                ),
            ))
            .await
            .unwrap();
        assert_eq!(set_secret_response.status(), StatusCode::OK);
        let set_body = axum::body::to_bytes(set_secret_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let set_json: serde_json::Value = serde_json::from_slice(&set_body).unwrap();
        assert_eq!(set_json["secretReference"]["logicalName"], "repo.token");
        assert!(!String::from_utf8_lossy(&set_body).contains(secret_value));

        let inspect_response = app
            .clone()
            .oneshot(request_with_headers(
                "GET",
                &format!("/admin/tenants/{}/config", tenant.id.as_str()),
                "platformAdmin",
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(inspect_response.status(), StatusCode::OK);
        let inspect_body = axum::body::to_bytes(inspect_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let inspect_text = String::from_utf8_lossy(&inspect_body);
        assert!(!inspect_text.contains(secret_value));
        let inspect_json: serde_json::Value = serde_json::from_slice(&inspect_body).unwrap();
        assert_eq!(
            inspect_json["config"]["fields"]["runtime.logLevel"]["value"],
            "info"
        );
        assert_eq!(
            inspect_json["config"]["secretReferences"]["repo.token"]["secretKey"],
            "repo.token"
        );

        let delete_response = app
            .oneshot(request_with_headers(
                "DELETE",
                &format!("/admin/tenants/{}/secrets/repo.token", tenant.id.as_str()),
                "platformAdmin",
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(delete_response.status(), StatusCode::OK);
        let delete_body = axum::body::to_bytes(delete_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let delete_json: serde_json::Value = serde_json::from_slice(&delete_body).unwrap();
        assert_eq!(delete_json["deleted"], true);
    }

    #[tokio::test]
    async fn tenant_config_validate_rejects_raw_secret_material() {
        let Some((app, tenant)) = app_with_tenant().await else {
            eprintln!("Skipping tenant_api test: Docker not available");
            return;
        };

        let response = app
            .oneshot(request_with_headers(
                "POST",
                &format!("/admin/tenants/{}/config/validate", tenant.id.as_str()),
                "platformAdmin",
                Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "fields": {
                            "database.password": {
                                "ownership": "Tenant",
                                "value": "plain-text-secret"
                            }
                        },
                        "secretReferences": {}
                    }))
                    .unwrap(),
                ),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["valid"], false);
        assert_eq!(json["error"], "tenant_config_invalid");
    }

    #[tokio::test]
    async fn tenant_admin_secret_mutation_rejects_platform_owned_entries() {
        let Some((app, tenant)) = app_with_tenant().await else {
            eprintln!("Skipping tenant_api test: Docker not available");
            return;
        };

        let request = Request::builder()
            .method("PUT")
            .uri("/admin/tenant-config/secrets/platform.token")
            .header("content-type", "application/json")
            .header("x-user-id", "tenant-admin-user")
            .header("x-user-role", "admin")
            .header("x-tenant-id", "default")
            .header("x-target-tenant-id", tenant.id.as_str())
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "ownership": "Platform",
                    "secretValue": "secret"
                }))
                .unwrap(),
            ))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn default_tenant_secret_ownership_is_tenant() {
        let request: SetTenantSecretRequest = serde_json::from_value(serde_json::json!({
            "secretValue": "value"
        }))
        .unwrap();
        assert_eq!(request.ownership, TenantConfigOwnership::Tenant);
    }

    #[test]
    fn tenant_config_request_deserializes_with_camel_case() {
        let request: UpsertTenantConfigRequest = serde_json::from_value(serde_json::json!({
            "fields": {
                "runtime.logLevel": {
                    "ownership": "Tenant",
                    "value": "info"
                }
            },
            "secretReferences": {
                "repo.token": {
                    "logicalName": "repo.token",
                    "ownership": "Tenant",
                    "kind": "postgres",
                    "secretId": "22222222-2222-2222-2222-222222222222"
                }
            }
        }))
        .unwrap();

        assert_eq!(request.fields.len(), 1);
        assert_eq!(request.secret_references.len(), 1);
    }

    #[test]
    fn tenant_admin_guard_rejects_platform_owned_fields() {
        let tenant_id =
            mk_core::types::TenantId::new("11111111-1111-1111-1111-111111111111".to_string())
                .unwrap();
        let mut fields = BTreeMap::new();
        fields.insert(
            "platform.control".to_string(),
            TenantConfigField {
                ownership: TenantConfigOwnership::Platform,
                value: serde_json::json!("x"),
            },
        );

        let document = TenantConfigDocument {
            tenant_id,
            fields,
            secret_references: BTreeMap::new(),
        };

        let response = reject_non_tenant_owned_config(&document).unwrap_err();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn map_validation_error_to_unprocessable_entity() {
        let response = map_tenant_config_provider_error(
            "validate",
            TenantConfigProviderError::Validation("broken".to_string()),
        );
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn tenant_admin_cannot_target_different_tenant() {
        let Some((app, tenant)) = app_with_tenant().await else {
            eprintln!("Skipping tenant_api test: Docker not available");
            return;
        };

        // A second tenant to try to cross into
        let second_tenant_id = tenant.id.as_str().to_string();

        // TenantAdmin whose own tenant is "default" tries to inspect another tenant's config
        let request = Request::builder()
            .method("GET")
            .uri("/admin/tenant-config")
            .header("x-user-id", "tenant-admin-user")
            .header("x-user-role", "admin")
            .header("x-tenant-id", "default")
            .header("x-target-tenant-id", second_tenant_id.as_str())
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        // "default" != second_tenant_id, so FORBIDDEN
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "forbidden");
    }

    #[tokio::test]
    async fn platform_admin_can_target_any_tenant() {
        let Some((app, tenant)) = app_with_tenant().await else {
            eprintln!("Skipping tenant_api test: Docker not available");
            return;
        };

        // PlatformAdmin is allowed to target any tenant
        let request = Request::builder()
            .method("GET")
            .uri("/admin/tenant-config")
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", "platformAdmin")
            .header("x-tenant-id", "default")
            .header("x-target-tenant-id", tenant.id.as_str())
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        // No config stored yet — 200 with null config is acceptable
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn tenant_admin_cannot_delete_platform_owned_secret() {
        let Some((app, tenant)) = app_with_tenant().await else {
            eprintln!("Skipping tenant_api test: Docker not available");
            return;
        };

        // First: GlobalAdmin plants a platform-owned secret via the global route
        let set_request = Request::builder()
            .method("PUT")
            .uri(format!(
                "/admin/tenants/{}/secrets/platform.token",
                tenant.id.as_str()
            ))
            .header("content-type", "application/json")
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", "platformAdmin")
            .header("x-tenant-id", "default")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "ownership": "Platform",
                    "secretValue": "platform-secret"
                }))
                .unwrap(),
            ))
            .unwrap();
        let set_response = app.clone().oneshot(set_request).await.unwrap();
        assert_eq!(set_response.status(), StatusCode::OK);

        // Now: TenantAdmin tries to delete it via the self-scoped route
        let delete_request = Request::builder()
            .method("DELETE")
            .uri("/admin/tenant-config/secrets/platform.token")
            .header("x-user-id", "tenant-admin-user")
            .header("x-user-role", "admin")
            .header("x-tenant-id", tenant.id.as_str())
            .body(Body::empty())
            .unwrap();

        let delete_response = app.oneshot(delete_request).await.unwrap();
        assert_eq!(delete_response.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(delete_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "forbidden");
    }

    #[tokio::test]
    async fn tenant_admin_validate_rejects_platform_owned_fields() {
        let Some((app, tenant)) = app_with_tenant().await else {
            eprintln!("Skipping tenant_api test: Docker not available");
            return;
        };

        let request = Request::builder()
            .method("POST")
            .uri("/admin/tenant-config/validate")
            .header("content-type", "application/json")
            .header("x-user-id", "tenant-admin-user")
            .header("x-user-role", "admin")
            .header("x-tenant-id", tenant.id.as_str())
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "fields": {
                        "platform.control": {
                            "ownership": "Platform",
                            "value": "blocked"
                        }
                    },
                    "secretReferences": {}
                }))
                .unwrap(),
            ))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn validate_my_tenant_config_rejects_unauthenticated() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping tenant_api test: Docker not available");
            return;
        };

        // No role header → require_tenant_admin_context must return FORBIDDEN
        let request = Request::builder()
            .method("POST")
            .uri("/admin/tenant-config/validate")
            .header("content-type", "application/json")
            .header("x-user-id", "anonymous")
            .header("x-tenant-id", "default")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "fields": {},
                    "secretReferences": {}
                }))
                .unwrap(),
            ))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    // -----------------------------------------------------------------------
    // Task 4.3: End-to-end coverage — tenant config provisioning, secret
    // administration, deployment materialization, and tenant bootstrap flows.
    // -----------------------------------------------------------------------

    // Helper: create a Git provider connection and return its id.
    async fn create_connection(app: axum::Router, name: &str) -> String {
        create_connection_with_id(app, name, None).await
    }

    async fn create_connection_with_id(app: axum::Router, name: &str, id: Option<&str>) -> String {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;
        let req = Request::builder()
            .method("POST")
            .uri("/admin/git-provider-connections")
            .header("content-type", "application/json")
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", "platformAdmin")
            .header("x-tenant-id", "default")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "id": id,
                    "name": name,
                    "providerKind": "GitHubApp",
                    "appId": 123456u64,
                    "installationId": 9876543u64,
                    "pemSecretRef": "secret/aeterna-github-app-pem/pem-key",
                    "webhookSecretRef": null
                }))
                .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::CREATED,
            "connection create should return 201"
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        json["connection"]["id"]
            .as_str()
            .expect("connection id should be present")
            .to_string()
    }

    #[tokio::test]
    async fn git_provider_connection_create_and_list() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping: Docker not available");
            return;
        };

        // Create two connections
        let id1 = create_connection(app.clone(), "Acme GitHub App").await;
        let id2 = create_connection(app.clone(), "Platform GitHub App").await;
        assert_ne!(id1, id2, "connection ids must be unique");

        // List connections
        let list_req = Request::builder()
            .method("GET")
            .uri("/admin/git-provider-connections")
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", "platformAdmin")
            .header("x-tenant-id", "default")
            .body(Body::empty())
            .unwrap();
        let list_resp = app.oneshot(list_req).await.unwrap();
        assert_eq!(list_resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(list_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let connections = json["connections"].as_array().unwrap();
        assert!(
            connections.len() >= 2,
            "should list at least 2 connections, got {}",
            connections.len()
        );
        // PEM refs must be redacted (REDACTED token) in list output
        for conn in connections {
            assert!(
                conn["pemSecretRef"]
                    .as_str()
                    .unwrap_or("")
                    .contains("REDACTED"),
                "pemSecretRef must be redacted in list response"
            );
        }
    }

    #[tokio::test]
    async fn git_provider_connection_create_honors_explicit_id() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping: Docker not available");
            return;
        };

        let id = create_connection_with_id(
            app.clone(),
            "Explicit ID Connection",
            Some("shared-github-app"),
        )
        .await;
        assert_eq!(id, "shared-github-app");
    }

    #[tokio::test]
    async fn git_provider_connection_create_rejects_invalid_explicit_id() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping: Docker not available");
            return;
        };

        let req = Request::builder()
            .method("POST")
            .uri("/admin/git-provider-connections")
            .header("content-type", "application/json")
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", "platformAdmin")
            .header("x-tenant-id", "default")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "id": "Bad_Id",
                    "name": "Invalid Connection",
                    "providerKind": "GitHubApp",
                    "appId": 123456u64,
                    "installationId": 9876543u64,
                    "pemSecretRef": "secret/aeterna-github-app-pem/pem-key",
                    "webhookSecretRef": null
                }))
                .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn git_provider_connection_create_rejects_duplicate_explicit_id() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping: Docker not available");
            return;
        };

        let id = create_connection_with_id(app.clone(), "Original", Some("shared-gh-app")).await;
        assert_eq!(id, "shared-gh-app");

        let req = Request::builder()
            .method("POST")
            .uri("/admin/git-provider-connections")
            .header("content-type", "application/json")
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", "platformAdmin")
            .header("x-tenant-id", "default")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "id": "shared-gh-app",
                    "name": "Duplicate",
                    "providerKind": "GitHubApp",
                    "appId": 123456u64,
                    "installationId": 9876543u64,
                    "pemSecretRef": "secret/aeterna-github-app-pem/pem-key",
                    "webhookSecretRef": null
                }))
                .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn git_provider_connection_show_redacts_pem() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping: Docker not available");
            return;
        };
        let conn_id = create_connection(app.clone(), "Show Redaction Test").await;

        let show_req = Request::builder()
            .method("GET")
            .uri(format!("/admin/git-provider-connections/{conn_id}"))
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", "platformAdmin")
            .header("x-tenant-id", "default")
            .body(Body::empty())
            .unwrap();
        let show_resp = app.oneshot(show_req).await.unwrap();
        assert_eq!(show_resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(show_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // PEM ref must be redacted
        assert!(
            json["connection"]["pemSecretRef"]
                .as_str()
                .unwrap_or("")
                .contains("REDACTED"),
            "show response must redact pemSecretRef"
        );
        // Raw PEM material must never appear
        assert!(
            !String::from_utf8_lossy(&body).contains("pem-key"),
            "raw pem key reference must not appear in show response"
        );
    }

    #[tokio::test]
    async fn git_provider_connection_show_404_for_unknown() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping: Docker not available");
            return;
        };
        let show_req = Request::builder()
            .method("GET")
            .uri("/admin/git-provider-connections/does-not-exist")
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", "platformAdmin")
            .header("x-tenant-id", "default")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(show_req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn git_provider_connection_create_denied_for_non_platform_admin() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping: Docker not available");
            return;
        };
        let req = Request::builder()
            .method("POST")
            .uri("/admin/git-provider-connections")
            .header("content-type", "application/json")
            .header("x-user-id", "tenant-admin-user")
            .header("x-user-role", "admin")
            .header("x-tenant-id", "default")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "name": "Unauthorized",
                    "providerKind": "GitHubApp",
                    "appId": 1u64,
                    "installationId": 2u64,
                    "pemSecretRef": "secret/foo/pem-key"
                }))
                .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "non-platform-admin must be denied connection creation"
        );
    }

    #[tokio::test]
    async fn git_provider_connection_grant_and_revoke_tenant_visibility() {
        let Some((app, tenant)) = app_with_tenant().await else {
            eprintln!("Skipping: Docker not available");
            return;
        };
        let conn_id = create_connection(app.clone(), "Grant-Revoke Test").await;
        let tid = tenant.id.as_str().to_string();

        // Initially tenant cannot see connection
        let list_before = Request::builder()
            .method("GET")
            .uri(format!("/admin/tenants/{tid}/git-provider-connections"))
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", "platformAdmin")
            .header("x-tenant-id", "default")
            .body(Body::empty())
            .unwrap();
        let resp_before = app.clone().oneshot(list_before).await.unwrap();
        assert_eq!(resp_before.status(), StatusCode::OK);
        let body_before = axum::body::to_bytes(resp_before.into_body(), usize::MAX)
            .await
            .unwrap();
        let json_before: serde_json::Value = serde_json::from_slice(&body_before).unwrap();
        assert_eq!(
            json_before["connections"].as_array().unwrap().len(),
            0,
            "tenant should see 0 connections before grant"
        );

        // Grant visibility
        let grant_req = Request::builder()
            .method("POST")
            .uri(format!(
                "/admin/git-provider-connections/{conn_id}/tenants/{tid}"
            ))
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", "platformAdmin")
            .header("x-tenant-id", "default")
            .body(Body::empty())
            .unwrap();
        let grant_resp = app.clone().oneshot(grant_req).await.unwrap();
        assert_eq!(
            grant_resp.status(),
            StatusCode::OK,
            "grant should return 200"
        );

        // Tenant now sees the connection
        let list_after = Request::builder()
            .method("GET")
            .uri(format!("/admin/tenants/{tid}/git-provider-connections"))
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", "platformAdmin")
            .header("x-tenant-id", "default")
            .body(Body::empty())
            .unwrap();
        let resp_after = app.clone().oneshot(list_after).await.unwrap();
        assert_eq!(resp_after.status(), StatusCode::OK);
        let body_after = axum::body::to_bytes(resp_after.into_body(), usize::MAX)
            .await
            .unwrap();
        let json_after: serde_json::Value = serde_json::from_slice(&body_after).unwrap();
        let conns = json_after["connections"].as_array().unwrap();
        assert_eq!(conns.len(), 1, "tenant should see 1 connection after grant");
        assert_eq!(
            conns[0]["id"].as_str().unwrap(),
            conn_id,
            "granted connection id must match"
        );

        // Revoke visibility
        let revoke_req = Request::builder()
            .method("DELETE")
            .uri(format!(
                "/admin/git-provider-connections/{conn_id}/tenants/{tid}"
            ))
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", "platformAdmin")
            .header("x-tenant-id", "default")
            .body(Body::empty())
            .unwrap();
        let revoke_resp = app.clone().oneshot(revoke_req).await.unwrap();
        assert_eq!(
            revoke_resp.status(),
            StatusCode::OK,
            "revoke should return 200"
        );

        // Tenant no longer sees the connection
        let list_final = Request::builder()
            .method("GET")
            .uri(format!("/admin/tenants/{tid}/git-provider-connections"))
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", "platformAdmin")
            .header("x-tenant-id", "default")
            .body(Body::empty())
            .unwrap();
        let resp_final = app.oneshot(list_final).await.unwrap();
        assert_eq!(resp_final.status(), StatusCode::OK);
        let body_final = axum::body::to_bytes(resp_final.into_body(), usize::MAX)
            .await
            .unwrap();
        let json_final: serde_json::Value = serde_json::from_slice(&body_final).unwrap();
        assert_eq!(
            json_final["connections"].as_array().unwrap().len(),
            0,
            "tenant should see 0 connections after revoke"
        );
    }

    #[tokio::test]
    async fn set_repository_binding_rejects_unvisible_connection() {
        let Some((app, tenant)) = app_with_tenant().await else {
            eprintln!("Skipping: Docker not available");
            return;
        };
        let tid = tenant.id.as_str().to_string();

        // Create a connection but do NOT grant it to the tenant
        let conn_id = create_connection(app.clone(), "Ungranted Connection").await;

        // Tenant binding with ungranted connection id must be rejected
        let bind_req = Request::builder()
            .method("POST")
            .uri(format!("/admin/tenants/{tid}/repository-binding"))
            .header("content-type", "application/json")
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", "platformAdmin")
            .header("x-tenant-id", "default")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "repoUrl": "https://github.com/acme/repo.git",
                    "credentialRef": null,
                    "gitProviderConnectionId": conn_id
                }))
                .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(bind_req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "binding with ungranted connection must be rejected with 400"
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            json["error"]
                .as_str()
                .unwrap_or("")
                .contains("git_provider_connection"),
            "error should indicate connection visibility problem, got: {json}"
        );
    }

    #[tokio::test]
    async fn set_repository_binding_accepts_granted_connection() {
        let Some((app, tenant)) = app_with_tenant().await else {
            eprintln!("Skipping: Docker not available");
            return;
        };
        let tid = tenant.id.as_str().to_string();

        // Create and grant a connection
        let conn_id = create_connection(app.clone(), "Granted Connection").await;
        let grant_req = Request::builder()
            .method("POST")
            .uri(format!(
                "/admin/git-provider-connections/{conn_id}/tenants/{tid}"
            ))
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", "platformAdmin")
            .header("x-tenant-id", "default")
            .body(Body::empty())
            .unwrap();
        let grant_resp = app.clone().oneshot(grant_req).await.unwrap();
        assert_eq!(grant_resp.status(), StatusCode::OK);

        // Now the binding should succeed
        let bind_req = Request::builder()
            .method("POST")
            .uri(format!("/admin/tenants/{tid}/repository-binding"))
            .header("content-type", "application/json")
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", "platformAdmin")
            .header("x-tenant-id", "default")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "repoUrl": "https://github.com/acme/repo.git",
                    "credentialRef": null,
                    "gitProviderConnectionId": conn_id
                }))
                .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(bind_req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "binding with granted connection must succeed"
        );
    }

    #[tokio::test]
    async fn full_tenant_bootstrap_flow() {
        // End-to-end: provision tenant config, set secret, inspect (redacted),
        // create + grant git provider connection, then verify tenant-side list.
        let Some((app, tenant)) = app_with_tenant().await else {
            eprintln!("Skipping: Docker not available");
            return;
        };
        let tid = tenant.id.as_str().to_string();

        // 1. GlobalAdmin upserts tenant config (deployment materialization step)
        let config_body = serde_json::json!({
            "fields": {
                "runtime.logLevel": { "ownership": "Tenant", "value": "warn" },
                "deployment.namespace": { "ownership": "Platform", "value": "aeterna-prod" }
            },
            "secretReferences": {}
        });
        let upsert = app
            .clone()
            .oneshot(request_with_headers(
                "PUT",
                &format!("/admin/tenants/{tid}/config"),
                "platformAdmin",
                Body::from(serde_json::to_vec(&config_body).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(
            upsert.status(),
            StatusCode::OK,
            "config upsert must succeed"
        );

        // 2. GlobalAdmin provisions a secret reference
        let set_secret = app
            .clone()
            .oneshot(request_with_headers(
                "PUT",
                &format!("/admin/tenants/{tid}/secrets/github.token"),
                "platformAdmin",
                Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "ownership": "Platform",
                        "secretValue": "ghp_bootstrap_secret_value"
                    }))
                    .unwrap(),
                ),
            ))
            .await
            .unwrap();
        assert_eq!(
            set_secret.status(),
            StatusCode::OK,
            "secret set must succeed"
        );
        let secret_body = axum::body::to_bytes(set_secret.into_body(), usize::MAX)
            .await
            .unwrap();
        // Secret value must never appear in response
        assert!(
            !String::from_utf8_lossy(&secret_body).contains("ghp_bootstrap_secret_value"),
            "secret value must not appear in set-secret response"
        );

        // 3. Inspect tenant config — must be fully redacted
        let inspect = app
            .clone()
            .oneshot(request_with_headers(
                "GET",
                &format!("/admin/tenants/{tid}/config"),
                "platformAdmin",
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(inspect.status(), StatusCode::OK);
        let inspect_body = axum::body::to_bytes(inspect.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&inspect_body).contains("ghp_bootstrap_secret_value"),
            "secret value must not appear in config inspect response"
        );
        let inspect_json: serde_json::Value = serde_json::from_slice(&inspect_body).unwrap();
        assert_eq!(
            inspect_json["config"]["fields"]["runtime.logLevel"]["value"], "warn",
            "tenant config field must be preserved"
        );
        assert!(
            inspect_json["config"]["secretReferences"]
                .get("github.token")
                .is_some(),
            "secret reference must be present in config inspect"
        );

        // 4. Platform admin creates + grants Git provider connection
        let conn_id = create_connection(app.clone(), "Bootstrap Test Connection").await;
        let grant = Request::builder()
            .method("POST")
            .uri(format!(
                "/admin/git-provider-connections/{conn_id}/tenants/{tid}"
            ))
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", "platformAdmin")
            .header("x-tenant-id", "default")
            .body(Body::empty())
            .unwrap();
        let grant_resp = app.clone().oneshot(grant).await.unwrap();
        assert_eq!(grant_resp.status(), StatusCode::OK, "grant must succeed");

        // 5. Verify tenant-side connection list reflects the grant
        let tenant_conns = Request::builder()
            .method("GET")
            .uri(format!("/admin/tenants/{tid}/git-provider-connections"))
            .header("x-user-id", "platform-admin-user")
            .header("x-user-role", "platformAdmin")
            .header("x-tenant-id", "default")
            .body(Body::empty())
            .unwrap();
        let tenant_conns_resp = app.oneshot(tenant_conns).await.unwrap();
        assert_eq!(tenant_conns_resp.status(), StatusCode::OK);
        let tc_body = axum::body::to_bytes(tenant_conns_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let tc_json: serde_json::Value = serde_json::from_slice(&tc_body).unwrap();
        let visible = tc_json["connections"].as_array().unwrap();
        assert_eq!(
            visible.len(),
            1,
            "bootstrap flow: tenant should see exactly 1 connection"
        );
        assert_eq!(
            visible[0]["id"].as_str().unwrap(),
            conn_id,
            "visible connection id must match the granted one"
        );
    }

    #[test]
    fn validate_manifest_accepts_valid_minimal() {
        let m = TenantManifest {
            api_version: "aeterna.io/v1".into(),
            kind: "TenantManifest".into(),
            metadata: None,
            providers: None,
            tenant: ManifestTenant {
                slug: "my-tenant".into(),
                name: "My Tenant".into(),
                account_ref: None,
                environment: None,
                domain_mappings: None,
            },
            config: None,
            secrets: None,
            repository: None,
            hierarchy: None,
            roles: None,
        };
        let errors = validate_manifest(&m);
        assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    }

    #[test]
    fn validate_manifest_rejects_bad_api_version() {
        let m = TenantManifest {
            api_version: "v2".into(),
            kind: "TenantManifest".into(),
            metadata: None,
            providers: None,
            tenant: ManifestTenant {
                slug: "ok".into(),
                name: "Ok".into(),
                account_ref: None,
                environment: None,
                domain_mappings: None,
            },
            config: None,
            secrets: None,
            repository: None,
            hierarchy: None,
            roles: None,
        };
        let errors = validate_manifest(&m);
        assert!(
            errors.iter().any(|e| e.contains("apiVersion")),
            "expected apiVersion error, got: {errors:?}"
        );
    }

    #[test]
    fn validate_manifest_rejects_bad_kind() {
        let m = TenantManifest {
            api_version: "aeterna.io/v1".into(),
            kind: "WrongKind".into(),
            metadata: None,
            providers: None,
            tenant: ManifestTenant {
                slug: "ok".into(),
                name: "Ok".into(),
                account_ref: None,
                environment: None,
                domain_mappings: None,
            },
            config: None,
            secrets: None,
            repository: None,
            hierarchy: None,
            roles: None,
        };
        let errors = validate_manifest(&m);
        assert!(
            errors.iter().any(|e| e.contains("kind")),
            "expected kind error, got: {errors:?}"
        );
    }

    #[test]
    fn validate_manifest_rejects_empty_slug() {
        let m = TenantManifest {
            api_version: "aeterna.io/v1".into(),
            kind: "TenantManifest".into(),
            metadata: None,
            providers: None,
            tenant: ManifestTenant {
                slug: String::new(),
                name: "Ok".into(),
                account_ref: None,
                environment: None,
                domain_mappings: None,
            },
            config: None,
            secrets: None,
            repository: None,
            hierarchy: None,
            roles: None,
        };
        let errors = validate_manifest(&m);
        assert!(
            errors.iter().any(|e| e.contains("slug")),
            "expected slug error, got: {errors:?}"
        );
    }

    #[test]
    fn validate_manifest_rejects_non_kebab_slug() {
        let m = TenantManifest {
            api_version: "aeterna.io/v1".into(),
            kind: "TenantManifest".into(),
            metadata: None,
            providers: None,
            tenant: ManifestTenant {
                slug: "My_Tenant".into(),
                name: "My Tenant".into(),
                account_ref: None,
                environment: None,
                domain_mappings: None,
            },
            config: None,
            secrets: None,
            repository: None,
            hierarchy: None,
            roles: None,
        };
        let errors = validate_manifest(&m);
        assert!(
            errors.iter().any(|e| e.contains("kebab-case")),
            "expected kebab-case error, got: {errors:?}"
        );
    }

    #[test]
    fn validate_manifest_rejects_slug_leading_hyphen() {
        let m = TenantManifest {
            api_version: "aeterna.io/v1".into(),
            kind: "TenantManifest".into(),
            metadata: None,
            providers: None,
            tenant: ManifestTenant {
                slug: "-leading".into(),
                name: "Ok".into(),
                account_ref: None,
                environment: None,
                domain_mappings: None,
            },
            config: None,
            secrets: None,
            repository: None,
            hierarchy: None,
            roles: None,
        };
        let errors = validate_manifest(&m);
        assert!(!errors.is_empty(), "expected slug error for leading hyphen");
    }

    #[test]
    fn validate_manifest_rejects_empty_name() {
        let m = TenantManifest {
            api_version: "aeterna.io/v1".into(),
            kind: "TenantManifest".into(),
            metadata: None,
            providers: None,
            tenant: ManifestTenant {
                slug: "ok".into(),
                name: "   ".into(),
                account_ref: None,
                environment: None,
                domain_mappings: None,
            },
            config: None,
            secrets: None,
            repository: None,
            hierarchy: None,
            roles: None,
        };
        let errors = validate_manifest(&m);
        assert!(
            errors.iter().any(|e| e.contains("name")),
            "expected name error, got: {errors:?}"
        );
    }

    #[test]
    fn validate_manifest_rejects_platform_admin_in_roles() {
        let m = TenantManifest {
            api_version: "aeterna.io/v1".into(),
            kind: "TenantManifest".into(),
            metadata: None,
            providers: None,
            tenant: ManifestTenant {
                slug: "ok".into(),
                name: "Ok".into(),
                account_ref: None,
                environment: None,
                domain_mappings: None,
            },
            config: None,
            secrets: None,
            repository: None,
            hierarchy: None,
            roles: Some(vec![ManifestRoleAssignment {
                user_id: "alice".into(),
                role: Role::PlatformAdmin.into(),
                unit: None,
            }]),
        };
        let errors = validate_manifest(&m);
        assert!(
            errors.iter().any(|e| e.contains("PlatformAdmin")),
            "expected PlatformAdmin error, got: {errors:?}"
        );
    }

    #[test]
    fn validate_manifest_rejects_empty_hierarchy_names() {
        let m = TenantManifest {
            api_version: "aeterna.io/v1".into(),
            kind: "TenantManifest".into(),
            metadata: None,
            providers: None,
            tenant: ManifestTenant {
                slug: "ok".into(),
                name: "Ok".into(),
                account_ref: None,
                environment: None,
                domain_mappings: None,
            },
            config: None,
            secrets: None,
            repository: None,
            hierarchy: Some(vec![ManifestOrg {
                name: String::new(),
                teams: Some(vec![ManifestTeam {
                    name: String::new(),
                    members: None,
                }]),
                members: None,
            }]),
            roles: None,
        };
        let errors = validate_manifest(&m);
        assert!(
            errors
                .iter()
                .any(|e| e == "hierarchy org name must not be empty"),
            "expected org-name validation error, got: {errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|e| e == "hierarchy team name must not be empty"),
            "expected team-name validation error, got: {errors:?}"
        );
    }

    #[tokio::test]
    async fn provision_tenant_happy_path() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping provision test: Docker not available");
            return;
        };

        let manifest = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": {
                "slug": "provision-test",
                "name": "Provision Test Tenant"
            }
        });

        let response = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision",
                "platformAdmin",
                Body::from(serde_json::to_vec(&manifest).unwrap()),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
        assert!(
            json["tenantId"].as_str().is_some(),
            "tenantId must be present"
        );
        assert_eq!(json["slug"], "provision-test");
    }

    // ─── inline-secret gate (B2 tasks 4.1 / 4.2 / 4.3) ────────────────────

    /// Default config + no `allowInline=true` query flag → a manifest
    /// carrying non-empty `secrets[].secretValue` is rejected with 422.
    /// The error surface must (1) use the stable error code
    /// `inline_secret_not_allowed`, (2) enumerate the offending logical
    /// names (never values or lengths), and (3) point operators at
    /// `config.secretReferences` as the preferred remediation.
    #[tokio::test]
    async fn provision_rejects_inline_secret_by_default() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping inline-secret gate test: Docker not available");
            return;
        };

        let manifest = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "inline-gate", "name": "Inline Gate" },
            "secrets": [
                { "logicalName": "openai_api_key", "secretValue": "sk-test-xxxxxxxx" },
                { "logicalName": "anthropic_api_key", "secretValue": "sk-ant-xxxxxxxx" }
            ]
        });

        let response = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision",
                "platformAdmin",
                Body::from(serde_json::to_vec(&manifest).unwrap()),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["success"], false);
        assert_eq!(json["error"], "inline_secret_not_allowed");

        // Offending names listed, values not echoed.
        let offending = json["offendingSecrets"].as_array().unwrap();
        let names: Vec<&str> = offending.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"openai_api_key"));
        assert!(names.contains(&"anthropic_api_key"));

        // Plaintext must never leak back through the error body.
        let body_str = String::from_utf8_lossy(
            &axum::body::to_bytes(
                // Re-render to string for substring scan.
                axum::body::Body::from(serde_json::to_vec(&json).unwrap()),
                usize::MAX,
            )
            .await
            .unwrap(),
        )
        .to_string();
        assert!(!body_str.contains("sk-test-xxxxxxxx"));
        assert!(!body_str.contains("sk-ant-xxxxxxxx"));

        // Remediation surface points operators at the preferred path and
        // exposes the exact two gates they need to flip.
        assert_eq!(json["remediation"]["preferred"], "config.secretReferences");
        assert_eq!(
            json["remediation"]["fallback"]["serverConfigKey"],
            "provisioning.allowInlineSecret"
        );
        assert_eq!(
            json["remediation"]["fallback"]["queryParameter"],
            "allowInline=true"
        );
        assert_eq!(json["remediation"]["fallback"]["serverFlagEnabled"], false);
        assert_eq!(json["remediation"]["fallback"]["queryFlagProvided"], false);
    }

    /// `allowInline=true` on the query alone is not sufficient — the
    /// server config gate must *also* be flipped. This proves the
    /// fallback "both switches required" chain and guards against
    /// a future refactor that accidentally honors the query flag on
    /// its own. Note how the diagnostic JSON correctly reports
    /// `queryFlagProvided=true` while `serverFlagEnabled=false`.
    #[tokio::test]
    async fn provision_rejects_inline_secret_when_only_query_flag_is_set() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping inline-secret gate test: Docker not available");
            return;
        };

        let manifest = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "inline-gate-q", "name": "Inline Gate Q" },
            "secrets": [
                { "logicalName": "stripe_key", "secretValue": "sk_live_xxxxxxxx" }
            ]
        });

        let response = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision?allowInline=true",
                "platformAdmin",
                Body::from(serde_json::to_vec(&manifest).unwrap()),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "inline_secret_not_allowed");
        assert_eq!(json["remediation"]["fallback"]["queryFlagProvided"], true);
        assert_eq!(
            json["remediation"]["fallback"]["serverFlagEnabled"], false,
            "server flag defaults off in tests; the query flag alone \
             must not open the gate"
        );
    }

    /// Empty-valued `secrets[]` entries must pass the gate — they carry
    /// a `logicalName` but no plaintext, so they express intent
    /// (materialise a secret with this name) without violating the
    /// policy. This is the shape callers are expected to migrate to
    /// once `config.secretReferences` supplies the bytes.
    #[tokio::test]
    async fn provision_accepts_empty_secret_values_without_allow_inline() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping inline-secret gate test: Docker not available");
            return;
        };

        let manifest = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": {
                "slug": "inline-gate-empty",
                "name": "Inline Gate Empty"
            },
            // Empty byte string — deserializes to a 0-byte SecretBytes,
            // which the gate explicitly allows. No `?allowInline=true`.
            "secrets": [
                { "logicalName": "vault_token", "secretValue": "" }
            ]
        });

        let response = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision",
                "platformAdmin",
                Body::from(serde_json::to_vec(&manifest).unwrap()),
            ))
            .await
            .unwrap();

        // Either 200 (full pipeline ran with an empty secret) or some
        // downstream error, but NOT the 4.3 gate rejection — the gate
        // must not fire on an empty value.
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        if status == StatusCode::UNPROCESSABLE_ENTITY {
            let json: serde_json::Value =
                serde_json::from_slice(&body).expect("422 body must be JSON");
            assert_ne!(
                json["error"], "inline_secret_not_allowed",
                "the inline-secret gate must not reject a zero-byte \
                 secretValue — that shape is the preferred migration \
                 target"
            );
        }
    }

    // ─── metadata / providers schema regressions ─────────────────────────

    #[test]
    fn manifest_deserializes_without_metadata_or_providers() {
        // Backward-compat: pre-B2 manifests (no metadata, no providers) must
        // still round-trip through serde without errors.
        let raw = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "legacy", "name": "Legacy" }
        });
        let m: TenantManifest = serde_json::from_value(raw).expect("legacy manifest must parse");
        assert!(m.metadata.is_none());
        assert!(m.providers.is_none());
        assert!(validate_manifest(&m).is_empty());
    }

    #[test]
    fn manifest_deserializes_with_metadata_generation() {
        let raw = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "metadata": { "generation": 7, "labels": { "env": "prod" } },
            "tenant": { "slug": "gen", "name": "Gen" }
        });
        let m: TenantManifest = serde_json::from_value(raw).unwrap();
        let meta = m.metadata.as_ref().unwrap();
        assert_eq!(meta.generation, Some(7));
        assert_eq!(meta.labels.get("env"), Some(&"prod".to_string()));
        assert!(validate_manifest(&m).is_empty());
    }

    #[test]
    fn manifest_deserializes_with_account_ref_and_environment() {
        let raw = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": {
                "slug": "acme-prod",
                "name": "Acme Prod",
                "accountRef": "acme",
                "environment": "prod"
            }
        });
        let m: TenantManifest = serde_json::from_value(raw).unwrap();
        assert_eq!(m.tenant.account_ref.as_deref(), Some("acme"));
        assert_eq!(m.tenant.environment.as_deref(), Some("prod"));
        assert!(validate_manifest(&m).is_empty());
    }

    #[test]
    fn validate_manifest_rejects_generation_zero() {
        let m = TenantManifest {
            api_version: "aeterna.io/v1".into(),
            kind: "TenantManifest".into(),
            metadata: Some(ManifestMetadata {
                generation: Some(0),
                ..Default::default()
            }),
            providers: None,
            tenant: ManifestTenant {
                slug: "gz".into(),
                name: "Gz".into(),
                account_ref: None,
                environment: None,
                domain_mappings: None,
            },
            config: None,
            secrets: None,
            repository: None,
            hierarchy: None,
            roles: None,
        };
        let errors = validate_manifest(&m);
        assert!(
            errors.iter().any(|e| e.contains("generation")),
            "expected generation error, got: {errors:?}"
        );
    }

    #[test]
    fn manifest_deserializes_with_providers_block() {
        let raw = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "p", "name": "P" },
            "config": {
                "secretReferences": {
                    "openai.key": {
                        "logicalName": "openai.key",
                        "ownership": "Tenant",
                        "kind": "postgres",
                        "secretId": "11111111-1111-1111-1111-111111111111"
                    }
                }
            },
            "providers": {
                "llm": {
                    "kind": "openai",
                    "model": "gpt-4o",
                    "secretRef": "openai.key",
                    "config": { "baseUrl": "https://api.openai.com/v1" }
                },
                "embedding": {
                    "kind": "openai",
                    "model": "text-embedding-3-small",
                    "secretRef": "openai.key"
                },
                "memoryLayers": {
                    "episodic": { "kind": "qdrant", "config": { "collection": "ep" } }
                }
            }
        });
        let m: TenantManifest = serde_json::from_value(raw).unwrap();
        let providers = m.providers.as_ref().unwrap();
        assert_eq!(providers.llm.as_ref().unwrap().kind, "openai");
        assert_eq!(
            providers.llm.as_ref().unwrap().secret_ref.as_deref(),
            Some("openai.key")
        );
        assert!(providers.memory_layers.contains_key("episodic"));
        assert!(
            validate_manifest(&m).is_empty(),
            "a manifest with secret_ref that resolves must validate clean"
        );
    }

    #[test]
    fn validate_manifest_rejects_unresolved_provider_secret_ref() {
        let raw = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "p", "name": "P" },
            "providers": {
                "llm": { "kind": "openai", "secretRef": "does.not.exist" }
            }
        });
        let m: TenantManifest = serde_json::from_value(raw).unwrap();
        let errors = validate_manifest(&m);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("does.not.exist") && e.contains("secretRef")),
            "expected unresolved-secretRef error, got: {errors:?}"
        );
    }

    #[test]
    fn validate_manifest_rejects_provider_empty_kind() {
        let raw = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "p", "name": "P" },
            "providers": {
                "llm": { "kind": "" }
            }
        });
        let m: TenantManifest = serde_json::from_value(raw).unwrap();
        let errors = validate_manifest(&m);
        assert!(
            errors.iter().any(|e| e.contains("providers.llm.kind")),
            "expected empty-kind error, got: {errors:?}"
        );
    }

    // ── §2.2-A apply_manifest_providers_to_config tests ──────────────────
    // These cover the forward-apply parity fix for FINDINGS-2-2 §2.2-A.
    // Before this helper, `manifest.providers` was validated by
    // `validate_manifest` but silently dropped by `provision_tenant`
    // (bug reproduction described in openspec/changes/harden-tenant-
    // provisioning/FINDINGS-2-2.md Finding 1).

    fn empty_doc() -> TenantConfigDocument {
        TenantConfigDocument {
            tenant_id: mk_core::types::TenantId::new("t".to_string()).expect("'t' is a valid slug"),
            fields: BTreeMap::new(),
            secret_references: BTreeMap::new(),
        }
    }

    #[test]
    fn apply_providers_llm_writes_canonical_keys() {
        // A minimal openai LLM block must produce the exact keys the
        // provider registry reads in `memory/src/provider_registry.rs`
        // (config_keys::LLM_PROVIDER / LLM_MODEL). If the registry's
        // key names ever drift, this test fails loudly rather than
        // silently leaving tenants without a provider.
        let providers = ManifestProviders {
            llm: Some(ManifestProvider {
                kind: "openai".to_string(),
                model: Some("gpt-4o".to_string()),
                secret_ref: None,
                config: BTreeMap::new(),
            }),
            embedding: None,
            memory_layers: BTreeMap::new(),
        };
        let mut doc = empty_doc();
        apply_manifest_providers_to_config(&providers, &mut doc);
        assert_eq!(
            doc.fields.get(config_keys::LLM_PROVIDER).map(|f| &f.value),
            Some(&serde_json::json!("openai"))
        );
        assert_eq!(
            doc.fields.get(config_keys::LLM_MODEL).map(|f| &f.value),
            Some(&serde_json::json!("gpt-4o"))
        );
        // Embedding untouched because it wasn't declared.
        assert!(!doc.fields.contains_key(config_keys::EMBEDDING_PROVIDER));
        // All writes are Platform-owned (operator cannot override via
        // TenantAdmin config writes — mirrors the set_provider_llm
        // handler's ownership choice).
        for key in [config_keys::LLM_PROVIDER, config_keys::LLM_MODEL] {
            assert_eq!(
                doc.fields.get(key).unwrap().ownership,
                TenantConfigOwnership::Platform,
                "field {key} should be platform-owned"
            );
        }
    }

    #[test]
    fn apply_providers_model_is_optional() {
        // `ManifestProvider.model: Option<String>` — absence means
        // "no model override"; the provider key still gets written so
        // downstream code knows the provider was declared.
        let providers = ManifestProviders {
            llm: Some(ManifestProvider {
                kind: "openai".to_string(),
                model: None,
                secret_ref: None,
                config: BTreeMap::new(),
            }),
            ..Default::default()
        };
        let mut doc = empty_doc();
        apply_manifest_providers_to_config(&providers, &mut doc);
        assert!(doc.fields.contains_key(config_keys::LLM_PROVIDER));
        assert!(
            !doc.fields.contains_key(config_keys::LLM_MODEL),
            "absent model must not write an empty key"
        );
    }

    #[test]
    fn apply_providers_google_extras_flattened() {
        // Google Vertex: the manifest carries camelCase projectId /
        // location in the `config` map; storage uses snake_case under
        // config_keys. The helper bridges the convention gap.
        let providers = ManifestProviders {
            llm: Some(ManifestProvider {
                kind: "google".to_string(),
                model: Some("gemini-1.5-pro".to_string()),
                secret_ref: None,
                config: BTreeMap::from([
                    ("projectId".to_string(), "my-proj".to_string()),
                    ("location".to_string(), "europe-west1".to_string()),
                ]),
            }),
            ..Default::default()
        };
        let mut doc = empty_doc();
        apply_manifest_providers_to_config(&providers, &mut doc);
        assert_eq!(
            doc.fields
                .get(config_keys::LLM_GOOGLE_PROJECT_ID)
                .map(|f| &f.value),
            Some(&serde_json::json!("my-proj"))
        );
        assert_eq!(
            doc.fields
                .get(config_keys::LLM_GOOGLE_LOCATION)
                .map(|f| &f.value),
            Some(&serde_json::json!("europe-west1"))
        );
        // Bedrock region must NOT be written for a google provider.
        assert!(
            !doc.fields.contains_key(config_keys::LLM_BEDROCK_REGION),
            "bedrock_region leaked into a google provider config"
        );
    }

    #[test]
    fn apply_providers_bedrock_region_flattened() {
        let providers = ManifestProviders {
            embedding: Some(ManifestProvider {
                kind: "bedrock".to_string(),
                model: Some("amazon.titan-embed-text-v2:0".to_string()),
                secret_ref: None,
                config: BTreeMap::from([("region".to_string(), "us-east-1".to_string())]),
            }),
            ..Default::default()
        };
        let mut doc = empty_doc();
        apply_manifest_providers_to_config(&providers, &mut doc);
        assert_eq!(
            doc.fields
                .get(config_keys::EMBEDDING_BEDROCK_REGION)
                .map(|f| &f.value),
            Some(&serde_json::json!("us-east-1"))
        );
        // Google-specific keys must stay absent for a bedrock provider.
        assert!(
            !doc.fields
                .contains_key(config_keys::EMBEDDING_GOOGLE_PROJECT_ID)
        );
    }

    #[test]
    fn apply_providers_secret_ref_aliased_to_canonical() {
        // Operator names their secret `openai_key`; the provider block
        // points at it via `secret_ref: "openai_key"`. The helper must
        // clone that TenantSecretReference under the canonical
        // `llm_api_key` so provider_registry can resolve it, while
        // leaving the operator-named entry intact (both keys point at
        // the same underlying secret_id — no bytes duplicated).
        let source_ref = TenantSecretReference {
            logical_name: "openai_key".to_string(),
            ownership: TenantConfigOwnership::Tenant,
            reference: mk_core::secret::SecretReference::Postgres {
                secret_id: uuid::Uuid::nil(),
            },
        };
        let mut doc = empty_doc();
        doc.secret_references
            .insert("openai_key".to_string(), source_ref.clone());

        let providers = ManifestProviders {
            llm: Some(ManifestProvider {
                kind: "openai".to_string(),
                model: Some("gpt-4o".to_string()),
                secret_ref: Some("openai_key".to_string()),
                config: BTreeMap::new(),
            }),
            ..Default::default()
        };
        apply_manifest_providers_to_config(&providers, &mut doc);

        assert!(
            doc.secret_references.contains_key("openai_key"),
            "operator name must remain so round-trip render can pick it back out"
        );
        let canonical = doc
            .secret_references
            .get(config_keys::LLM_API_KEY)
            .expect("canonical alias must be registered");
        // Clone, not move: underlying Postgres secret_id matches.
        match (&canonical.reference, &source_ref.reference) {
            (
                mk_core::secret::SecretReference::Postgres { secret_id: a },
                mk_core::secret::SecretReference::Postgres { secret_id: b },
            ) => assert_eq!(a, b, "alias must point at same secret_id as source"),
            _ => panic!("unexpected reference shape"),
        }
    }

    #[test]
    fn apply_providers_unresolvable_secret_ref_is_silent_skip() {
        // If `secret_ref` names a secret that isn't in the document's
        // secret_references map, the helper leaves the canonical alias
        // unset. This path is unreachable in production because
        // `validate_manifest` already rejects unresolvable refs — but
        // we test it as a defense-in-depth against a validator bypass,
        // and we must NEVER panic on bad input.
        let providers = ManifestProviders {
            llm: Some(ManifestProvider {
                kind: "openai".to_string(),
                model: None,
                secret_ref: Some("not-in-map".to_string()),
                config: BTreeMap::new(),
            }),
            ..Default::default()
        };
        let mut doc = empty_doc();
        apply_manifest_providers_to_config(&providers, &mut doc);
        // Provider key still written; alias NOT fabricated.
        assert!(doc.fields.contains_key(config_keys::LLM_PROVIDER));
        assert!(!doc.secret_references.contains_key(config_keys::LLM_API_KEY));
    }

    #[test]
    fn apply_providers_empty_block_is_noop() {
        // ManifestProviders::default() = all fields None / empty.
        // Applying it must leave the document entirely unchanged so a
        // tenant that doesn't declare any provider block in their
        // manifest continues to work with whatever provider state
        // was set via PUT /admin/tenants/{t}/providers/{llm,embedding}.
        let mut doc = empty_doc();
        doc.fields.insert(
            "existing_key".to_string(),
            TenantConfigField {
                ownership: TenantConfigOwnership::Tenant,
                value: serde_json::json!("keep-me"),
            },
        );
        apply_manifest_providers_to_config(&ManifestProviders::default(), &mut doc);
        assert_eq!(doc.fields.len(), 1);
        assert_eq!(
            doc.fields.get("existing_key").unwrap().value,
            serde_json::json!("keep-me")
        );
    }

    #[test]
    fn apply_providers_memory_layers_write_dot_prefixed_keys() {
        // B3 §2.2-D: memoryLayers now round-trip. Each layer produces
        // `memory_layer.{layer}.provider` (+ optional `.model` /
        // `.api_key` / provider-kind extras). Dot-separator at the
        // prefix boundary is the collision-free namespace marker.
        let providers = ManifestProviders {
            memory_layers: BTreeMap::from([(
                "episodic".to_string(),
                ManifestProvider {
                    kind: "qdrant".to_string(),
                    model: Some("text-embedding-3-small".to_string()),
                    secret_ref: None,
                    config: BTreeMap::new(),
                },
            )]),
            ..Default::default()
        };
        let mut doc = empty_doc();
        apply_manifest_providers_to_config(&providers, &mut doc);

        assert_eq!(
            doc.fields
                .get("memory_layer.episodic.provider")
                .unwrap()
                .value,
            serde_json::json!("qdrant")
        );
        assert_eq!(
            doc.fields.get("memory_layer.episodic.model").unwrap().value,
            serde_json::json!("text-embedding-3-small")
        );
        // No secret_ref on this fixture → no api_key alias written.
        assert!(!doc.fields.contains_key("memory_layer.episodic.api_key"));
        assert!(doc.secret_references.is_empty());
        // Exactly two fields (provider + model) — nothing leaked.
        assert_eq!(doc.fields.len(), 2);
    }

    #[test]
    fn apply_providers_memory_layers_write_secret_alias_and_bedrock_extra() {
        // Stacks §2.2-D on top of §2.2-A's apply_one machinery:
        // bedrock extras + secret aliasing should pass through with
        // the layer-scoped prefix unchanged.
        let mut doc = empty_doc();
        doc.secret_references.insert(
            "episodic_key".to_string(),
            TenantSecretReference {
                logical_name: "episodic_key".to_string(),
                ownership: TenantConfigOwnership::Platform,
                reference: mk_core::SecretReference::Env {
                    var: "AETERNA_EPISODIC".to_string(),
                },
            },
        );
        let providers = ManifestProviders {
            memory_layers: BTreeMap::from([(
                "episodic".to_string(),
                ManifestProvider {
                    kind: "bedrock".to_string(),
                    model: None,
                    secret_ref: Some("episodic_key".to_string()),
                    config: BTreeMap::from([("region".to_string(), "eu-west-3".to_string())]),
                },
            )]),
            ..Default::default()
        };
        apply_manifest_providers_to_config(&providers, &mut doc);

        assert_eq!(
            doc.fields
                .get("memory_layer.episodic.bedrock_region")
                .unwrap()
                .value,
            json!("eu-west-3")
        );
        // Canonical api_key alias points at the same SecretReference
        // as the operator name — identical to how llm/embedding alias.
        let operator = doc.secret_references.get("episodic_key").unwrap();
        let canonical = doc
            .secret_references
            .get("memory_layer.episodic.api_key")
            .unwrap();
        assert_eq!(operator.reference, canonical.reference);
    }

    #[test]
    fn memory_layer_key_validation_accepts_valid_and_rejects_invalid() {
        // Positive: lowercase alpha start, alphanumeric + `_`/`-` body,
        // 1–63 chars.
        for ok in &["a", "episodic", "long_term", "short-term", "l0"] {
            assert!(is_valid_memory_layer_key(ok), "expected {ok:?} to be valid");
        }
        // Negative: empty, too long, leading digit, leading dash,
        // leading underscore, uppercase, dot, whitespace, unicode.
        let too_long = "a".repeat(64);
        for bad in &[
            "",
            too_long.as_str(),
            "0abc",
            "-abc",
            "_abc",
            "Episodic",
            "a.b",
            "a b",
            "épisode",
        ] {
            assert!(
                !is_valid_memory_layer_key(bad),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn validate_manifest_rejects_invalid_memory_layer_key() {
        // Validator surfaces a diagnostic that names the bad key, so
        // operators can grep their manifest file.
        let mut layers = BTreeMap::new();
        layers.insert(
            "Bad.Name".to_string(),
            ManifestProvider {
                kind: "qdrant".to_string(),
                model: None,
                secret_ref: None,
                config: BTreeMap::new(),
            },
        );
        let m = TenantManifest {
            api_version: "aeterna.io/v1".into(),
            kind: "TenantManifest".into(),
            metadata: None,
            tenant: ManifestTenant {
                slug: "acme".into(),
                name: "Acme".into(),
                account_ref: None,
                environment: None,
                domain_mappings: None,
            },
            config: None,
            secrets: None,
            providers: Some(ManifestProviders {
                memory_layers: layers,
                ..Default::default()
            }),
            repository: None,
            hierarchy: None,
            roles: None,
        };
        let errors = validate_manifest(&m);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("Bad.Name") && e.contains("invalid")),
            "missing validator error for invalid layer key: {errors:?}"
        );
    }

    // ── B1 §1.2 SecretReference variant validation tests ─────────────────
    // These exercise validate_manifest's per-variant well-formedness
    // checks. The #[serde(tag = "kind")] union handles unknown kinds at
    // parse time; validate_manifest handles empty/malformed fields
    // within known kinds.

    #[test]
    fn validate_manifest_accepts_all_secret_reference_variants() {
        // One-shot coverage: every variant in its canonical well-formed
        // shape must pass validate_manifest.
        let raw = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "sr", "name": "SR" },
            "config": {
                "fields": {},
                "secretReferences": {
                    "inline-ok":   { "logicalName": "a", "ownership": "Tenant",
                                     "kind": "inline",   "plaintext": "hunter2" },
                    "env-ok":      { "logicalName": "b", "ownership": "Tenant",
                                     "kind": "env",      "var": "DATABASE_PASSWORD" },
                    "file-ok":     { "logicalName": "c", "ownership": "Tenant",
                                     "kind": "file",     "path": "/run/secrets/db" },
                    "k8s-ok":      { "logicalName": "d", "ownership": "Tenant",
                                     "kind": "k8s",      "name": "db-secret",
                                                         "key": "password" },
                    "k8s-ns-ok":   { "logicalName": "e", "ownership": "Tenant",
                                     "kind": "k8s",      "name": "db-secret",
                                                         "key": "password",
                                                         "namespace": "aeterna" },
                    "vault-ok":    { "logicalName": "f", "ownership": "Tenant",
                                     "kind": "vault",    "mount": "secret",
                                                         "path": "tenants/acme/db",
                                                         "field": "password" }
                }
            }
        });
        let m: TenantManifest = serde_json::from_value(raw).unwrap();
        assert!(
            validate_manifest(&m).is_empty(),
            "well-formed manifest must validate cleanly, got: {:?}",
            validate_manifest(&m)
        );
    }

    #[test]
    fn validate_manifest_rejects_inline_empty_plaintext() {
        let raw = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "sr", "name": "SR" },
            "config": {
                "fields": {},
                "secretReferences": {
                    "bad": { "logicalName": "a", "ownership": "Tenant",
                             "kind": "inline", "plaintext": "" }
                }
            }
        });
        let m: TenantManifest = serde_json::from_value(raw).unwrap();
        let errors = validate_manifest(&m);
        assert!(
            errors.iter().any(|e| e.contains("inline plaintext")),
            "expected inline-empty error, got: {errors:?}"
        );
    }

    #[test]
    fn validate_manifest_rejects_env_empty_var() {
        let raw = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "sr", "name": "SR" },
            "config": {
                "fields": {},
                "secretReferences": {
                    "bad": { "logicalName": "a", "ownership": "Tenant",
                             "kind": "env", "var": "" }
                }
            }
        });
        let m: TenantManifest = serde_json::from_value(raw).unwrap();
        let errors = validate_manifest(&m);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("bad.var must not be empty")),
            "expected env-empty error, got: {errors:?}"
        );
    }

    #[test]
    fn validate_manifest_rejects_env_var_with_equals() {
        let raw = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "sr", "name": "SR" },
            "config": {
                "fields": {},
                "secretReferences": {
                    "bad": { "logicalName": "a", "ownership": "Tenant",
                             "kind": "env", "var": "FOO=BAR" }
                }
            }
        });
        let m: TenantManifest = serde_json::from_value(raw).unwrap();
        let errors = validate_manifest(&m);
        assert!(
            errors.iter().any(|e| e.contains("'=' or null bytes")),
            "expected env-malformed error, got: {errors:?}"
        );
    }

    #[test]
    fn validate_manifest_rejects_file_relative_path() {
        let raw = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "sr", "name": "SR" },
            "config": {
                "fields": {},
                "secretReferences": {
                    "bad": { "logicalName": "a", "ownership": "Tenant",
                             "kind": "file", "path": "relative/path" }
                }
            }
        });
        let m: TenantManifest = serde_json::from_value(raw).unwrap();
        let errors = validate_manifest(&m);
        assert!(
            errors.iter().any(|e| e.contains("must be absolute")),
            "expected file-relative error, got: {errors:?}"
        );
    }

    #[test]
    fn validate_manifest_rejects_k8s_empty_fields() {
        let raw = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "sr", "name": "SR" },
            "config": {
                "fields": {},
                "secretReferences": {
                    "bad": { "logicalName": "a", "ownership": "Tenant",
                             "kind": "k8s", "name": "", "key": "" }
                }
            }
        });
        let m: TenantManifest = serde_json::from_value(raw).unwrap();
        let errors = validate_manifest(&m);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("bad.name must not be empty")),
            "missing name error, got: {errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|e| e.contains("bad.key must not be empty")),
            "missing key error, got: {errors:?}"
        );
    }

    #[test]
    fn validate_manifest_rejects_k8s_empty_namespace_when_set() {
        let raw = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "sr", "name": "SR" },
            "config": {
                "fields": {},
                "secretReferences": {
                    "bad": { "logicalName": "a", "ownership": "Tenant",
                             "kind": "k8s", "name": "n", "key": "k",
                             "namespace": "   " }
                }
            }
        });
        let m: TenantManifest = serde_json::from_value(raw).unwrap();
        let errors = validate_manifest(&m);
        assert!(
            errors.iter().any(|e| e.contains("namespace, when set")),
            "expected namespace-when-set error, got: {errors:?}"
        );
    }

    #[test]
    fn validate_manifest_rejects_vault_partial_fields() {
        let raw = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "sr", "name": "SR" },
            "config": {
                "fields": {},
                "secretReferences": {
                    "bad": { "logicalName": "a", "ownership": "Tenant",
                             "kind": "vault", "mount": "secret",
                             "path": "", "field": "" }
                }
            }
        });
        let m: TenantManifest = serde_json::from_value(raw).unwrap();
        let errors = validate_manifest(&m);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("bad.path must not be empty")),
            "missing path error, got: {errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|e| e.contains("bad.field must not be empty")),
            "missing field error, got: {errors:?}"
        );
    }

    #[test]
    fn deserialize_rejects_unknown_secret_reference_kind_at_parse_time() {
        // serde tag="kind" short-circuits before validate_manifest ever
        // sees the value. This test locks that contract so a future
        // change making the enum non-exhaustive (e.g. #[serde(other)])
        // is a deliberate decision and breaks this test.
        let raw = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "sr", "name": "SR" },
            "config": {
                "fields": {},
                "secretReferences": {
                    "bad": { "logicalName": "a", "ownership": "Tenant",
                             "kind": "mysterybox", "whatever": 1 }
                }
            }
        });
        let parsed: Result<TenantManifest, _> = serde_json::from_value(raw);
        assert!(
            parsed.is_err(),
            "unknown secret-reference kind must fail at serde parse, not reach validate_manifest"
        );
    }

    #[tokio::test]
    async fn provision_tenant_idempotent_reapply() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping provision idempotent test: Docker not available");
            return;
        };

        let manifest = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": {
                "slug": "idempotent-test",
                "name": "Idempotent Test"
            }
        });

        let r1 = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision",
                "platformAdmin",
                Body::from(serde_json::to_vec(&manifest).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(r1.status(), StatusCode::OK);
        let b1 = axum::body::to_bytes(r1.into_body(), usize::MAX)
            .await
            .unwrap();
        let j1: serde_json::Value = serde_json::from_slice(&b1).unwrap();
        let tenant_id_1 = j1["tenantId"].as_str().unwrap().to_string();

        let r2 = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision",
                "platformAdmin",
                Body::from(serde_json::to_vec(&manifest).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(r2.status(), StatusCode::OK);
        let b2 = axum::body::to_bytes(r2.into_body(), usize::MAX)
            .await
            .unwrap();
        let j2: serde_json::Value = serde_json::from_slice(&b2).unwrap();
        assert_eq!(j2["success"], true);

        // B2 task 1.6 contract: second apply of the same manifest short-
        // circuits — `status == "unchanged"`, the `steps` array is empty
        // (no pipeline was executed), and the returned hash/generation
        // match the first apply. `tenantId` is NOT returned on the
        // short-circuit path because we never reload the row; callers who
        // need it should read it from the first apply's response.
        assert_eq!(
            j2["status"].as_str(),
            Some("unchanged"),
            "re-apply with identical manifest must short-circuit"
        );
        assert_eq!(
            j2["steps"].as_array().map(|a| a.len()),
            Some(0),
            "short-circuit must skip all pipeline steps"
        );
        assert_eq!(j2["slug"], "idempotent-test");
        assert!(
            j2["hash"]
                .as_str()
                .is_some_and(|h| h.starts_with("sha256:")),
            "hash must be a sha256: fingerprint, got: {:?}",
            j2["hash"]
        );
        assert_eq!(
            j2["generation"], 1,
            "first apply auto-assigns generation=1, re-apply reports the same"
        );

        // Sanity: tenant ID on first apply was stable (we can still assert
        // against it even though the re-apply response does not echo it,
        // via the slug lookup below).
        let _ = tenant_id_1;
    }

    #[tokio::test]
    async fn provision_tenant_bumps_generation_on_modified_reapply() {
        // When the manifest *changes* (different name → different hash),
        // provision_tenant runs the full pipeline and persists the new
        // `(hash, generation)` pair. The second apply's generation must be
        // current + 1 because the caller did not set metadata.generation.
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping generation-bump test: Docker not available");
            return;
        };

        let m1 = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "genbump-test", "name": "Genbump v1" }
        });
        let m2 = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "genbump-test", "name": "Genbump v2" }
        });

        let r1 = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision",
                "platformAdmin",
                Body::from(serde_json::to_vec(&m1).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(r1.status(), StatusCode::OK);
        let j1: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(r1.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(j1["status"], "applied");
        assert_eq!(j1["generation"], 1);
        let hash1 = j1["hash"].as_str().unwrap().to_string();

        let r2 = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision",
                "platformAdmin",
                Body::from(serde_json::to_vec(&m2).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(r2.status(), StatusCode::OK);
        let j2: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(r2.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(j2["status"], "applied", "content change must run pipeline");
        assert_eq!(j2["generation"], 2, "omitted generation auto-increments");
        let hash2 = j2["hash"].as_str().unwrap();
        assert_ne!(
            hash1, hash2,
            "differing manifests must produce differing hashes"
        );
    }

    #[tokio::test]
    async fn provision_tenant_rejects_non_increasing_generation() {
        // When the caller pins metadata.generation, it MUST strictly exceed
        // the row's current generation. Equal or lower → 409 Conflict with
        // a structured error envelope; no pipeline side effects.
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping generation-conflict test: Docker not available");
            return;
        };

        let m1 = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "metadata": { "generation": 1 },
            "tenant": { "slug": "genconflict-test", "name": "Genconflict" }
        });
        // Second submit pins the same generation 1 → must be rejected even
        // though the body differs (name change).
        let m2_stale = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "metadata": { "generation": 1 },
            "tenant": { "slug": "genconflict-test", "name": "Genconflict updated" }
        });

        let r1 = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision",
                "platformAdmin",
                Body::from(serde_json::to_vec(&m1).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(r1.status(), StatusCode::OK);

        let r2 = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision",
                "platformAdmin",
                Body::from(serde_json::to_vec(&m2_stale).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(
            r2.status(),
            StatusCode::CONFLICT,
            "stale generation must surface as 409"
        );
        let j2: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(r2.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(j2["error"], "generation_conflict");
        assert_eq!(j2["currentGeneration"], 1);
        assert_eq!(j2["submittedGeneration"], 1);
    }

    #[tokio::test]
    async fn provision_tenant_rejects_invalid_manifest() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping provision validation test: Docker not available");
            return;
        };

        let manifest = serde_json::json!({
            "apiVersion": "wrong",
            "kind": "TenantManifest",
            "tenant": {
                "slug": "My_Bad_Slug",
                "name": "Test"
            }
        });

        let response = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision",
                "platformAdmin",
                Body::from(serde_json::to_vec(&manifest).unwrap()),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], false);
        assert!(
            json["validationErrors"].as_array().unwrap().len() >= 2,
            "expected at least 2 validation errors"
        );
    }

    #[tokio::test]
    async fn provision_tenant_requires_platform_admin() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping provision auth test: Docker not available");
            return;
        };

        let manifest = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": {
                "slug": "auth-test",
                "name": "Auth Test"
            }
        });

        let response = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision",
                "developer",
                Body::from(serde_json::to_vec(&manifest).unwrap()),
            ))
            .await
            .unwrap();

        assert!(
            response.status() == StatusCode::FORBIDDEN
                || response.status() == StatusCode::UNAUTHORIZED,
            "expected 403 or 401, got {}",
            response.status()
        );
    }

    // ── B3 §2.1 dry-run tests ────────────────────────────────────────────
    // Docker-gated like the rest of the provision_* suite. Each test
    // asserts both the wire shape of ProvisionPlan AND that dry-run did
    // not leak state (via a follow-up non-dry-run apply observing
    // `status: "applied"` with `generation: 1`, which could only happen
    // if the dry-run left no row behind).

    #[tokio::test]
    async fn provision_tenant_dry_run_returns_create_plan_for_new_slug() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping dry-run create test: Docker not available");
            return;
        };

        let manifest = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "dryrun-new", "name": "Dryrun New" },
            "config": {
                "fields": {
                    "ui.theme": { "ownership": "Tenant", "value": "dark" }
                },
                "secretReferences": {}
            }
        });

        let resp = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision?dryRun=true",
                "platformAdmin",
                Body::from(serde_json::to_vec(&manifest).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(j["dryRun"], true);
        assert_eq!(j["status"], "create");
        assert_eq!(j["slug"], "dryrun-new");
        assert_eq!(j["currentGeneration"], 0);
        assert_eq!(j["nextGeneration"], 1);
        assert!(j["currentHash"].is_null(), "no prior state → null hash");
        assert!(
            j["incomingHash"]
                .as_str()
                .is_some_and(|h| h.starts_with("sha256:")),
            "incomingHash must be sha256: fingerprint, got: {:?}",
            j["incomingHash"]
        );
        assert_eq!(j["configFieldCount"], 1);
        assert_eq!(j["secretReferenceCount"], 0);
        assert_eq!(j["hasRepositoryBinding"], false);
        assert_eq!(j["hasDomainMappings"], false);
        assert_eq!(j["hasHierarchy"], false);
        assert_eq!(j["hasRoles"], false);
        assert_eq!(j["hasProviders"], false);

        // Prove dry-run did not create the tenant: a subsequent real
        // apply lands as a fresh creation with generation=1.
        let resp2 = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision",
                "platformAdmin",
                Body::from(serde_json::to_vec(&manifest).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);
        let j2: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp2.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            j2["status"], "applied",
            "dry-run must not have persisted anything, so the real apply is a fresh create"
        );
        assert_eq!(j2["generation"], 1);
    }

    #[tokio::test]
    async fn provision_tenant_dry_run_reports_update_for_existing_tenant() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping dry-run update test: Docker not available");
            return;
        };

        let m1 = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "dryrun-update", "name": "Dryrun v1" }
        });
        let m2 = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "dryrun-update", "name": "Dryrun v2" }
        });

        // First: real apply of v1.
        let r1 = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision",
                "platformAdmin",
                Body::from(serde_json::to_vec(&m1).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(r1.status(), StatusCode::OK);
        let j1: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(r1.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        let hash1 = j1["hash"].as_str().unwrap().to_string();

        // Then: dry-run of v2 reports "update" with currentHash == hash1
        // and nextGeneration == 2.
        let r2 = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision?dryRun=true",
                "platformAdmin",
                Body::from(serde_json::to_vec(&m2).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(r2.status(), StatusCode::OK);
        let j2: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(r2.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(j2["dryRun"], true);
        assert_eq!(j2["status"], "update");
        assert_eq!(j2["currentHash"], hash1);
        assert_eq!(j2["currentGeneration"], 1);
        assert_eq!(j2["nextGeneration"], 2);
        assert_ne!(
            j2["incomingHash"], hash1,
            "v2 must hash differently than v1"
        );

        // Prove dry-run did not bump the generation: a real re-apply of
        // v1 is still "unchanged" at generation=1 (not 2 or 3).
        let r3 = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision",
                "platformAdmin",
                Body::from(serde_json::to_vec(&m1).unwrap()),
            ))
            .await
            .unwrap();
        let j3: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(r3.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(j3["status"], "unchanged");
        assert_eq!(j3["generation"], 1, "dry-run must not bump generation");
    }

    #[tokio::test]
    async fn provision_tenant_dry_run_unchanged_echoes_flag() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping dry-run unchanged test: Docker not available");
            return;
        };

        let m = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "tenant": { "slug": "dryrun-unchanged", "name": "Dryrun Unchanged" }
        });

        let _ = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision",
                "platformAdmin",
                Body::from(serde_json::to_vec(&m).unwrap()),
            ))
            .await
            .unwrap();

        // Same manifest, dry-run: exercises the short-circuit path which
        // returns the unchanged envelope with dryRun=true echoed.
        let r = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision?dryRun=true",
                "platformAdmin",
                Body::from(serde_json::to_vec(&m).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let j: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(r.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(j["status"], "unchanged");
        assert_eq!(j["dryRun"], true);
        assert_eq!(j["generation"], 1);
    }

    #[tokio::test]
    async fn provision_tenant_dry_run_does_not_mask_validation_errors() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping dry-run validation test: Docker not available");
            return;
        };

        // metadata.generation == 0 is rejected by validate_manifest.
        // Dry-run must NOT swallow the rejection — preview still surfaces
        // real errors.
        let bad = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "metadata": { "generation": 0 },
            "tenant": { "slug": "dryrun-invalid", "name": "Invalid" }
        });

        let r = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision?dryRun=true",
                "platformAdmin",
                Body::from(serde_json::to_vec(&bad).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let j: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(r.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(j["error"], "manifest_validation_failed");
    }

    #[tokio::test]
    async fn provision_tenant_dry_run_surfaces_generation_conflict() {
        let Some((app, _tenant)) = app_with_tenant().await else {
            eprintln!("Skipping dry-run generation-conflict test: Docker not available");
            return;
        };

        // Apply at generation=2, then dry-run at pinned generation=2:
        // must surface 409 (dry-run preview includes the generation gate).
        let m1 = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "metadata": { "generation": 2 },
            "tenant": { "slug": "dryrun-conflict", "name": "Conflict v1" }
        });
        let m2_stale = serde_json::json!({
            "apiVersion": "aeterna.io/v1",
            "kind": "TenantManifest",
            "metadata": { "generation": 2 },
            "tenant": { "slug": "dryrun-conflict", "name": "Conflict v2" }
        });

        let _ = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision",
                "platformAdmin",
                Body::from(serde_json::to_vec(&m1).unwrap()),
            ))
            .await
            .unwrap();

        let r = app
            .clone()
            .oneshot(request_with_headers(
                "POST",
                "/admin/tenants/provision?dryRun=true",
                "platformAdmin",
                Body::from(serde_json::to_vec(&m2_stale).unwrap()),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::CONFLICT);
        let j: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(r.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(j["error"], "generation_conflict");
    }

    mod permission_matrix_tests {
        use adapters::auth::matrix::{ALL_ACTIONS, role_permission_matrix};
        use std::collections::HashSet;

        #[test]
        fn all_actions_has_68_entries() {
            assert_eq!(
                ALL_ACTIONS.len(),
                68,
                "ALL_ACTIONS must have exactly 68 Cedar domain actions"
            );
        }

        #[test]
        fn all_actions_has_no_duplicates() {
            let set: HashSet<&str> = ALL_ACTIONS.iter().copied().collect();
            assert_eq!(
                set.len(),
                ALL_ACTIONS.len(),
                "ALL_ACTIONS must not contain duplicates"
            );
        }

        #[test]
        fn matrix_has_all_seven_user_roles() {
            let matrix = role_permission_matrix();
            let expected_roles = [
                "platformAdmin",
                "tenantAdmin",
                "admin",
                "architect",
                "techLead",
                "developer",
                "viewer",
            ];
            for role in &expected_roles {
                assert!(matrix.contains_key(*role), "matrix missing role '{role}'");
            }
            assert_eq!(
                matrix.len(),
                expected_roles.len(),
                "matrix has unexpected extra roles"
            );
        }

        #[test]
        fn all_role_actions_exist_in_all_actions() {
            let matrix = role_permission_matrix();
            let valid: HashSet<&str> = ALL_ACTIONS.iter().copied().collect();
            for (role, actions) in &matrix {
                for action in actions {
                    assert!(
                        valid.contains(action.as_str()),
                        "role '{role}' references unknown action '{action}'"
                    );
                }
            }
        }

        #[test]
        fn platform_admin_has_all_actions() {
            let matrix = role_permission_matrix();
            let pa = &matrix["platformAdmin"];
            assert_eq!(
                pa.len(),
                ALL_ACTIONS.len(),
                "PlatformAdmin must have ALL {} actions, got {}",
                ALL_ACTIONS.len(),
                pa.len()
            );
        }

        const CROSS_TENANT_ACTIONS: &[&str] = &[
            "ListTenants",
            "CreateTenant",
            "ManageGitProviderConnections",
            "AdminSyncGitHub",
        ];

        #[test]
        fn tenant_admin_excludes_cross_tenant_actions() {
            let matrix = role_permission_matrix();
            let ta = &matrix["tenantAdmin"];
            for action in CROSS_TENANT_ACTIONS {
                assert!(
                    !ta.contains(&action.to_string()),
                    "TenantAdmin must NOT have cross-tenant action '{action}'"
                );
            }
        }

        #[test]
        fn admin_matches_tenant_admin() {
            let matrix = role_permission_matrix();
            let mut admin: Vec<String> = matrix["admin"].clone();
            let mut tenant_admin: Vec<String> = matrix["tenantAdmin"].clone();
            admin.sort();
            tenant_admin.sort();
            assert_eq!(
                admin, tenant_admin,
                "Admin and TenantAdmin must have identical permissions"
            );
        }

        #[test]
        fn viewer_is_read_only() {
            let matrix = role_permission_matrix();
            let viewer = &matrix["viewer"];
            let write_actions: HashSet<&str> = [
                "CreateMemory",
                "UpdateMemory",
                "DeleteMemory",
                "PromoteMemory",
                "OptimizeMemory",
                "ReasonMemory",
                "CloseMemory",
                "FeedbackMemory",
                "ProposeKnowledge",
                "EditKnowledge",
                "ApproveKnowledge",
                "DeprecateKnowledge",
                "BatchKnowledge",
                "CreatePolicy",
                "EditPolicy",
                "ApprovePolicy",
                "SubmitGovernanceRequest",
                "ApproveGovernanceRequest",
                "RejectGovernanceRequest",
                "CreateOrganization",
                "CreateTeam",
                "CreateProject",
                "ManageMembers",
                "AssignRoles",
                "RegisterAgent",
                "RevokeAgent",
                "DelegateToAgent",
                "ExportData",
                "ImportData",
                "ConfigureGovernance",
                "CreateTenant",
                "UpdateTenant",
                "DeactivateTenant",
                "UpdateTenantConfig",
                "ManageTenantSecrets",
                "UpdateRepositoryBinding",
                "ManageGitProviderConnections",
                "CreateSession",
                "EndSession",
                "TriggerSync",
                "ResolveConflict",
                "ModifyGraph",
                "InvokeCCA",
                "InvokeMcpTool",
                "RegisterUser",
                "UpdateUser",
                "DeactivateUser",
                "ListTenants",
                "AdminSyncGitHub",
            ]
            .into_iter()
            .collect();

            for action in viewer {
                assert!(
                    !write_actions.contains(action.as_str()),
                    "Viewer must NOT have write action '{action}'"
                );
            }
        }

        #[test]
        fn no_role_below_admin_has_cross_tenant_actions() {
            let matrix = role_permission_matrix();
            let non_admin_roles = ["architect", "techLead", "developer", "viewer"];
            for role in &non_admin_roles {
                let actions = &matrix[*role];
                for ct_action in CROSS_TENANT_ACTIONS {
                    assert!(
                        !actions.contains(&ct_action.to_string()),
                        "Role '{role}' must NOT have cross-tenant action '{ct_action}'"
                    );
                }
            }
        }

        #[test]
        fn only_admin_plus_has_assign_roles() {
            let matrix = role_permission_matrix();
            let assign = "AssignRoles".to_string();
            let roles_with_assign: Vec<&str> = matrix
                .iter()
                .filter(|(_, actions)| actions.contains(&assign))
                .map(|(role, _)| role.as_str())
                .collect();

            for role in &roles_with_assign {
                assert!(
                    matches!(*role, "platformAdmin" | "tenantAdmin" | "admin"),
                    "Only Admin+ roles may have AssignRoles, but '{role}' has it"
                );
            }
        }

        #[test]
        fn role_action_counts() {
            let matrix = role_permission_matrix();
            assert_eq!(matrix["platformAdmin"].len(), 68);
            assert_eq!(matrix["tenantAdmin"].len(), 64);
            assert_eq!(matrix["admin"].len(), 64);
            assert_eq!(matrix["architect"].len(), 51);
            assert_eq!(matrix["techLead"].len(), 47);
            assert_eq!(matrix["developer"].len(), 29);
            assert_eq!(matrix["viewer"].len(), 18);
        }

        #[test]
        fn higher_role_is_superset_of_lower() {
            let matrix = role_permission_matrix();
            let hierarchy: Vec<(&str, &str)> = vec![
                ("developer", "viewer"),
                ("techLead", "developer"),
                ("architect", "techLead"),
                ("admin", "architect"),
                ("tenantAdmin", "admin"),
                ("platformAdmin", "tenantAdmin"),
            ];

            for (higher, lower) in &hierarchy {
                let higher_set: HashSet<&str> = matrix[*higher]
                    .iter()
                    .map(std::string::String::as_str)
                    .collect();
                let lower_set: HashSet<&str> = matrix[*lower]
                    .iter()
                    .map(std::string::String::as_str)
                    .collect();
                let missing: Vec<&&str> = lower_set
                    .iter()
                    .filter(|a| !higher_set.contains(**a))
                    .collect();
                assert!(
                    missing.is_empty(),
                    "Role '{higher}' must be a superset of '{lower}', but is missing: {missing:?}"
                );
            }
        }
    }

    mod provider_api_serde_tests {
        use super::super::*;

        #[test]
        fn set_provider_request_deserializes_minimal() {
            let json = r#"{"provider":"openai","model":"gpt-4o"}"#;
            let req: SetProviderRequest = serde_json::from_str(json).unwrap();
            assert_eq!(req.provider, "openai");
            assert_eq!(req.model, "gpt-4o");
            assert!(req.api_key.is_none());
            assert!(req.google_project_id.is_none());
            assert!(req.google_location.is_none());
            assert!(req.bedrock_region.is_none());
        }

        #[test]
        fn set_provider_request_deserializes_full() {
            let json = r#"{
                "provider": "google",
                "model": "gemini-1.5-pro",
                "apiKey": "sk-test",
                "googleProjectId": "my-project",
                "googleLocation": "us-central1",
                "bedrockRegion": "us-east-1"
            }"#;
            let req: SetProviderRequest = serde_json::from_str(json).unwrap();
            assert_eq!(req.provider, "google");
            assert_eq!(req.model, "gemini-1.5-pro");
            assert_eq!(req.api_key.as_deref(), Some("sk-test"));
            assert_eq!(req.google_project_id.as_deref(), Some("my-project"));
            assert_eq!(req.google_location.as_deref(), Some("us-central1"));
            assert_eq!(req.bedrock_region.as_deref(), Some("us-east-1"));
        }

        #[test]
        fn provider_info_serializes_camel_case() {
            let info = ProviderInfo {
                provider: Some("openai".to_string()),
                model: Some("gpt-4o".to_string()),
                configured: true,
            };
            let json = serde_json::to_value(&info).unwrap();
            assert_eq!(json["provider"], "openai");
            assert_eq!(json["model"], "gpt-4o");
            assert_eq!(json["configured"], true);
        }

        #[test]
        fn provider_info_serializes_unconfigured() {
            let info = ProviderInfo {
                provider: None,
                model: None,
                configured: false,
            };
            let json = serde_json::to_value(&info).unwrap();
            assert!(json["provider"].is_null());
            assert!(json["model"].is_null());
            assert_eq!(json["configured"], false);
        }

        #[test]
        fn tenant_providers_response_serializes() {
            let resp = TenantProvidersResponse {
                llm: ProviderInfo {
                    provider: Some("openai".to_string()),
                    model: Some("gpt-4o".to_string()),
                    configured: true,
                },
                embedding: ProviderInfo {
                    provider: None,
                    model: None,
                    configured: false,
                },
                source: "tenant".to_string(),
            };
            let json = serde_json::to_value(&resp).unwrap();
            assert_eq!(json["source"], "tenant");
            assert_eq!(json["llm"]["provider"], "openai");
            assert_eq!(json["embedding"]["configured"], false);
        }

        #[test]
        fn provider_status_info_serializes_ok() {
            let info = ProviderStatusInfo {
                status: "ok".to_string(),
                latency_ms: Some(42),
                dimension: Some(1536),
                error: None,
            };
            let json = serde_json::to_value(&info).unwrap();
            assert_eq!(json["status"], "ok");
            assert_eq!(json["latencyMs"], 42);
            assert_eq!(json["dimension"], 1536);
            assert!(json.get("error").is_none());
        }

        #[test]
        fn provider_status_info_serializes_error() {
            let info = ProviderStatusInfo {
                status: "error".to_string(),
                latency_ms: Some(150),
                dimension: None,
                error: Some("connection refused".to_string()),
            };
            let json = serde_json::to_value(&info).unwrap();
            assert_eq!(json["status"], "error");
            assert_eq!(json["error"], "connection refused");
            assert!(json.get("dimension").is_none());
        }

        #[test]
        fn tenant_provider_status_response_serializes() {
            let resp = TenantProviderStatusResponse {
                llm: ProviderStatusInfo {
                    status: "ok".to_string(),
                    latency_ms: Some(50),
                    dimension: None,
                    error: None,
                },
                embedding: ProviderStatusInfo {
                    status: "not_configured".to_string(),
                    latency_ms: None,
                    dimension: None,
                    error: Some("No embedding service".to_string()),
                },
            };
            let json = serde_json::to_value(&resp).unwrap();
            assert_eq!(json["llm"]["status"], "ok");
            assert_eq!(json["embedding"]["status"], "not_configured");
        }

        #[test]
        fn extract_provider_info_from_none_config() {
            let info = extract_provider_info(
                &None,
                config_keys::LLM_PROVIDER,
                config_keys::LLM_MODEL,
                config_keys::LLM_API_KEY,
            );
            assert!(info.provider.is_none());
            assert!(info.model.is_none());
            assert!(!info.configured);
        }

        #[test]
        fn extract_provider_info_from_populated_config() {
            let tenant_id =
                mk_core::types::TenantId::new("11111111-1111-1111-1111-111111111111".to_string())
                    .unwrap();
            let mut fields = BTreeMap::new();
            fields.insert(
                config_keys::LLM_PROVIDER.to_string(),
                TenantConfigField {
                    ownership: TenantConfigOwnership::Platform,
                    value: serde_json::json!("openai"),
                },
            );
            fields.insert(
                config_keys::LLM_MODEL.to_string(),
                TenantConfigField {
                    ownership: TenantConfigOwnership::Platform,
                    value: serde_json::json!("gpt-4o"),
                },
            );
            let mut secret_references = BTreeMap::new();
            secret_references.insert(
                config_keys::LLM_API_KEY.to_string(),
                TenantSecretReference {
                    logical_name: config_keys::LLM_API_KEY.to_string(),
                    ownership: TenantConfigOwnership::Platform,
                    reference: mk_core::SecretReference::Postgres {
                        secret_id: uuid::Uuid::nil(),
                    },
                },
            );
            let config = TenantConfigDocument {
                tenant_id,
                fields,
                secret_references,
            };
            let info = extract_provider_info(
                &Some(config),
                config_keys::LLM_PROVIDER,
                config_keys::LLM_MODEL,
                config_keys::LLM_API_KEY,
            );
            assert_eq!(info.provider.as_deref(), Some("openai"));
            assert_eq!(info.model.as_deref(), Some("gpt-4o"));
            assert!(info.configured);
        }
    }
}
