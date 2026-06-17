//! Plugin authentication endpoints for the OpenCode plugin flow.
//!
//! Provides:
//! - `POST /api/v1/auth/plugin/bootstrap` — Exchange a GitHub OAuth access token
//!   for Aeterna-issued plugin session credentials (access + refresh tokens).
//! - `POST /api/v1/auth/plugin/refresh` — Refresh an expired access token.
//! - `POST /api/v1/auth/plugin/logout` — Revoke a refresh token.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header::AUTHORIZATION};
use axum::response::{IntoResponse, Redirect};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use mk_core::types::{
    DEFAULT_TENANT_SLUG, PROVIDER_GITHUB, RoleIdentifier, SYSTEM_USER_ID, TenantContext, TenantId,
    UserId,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use super::AppState;

const DEFAULT_REFRESH_TOKEN_TTL_SECS: u64 = 30 * 24 * 3600;
const DEFAULT_ADMIN_UI_ACCESS_TOKEN_TTL_SECS: u64 = 30 * 60;
const DEFAULT_ADMIN_UI_REFRESH_TOKEN_TTL_SECS: u64 = 14 * 24 * 3600;
const DEFAULT_GITHUB_API_BASE: &str = "https://api.github.com";

// ---------------------------------------------------------------------------
// Refresh token store
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct RefreshEntry {
    tenant_id: String,
    github_login: String,
    github_id: u64,
    email: Option<String>,
    expires_at: i64,
    token_kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RefreshTokenTakeResult {
    Missing,
    WrongKind,
}

/// In-process refresh-token store (single-use, rotation on refresh).
///
/// For HA deployments swap this for a Redis- or Postgres-backed store.
#[derive(Debug, Default)]
pub struct RefreshTokenStore {
    tokens: RwLock<HashMap<String, RefreshEntry>>,
}

impl RefreshTokenStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert(
        &self,
        token: String,
        tenant_id: String,
        github_login: String,
        github_id: u64,
        email: Option<String>,
        token_kind: String,
        ttl_seconds: u64,
    ) {
        let expires_at = Utc::now().timestamp() + ttl_seconds as i64;
        self.tokens.write().await.insert(
            token,
            RefreshEntry {
                tenant_id,
                github_login,
                github_id,
                email,
                expires_at,
                token_kind,
            },
        );
    }

    pub(super) async fn take_for_kind(
        &self,
        token: &str,
        expected_kind: &str,
    ) -> Result<RefreshEntry, RefreshTokenTakeResult> {
        let now = Utc::now().timestamp();
        let guard = self.tokens.read().await;
        let Some(entry) = guard.get(token).cloned() else {
            return Err(RefreshTokenTakeResult::Missing);
        };
        drop(guard);

        if entry.expires_at <= now {
            self.tokens.write().await.remove(token);
            return Err(RefreshTokenTakeResult::Missing);
        }
        if entry.token_kind != expected_kind {
            return Err(RefreshTokenTakeResult::WrongKind);
        }

        let mut guard = self.tokens.write().await;
        match guard.remove(token) {
            Some(entry) if entry.expires_at > now => Ok(entry),
            _ => Err(RefreshTokenTakeResult::Missing),
        }
    }

    pub async fn revoke_for_kind(&self, token: &str, expected_kind: &str) {
        let mut guard = self.tokens.write().await;
        let should_remove = guard
            .get(token)
            .map(|entry| entry.token_kind == expected_kind)
            .unwrap_or(false);
        if should_remove {
            guard.remove(token);
        }
    }
}

/// Redis-backed refresh-token store for HA / multi-instance deployments.
///
/// Uses [`storage::RedisStore`] with prefix `aeterna:refresh_tokens`.
/// Tokens are stored with a TTL and consumed atomically via `GETDEL`.
pub struct RedisRefreshTokenStore {
    store: storage::RedisStore,
}

impl RedisRefreshTokenStore {
    /// Create a new Redis-backed refresh token store.
    pub fn new(store: storage::RedisStore) -> Self {
        Self { store }
    }

    /// Store a refresh token with the given TTL.
    pub async fn insert(
        &self,
        token: String,
        tenant_id: String,
        github_login: String,
        github_id: u64,
        email: Option<String>,
        token_kind: String,
        ttl_seconds: u64,
    ) {
        let expires_at = Utc::now().timestamp() + ttl_seconds as i64;
        let entry = RefreshEntry {
            tenant_id,
            github_login,
            github_id,
            email,
            expires_at,
            token_kind,
        };
        if let Err(e) = self.store.set(&token, &entry, Some(ttl_seconds)).await {
            tracing::error!("Failed to store refresh token in Redis: {e}");
        }
    }

    pub(super) async fn take_for_kind(
        &self,
        token: &str,
        expected_kind: &str,
    ) -> Result<RefreshEntry, RefreshTokenTakeResult> {
        let Ok(entry) = self.store.get::<RefreshEntry>(token).await else {
            tracing::error!("Failed to read refresh token from Redis");
            return Err(RefreshTokenTakeResult::Missing);
        };
        let Some(entry) = entry else {
            return Err(RefreshTokenTakeResult::Missing);
        };
        if entry.expires_at <= Utc::now().timestamp() {
            let _ = self.store.delete(token).await;
            return Err(RefreshTokenTakeResult::Missing);
        }
        if entry.token_kind != expected_kind {
            return Err(RefreshTokenTakeResult::WrongKind);
        }

        match self.store.take::<RefreshEntry>(token).await {
            Ok(Some(entry)) if entry.expires_at > Utc::now().timestamp() => Ok(entry),
            Ok(_) => Err(RefreshTokenTakeResult::Missing),
            Err(e) => {
                tracing::error!("Failed to take refresh token from Redis: {e}");
                Err(RefreshTokenTakeResult::Missing)
            }
        }
    }

    /// Revoke (delete) a refresh token if it matches the expected token kind.
    pub async fn revoke_for_kind(&self, token: &str, expected_kind: &str) {
        match self.store.get::<RefreshEntry>(token).await {
            Ok(Some(entry)) if entry.token_kind == expected_kind => {
                if let Err(e) = self.store.delete(token).await {
                    tracing::error!("Failed to revoke refresh token in Redis: {e}");
                }
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!("Failed to inspect refresh token in Redis: {e}");
            }
        }
    }
}

impl std::fmt::Debug for RedisRefreshTokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisRefreshTokenStore").finish()
    }
}

/// Refresh token store that can be either in-memory or Redis-backed.
///
/// In unit tests and single-instance deployments the in-memory variant is used.
/// For HA deployments (Kubernetes ReplicaSet), the Redis variant ensures all
/// replicas share the same token state.
#[derive(Debug)]
pub enum RefreshTokenStoreBackend {
    /// In-memory store (single-instance only).
    InMemory(RefreshTokenStore),
    /// Redis-backed store (multi-instance safe).
    Redis(RedisRefreshTokenStore),
}

impl RefreshTokenStoreBackend {
    /// Store a refresh token.
    pub async fn insert(
        &self,
        token: String,
        tenant_id: String,
        github_login: String,
        github_id: u64,
        email: Option<String>,
        token_kind: String,
        ttl_seconds: u64,
    ) {
        match self {
            Self::InMemory(s) => {
                s.insert(
                    token,
                    tenant_id,
                    github_login,
                    github_id,
                    email,
                    token_kind,
                    ttl_seconds,
                )
                .await;
            }
            Self::Redis(s) => {
                s.insert(
                    token,
                    tenant_id,
                    github_login,
                    github_id,
                    email,
                    token_kind,
                    ttl_seconds,
                )
                .await;
            }
        }
    }

    pub(super) async fn take_for_kind(
        &self,
        token: &str,
        expected_kind: &str,
    ) -> Result<RefreshEntry, RefreshTokenTakeResult> {
        match self {
            Self::InMemory(s) => s.take_for_kind(token, expected_kind).await,
            Self::Redis(s) => s.take_for_kind(token, expected_kind).await,
        }
    }

    /// Revoke a refresh token if it matches the expected kind.
    pub async fn revoke_for_kind(&self, token: &str, expected_kind: &str) {
        match self {
            Self::InMemory(s) => s.revoke_for_kind(token, expected_kind).await,
            Self::Redis(s) => s.revoke_for_kind(token, expected_kind).await,
        }
    }
}

impl Default for RefreshTokenStoreBackend {
    fn default() -> Self {
        Self::InMemory(RefreshTokenStore::new())
    }
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PluginAuthBootstrapRequest {
    pub provider: String,
    pub github_access_token: String,
}

#[derive(Debug, Deserialize)]
pub struct PluginAuthRefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize)]
pub struct PluginAuthLogoutRequest {
    pub refresh_token: Option<String>,
}

// ---------------------------------------------------------------------------
// JWT claims
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct PluginTokenClaims {
    pub sub: String,
    pub idp_provider: String,
    pub tenant_id: String,
    pub iss: String,
    pub aud: Vec<String>,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
    pub github_id: u64,
    pub email: Option<String>,
    pub kind: String,

    // ── B2 §10.1 — scoped-token foundation claims ────────────────────────
    //
    // `token_type` disambiguates *who minted the token* — a human user flow
    // (§10 §10.2–§10.4 issuance by PlatformAdmin), a service identity
    // (§10.2 `POST /api/v1/auth/tokens`), or a refresh token. It deliberately
    // does NOT overlap with `kind`, which captures the token's *audience
    // contract* ("plugin-access" gates /plugin endpoints).
    //
    // `scopes` carries the capability list the middleware in §10.5 checks
    // against each route's required scope. Empty list is the user-caller
    // fallback path (scope-required routes defer to the role check).
    //
    // Both fields are `#[serde(default)]` so JWTs minted by pre-§10.1
    // builds still decode without `{token_type,scopes}_missing` errors —
    // critical for rolling deploys where old and new pods coexist.
    /// Token-kind discriminator: `"user"` for human-auth tokens
    /// (current OAuth flows), `"service"` for PlatformAdmin-issued
    /// service identities (§10.2), `"refresh"` for refresh tokens.
    /// Defaults to `"user"` when absent so legacy tokens behave as
    /// user tokens.
    #[serde(default = "PluginTokenClaims::default_token_type")]
    pub token_type: String,

    /// List of capability scopes granted to this token. Middleware
    /// (§10.5) checks these against each route's required scope.
    /// For user tokens this is empty and the middleware falls back
    /// to role-based checks — matching the pre-§10 behaviour.
    #[serde(default)]
    pub scopes: Vec<String>,
}

impl PluginTokenClaims {
    pub const AUDIENCE: &'static str = "aeterna-plugin";
    pub const KIND: &'static str = "plugin-access";
    pub const ADMIN_UI_AUDIENCE: &'static str = "aeterna-admin-ui";
    pub const ADMIN_UI_KIND: &'static str = "admin-ui";

    /// Human-auth token produced by the OAuth flow. This is the
    /// default every pre-§10.1 token decodes as, matching what the
    /// system has always minted.
    pub const TOKEN_TYPE_USER: &'static str = "user";

    /// PlatformAdmin-issued service-identity token. Follow-up §10.2
    /// will be the first minter; this constant is the canonical value
    /// used there and by §10.5's middleware scope check.
    pub const TOKEN_TYPE_SERVICE: &'static str = "service";

    /// Refresh token used by the OAuth refresh flow. Reserved for
    /// future refresh-token audit trails.
    pub const TOKEN_TYPE_REFRESH: &'static str = "refresh";

    /// Serde default for `token_type` when absent from a legacy
    /// JWT payload. A free function (`fn() -> String`) is required
    /// by `#[serde(default = "…")]`; a `const &'static str` can't
    /// be passed directly.
    fn default_token_type() -> String {
        Self::TOKEN_TYPE_USER.to_string()
    }
}

// ---------------------------------------------------------------------------
// Validated identity
// ---------------------------------------------------------------------------

/// Validated identity extracted from a plugin bearer token.
#[derive(Debug, Clone)]
pub struct PluginIdentity {
    pub idp_provider: String,
    pub tenant_id: String,
    pub github_login: String,
    pub github_id: u64,
    pub email: Option<String>,
    pub jti: String,
    pub kind: String,

    // ── B2 §10.1 — scope-check inputs for middleware (§10.5) ─────────
    /// Token-kind discriminator copied from the JWT `token_type` claim.
    /// Downstream authorization (§10.5) distinguishes `"user"` tokens —
    /// which defer to role checks — from `"service"` tokens, whose
    /// authority is expressed purely through `scopes`.
    pub token_type: String,

    /// Capability scopes carried by the token (§10.5). Empty for
    /// user tokens; populated for service tokens in §10.2.
    pub scopes: Vec<String>,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/auth/plugin/bootstrap", post(bootstrap_handler))
        .route("/auth/plugin/refresh", post(refresh_handler))
        .route("/auth/plugin/logout", post(logout_handler))
        .route("/auth/admin-ui/bootstrap", post(admin_ui_bootstrap_handler))
        .route("/auth/admin-ui/refresh", post(admin_ui_refresh_handler))
        .route("/auth/admin-ui/revoke", post(admin_ui_revoke_handler))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn plugin_access_ttls(cfg: &config::PluginAuthConfig) -> (u64, u64) {
    (
        cfg.access_token_ttl_seconds.unwrap_or(3600),
        cfg.refresh_token_ttl_seconds
            .unwrap_or(DEFAULT_REFRESH_TOKEN_TTL_SECS),
    )
}

fn admin_ui_ttls() -> (u64, u64) {
    (
        DEFAULT_ADMIN_UI_ACCESS_TOKEN_TTL_SECS,
        DEFAULT_ADMIN_UI_REFRESH_TOKEN_TTL_SECS,
    )
}

fn mint_access_token_for_kind(
    jwt_secret: &str,
    issuer: &str,
    tenant_id: &str,
    user: &GitHubUser,
    ttl_seconds: u64,
    audience: &str,
    kind: &str,
) -> anyhow::Result<String> {
    mint_access_token(
        jwt_secret,
        issuer,
        tenant_id,
        user,
        ttl_seconds,
        audience,
        kind,
    )
}

async fn bootstrap_for_kind(
    state: Arc<AppState>,
    req: PluginAuthBootstrapRequest,
    audience: &str,
    kind: &str,
    access_ttl: u64,
    refresh_ttl: u64,
) -> axum::response::Response {
    let cfg = &state.plugin_auth_state.config;

    if !cfg.enabled {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "plugin_auth_disabled",
                "message": "Plugin authentication is not enabled on this server"
            })),
        )
            .into_response();
    }

    if !cfg.allowed_providers.contains(&req.provider) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "unsupported_provider",
                "message": "Provider not supported"
            })),
        )
            .into_response();
    }

    let jwt_secret = match &cfg.jwt_secret {
        Some(s) => s.clone(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "configuration_error",
                    "message": "JWT secret is not configured"
                })),
            )
                .into_response();
        }
    };

    let github_user = match fetch_github_user(&req.github_access_token, cfg).await {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!("GitHub user fetch failed: {e}");
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "github_user_fetch_failed",
                    "message": format!("Failed to fetch GitHub user: {e}")
                })),
            )
                .into_response();
        }
    };

    let issuer = cfg
        .token_issuer
        .clone()
        .unwrap_or_else(|| "aeterna".to_string());
    let tenant_id = if let Some(t) = resolve_tenant_for_github_user(&github_user.login, cfg) {
        t
    } else {
        tracing::error!(login = %github_user.login, "No tenant configured for auth bootstrap");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "tenant_not_configured",
                "message": "No tenant configured for authentication. Set AETERNA_DEFAULT_TENANT_ID or configure default_tenant_id."
            })),
        )
            .into_response();
    };

    let access_token = match mint_access_token_for_kind(
        &jwt_secret,
        &issuer,
        &tenant_id,
        &github_user,
        access_ttl,
        audience,
        kind,
    ) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to mint access token: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "token_mint_failed",
                    "message": "Failed to issue access token"
                })),
            )
                .into_response();
        }
    };

    let refresh_token = Uuid::new_v4().to_string();
    state
        .plugin_auth_state
        .refresh_store
        .insert(
            refresh_token.clone(),
            tenant_id,
            github_user.login.clone(),
            github_user.id,
            github_user.email.clone(),
            kind.to_string(),
            refresh_ttl,
        )
        .await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "access_token": access_token,
            "refresh_token": refresh_token,
            "expires_in": access_ttl,
            "token_type": "Bearer",
            "github_login": github_user.login,
            "github_email": github_user.email,
            "kind": kind,
        })),
    )
        .into_response()
}

async fn refresh_for_kind(
    state: Arc<AppState>,
    req: PluginAuthRefreshRequest,
    audience: &str,
    kind: &str,
    access_ttl: u64,
    refresh_ttl: u64,
) -> axum::response::Response {
    let cfg = &state.plugin_auth_state.config;

    if !cfg.enabled {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "plugin_auth_disabled",
                "message": "Plugin authentication is not enabled on this server"
            })),
        )
            .into_response();
    }

    let jwt_secret = match &cfg.jwt_secret {
        Some(s) => s.clone(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "configuration_error",
                    "message": "JWT secret is not configured"
                })),
            )
                .into_response();
        }
    };

    let entry = match state
        .plugin_auth_state
        .refresh_store
        .take_for_kind(&req.refresh_token, kind)
        .await
    {
        Ok(e) => e,
        Err(RefreshTokenTakeResult::WrongKind) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "wrong_audience",
                    "message": "Refresh token belongs to a different token audience"
                })),
            )
                .into_response();
        }
        Err(RefreshTokenTakeResult::Missing) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "invalid_refresh_token",
                    "message": "Refresh token is invalid, expired, or already consumed"
                })),
            )
                .into_response();
        }
    };

    let issuer = cfg
        .token_issuer
        .clone()
        .unwrap_or_else(|| "aeterna".to_string());
    let user_info = GitHubUser {
        id: entry.github_id,
        login: entry.github_login,
        email: entry.email,
    };

    let access_token = match mint_access_token_for_kind(
        &jwt_secret,
        &issuer,
        &entry.tenant_id,
        &user_info,
        access_ttl,
        audience,
        kind,
    ) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to mint access token on refresh: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "token_mint_failed",
                    "message": "Failed to issue access token"
                })),
            )
                .into_response();
        }
    };

    let new_refresh = Uuid::new_v4().to_string();
    state
        .plugin_auth_state
        .refresh_store
        .insert(
            new_refresh.clone(),
            entry.tenant_id,
            user_info.login.clone(),
            user_info.id,
            user_info.email.clone(),
            kind.to_string(),
            refresh_ttl,
        )
        .await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "access_token": access_token,
            "refresh_token": new_refresh,
            "expires_in": access_ttl,
            "token_type": "Bearer",
            "github_login": user_info.login,
            "github_email": user_info.email,
            "kind": kind,
        })),
    )
        .into_response()
}

async fn revoke_for_kind(
    state: Arc<AppState>,
    req: PluginAuthLogoutRequest,
    kind: &str,
) -> axum::response::Response {
    if let Some(token) = &req.refresh_token {
        state
            .plugin_auth_state
            .refresh_store
            .revoke_for_kind(token, kind)
            .await;
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({ "message": "Logged out successfully" })),
    )
        .into_response()
}

#[tracing::instrument(skip_all, fields(provider = %req.provider))]
async fn bootstrap_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PluginAuthBootstrapRequest>,
) -> impl IntoResponse {
    let (access_ttl, refresh_ttl) = plugin_access_ttls(&state.plugin_auth_state.config);
    bootstrap_for_kind(
        state,
        req,
        PluginTokenClaims::AUDIENCE,
        PluginTokenClaims::KIND,
        access_ttl,
        refresh_ttl,
    )
    .await
}

#[tracing::instrument(skip_all)]
async fn refresh_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PluginAuthRefreshRequest>,
) -> impl IntoResponse {
    let (access_ttl, refresh_ttl) = plugin_access_ttls(&state.plugin_auth_state.config);
    refresh_for_kind(
        state,
        req,
        PluginTokenClaims::AUDIENCE,
        PluginTokenClaims::KIND,
        access_ttl,
        refresh_ttl,
    )
    .await
}

#[tracing::instrument(skip_all)]
async fn logout_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PluginAuthLogoutRequest>,
) -> impl IntoResponse {
    revoke_for_kind(state, req, PluginTokenClaims::KIND).await
}

#[tracing::instrument(skip_all, fields(provider = %req.provider))]
async fn admin_ui_bootstrap_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PluginAuthBootstrapRequest>,
) -> impl IntoResponse {
    let (access_ttl, refresh_ttl) = admin_ui_ttls();
    bootstrap_for_kind(
        state,
        req,
        PluginTokenClaims::ADMIN_UI_AUDIENCE,
        PluginTokenClaims::ADMIN_UI_KIND,
        access_ttl,
        refresh_ttl,
    )
    .await
}

#[tracing::instrument(skip_all)]
async fn admin_ui_refresh_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PluginAuthRefreshRequest>,
) -> impl IntoResponse {
    let (access_ttl, refresh_ttl) = admin_ui_ttls();
    refresh_for_kind(
        state,
        req,
        PluginTokenClaims::ADMIN_UI_AUDIENCE,
        PluginTokenClaims::ADMIN_UI_KIND,
        access_ttl,
        refresh_ttl,
    )
    .await
}

#[tracing::instrument(skip_all)]
async fn admin_ui_revoke_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PluginAuthLogoutRequest>,
) -> impl IntoResponse {
    revoke_for_kind(state, req, PluginTokenClaims::ADMIN_UI_KIND).await
}

// ---------------------------------------------------------------------------
// Admin session endpoint (returns user profile + roles + tenant memberships)
// ---------------------------------------------------------------------------

/// Router for the admin session convenience endpoint.
///
/// This is registered inside the protected API route group (requires bearer
/// token). It returns the authenticated user's profile, roles across all
/// tenants (including `__root__` PlatformAdmin grants), and tenant memberships
/// in a single response for efficient admin UI bootstrap.
///
/// Routes:
/// - `POST /auth/admin/session` — Legacy endpoint for OpenCode plugin consumers.
/// - `GET  /auth/session`       — Used by the admin UI after OAuth redirect to
///   validate the token and fetch the session profile.
pub fn admin_session_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/auth/admin/session", post(admin_session_handler))
        .route("/auth/session", get(admin_session_handler))
        .with_state(state)
}

#[tracing::instrument(skip_all)]
async fn admin_session_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let cfg = &state.plugin_auth_state.config;

    let jwt_secret = match &cfg.jwt_secret {
        Some(s) => s.clone(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "configuration_error",
                    "message": "JWT secret is not configured"
                })),
            );
        }
    };

    let identity = match validate_plugin_bearer(&headers, &jwt_secret) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "unauthorized",
                    "message": "Valid bearer token required"
                })),
            );
        }
    };

    let user_id = match state
        .postgres
        .resolve_user_id_by_idp_bootstrap(&identity.idp_provider, &identity.github_login)
        .await
    {
        Ok(Some(id)) => id,
        Ok(None) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "identity_not_provisioned",
                    "message": "GitHub identity is not provisioned in this Aeterna instance"
                })),
            );
        }
        Err(e) => {
            tracing::error!("Failed to resolve user identity: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "internal_error",
                    "message": "Failed to resolve user identity"
                })),
            );
        }
    };

    let roles = match state
        .postgres
        .get_user_roles_for_auth_bootstrap(&user_id, &identity.tenant_id)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to fetch user roles: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "internal_error",
                    "message": "Failed to fetch user roles"
                })),
            );
        }
    };

    let is_platform_admin = roles
        .iter()
        .any(|r| r == &RoleIdentifier::Known(mk_core::types::Role::PlatformAdmin));
    let is_tenant_admin = roles
        .iter()
        .any(|r| r == &RoleIdentifier::Known(mk_core::types::Role::TenantAdmin));

    let default_tenant_id = state
        .postgres
        .get_user_default_tenant_bootstrap(&user_id)
        .await
        .ok()
        .flatten();
    let default_tenant_slug = match default_tenant_id.as_deref() {
        Some(tid) => match state.tenant_store.get_tenant(tid).await {
            Ok(Some(t)) => Some(t.slug),
            _ => None,
        },
        None => None,
    };

    let tenants = if is_platform_admin {
        match state.tenant_store.list_tenants(false).await {
            Ok(t) => t
                .into_iter()
                .map(|t| {
                    serde_json::json!({
                        "id": t.id.as_str(),
                        "slug": t.slug,
                        "name": t.name,
                        "status": format!("{:?}", t.status),
                    })
                })
                .collect::<Vec<_>>(),
            Err(_) => vec![],
        }
    } else {
        match state.postgres.get_user_tenant_ids_bootstrap(&user_id).await {
            Ok(ids) => ids
                .into_iter()
                .map(|id| {
                    serde_json::json!({
                        "id": id,
                        "slug": id,
                        "name": id,
                        "status": "Active",
                    })
                })
                .collect::<Vec<_>>(),
            Err(_) => vec![],
        }
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "user": {
                "user_id": user_id,
                "github_login": identity.github_login,
                "github_id": identity.github_id,
                "email": identity.email,
            },
            "roles": roles,
            "tenants": tenants,
            "is_platform_admin": is_platform_admin,
            "is_tenant_admin": is_tenant_admin,
            "active_tenant_id": identity.tenant_id,
            // #44.b: surface the persistent preference so the admin UI can
            // restore the tenant selector on load without an extra call.
            "default_tenant_id": default_tenant_id,
            "default_tenant_slug": default_tenant_slug,
        })),
    )
}

// ---------------------------------------------------------------------------
// Token validation (used by request extractors elsewhere)
// ---------------------------------------------------------------------------

/// Validate a plugin bearer token from an Authorization header.
pub fn validate_plugin_bearer(headers: &HeaderMap, jwt_secret: &str) -> Option<PluginIdentity> {
    let auth_header = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let token = auth_header.strip_prefix("Bearer ")?;
    validate_plugin_token(token, jwt_secret)
}

/// Validate a raw plugin JWT and return the embedded identity.
pub fn validate_plugin_token(token: &str, jwt_secret: &str) -> Option<PluginIdentity> {
    let key = DecodingKey::from_secret(jwt_secret.as_bytes());
    let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.set_audience(&[
        PluginTokenClaims::AUDIENCE,
        PluginTokenClaims::ADMIN_UI_AUDIENCE,
    ]);
    validation.iss = None;

    match decode::<PluginTokenClaims>(token, &key, &validation) {
        Ok(data)
            if data.claims.kind == PluginTokenClaims::KIND
                || data.claims.kind == PluginTokenClaims::ADMIN_UI_KIND =>
        {
            Some(PluginIdentity {
                idp_provider: data.claims.idp_provider,
                tenant_id: data.claims.tenant_id,
                github_login: data.claims.sub,
                github_id: data.claims.github_id,
                email: data.claims.email,
                jti: data.claims.jti,
                kind: data.claims.kind,
                token_type: data.claims.token_type,
                scopes: data.claims.scopes,
            })
        }
        Ok(_) => {
            tracing::debug!("Bearer token rejected: unexpected kind");
            None
        }
        Err(e) => {
            tracing::debug!("Bearer token validation failed: {e}");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tenant context derivation
// ---------------------------------------------------------------------------

/// Derive a `TenantContext` from a plugin bearer token.
///
/// If the `Authorization: Bearer <token>` header is present and the token
/// validates, the GitHub login and tenant claim are used as the caller identity.
///
/// Returns `None` when no valid bearer token is present, allowing callers to
/// enforce fail-closed behaviour rather than silently falling back to a synthetic
/// default context.  Use [`tenant_context_from_plugin_bearer_or_default`] only in
/// contexts where an unauthenticated / development fallback is explicitly intended.
pub fn tenant_context_from_plugin_bearer(
    headers: &HeaderMap,
    jwt_secret: &str,
) -> Option<TenantContext> {
    let identity = validate_plugin_bearer(headers, jwt_secret)?;
    let tenant = sanitize_identifier(&identity.tenant_id, "");
    let login = identity.github_login.chars().take(100).collect::<String>();
    if let (Some(tenant_id), Some(user_id)) = (TenantId::new(tenant), UserId::new(login)) {
        Some(TenantContext::new(tenant_id, user_id))
    } else {
        tracing::warn!("Plugin token carried invalid tenant_id or user_id; rejecting context");
        None
    }
}

/// Derive a `TenantContext` from a plugin bearer token, falling back to a
/// synthetic `"default" / "system"` context when no valid token is present.
///
/// **This fallback is intentional for development/service-to-service mode only.**
/// It MUST NOT be used when plugin auth is enabled in a production-capable
/// deployment; use [`tenant_context_from_plugin_bearer`] and enforce the `None`
/// case explicitly.
pub fn tenant_context_from_plugin_bearer_or_default(
    headers: &HeaderMap,
    jwt_secret: &str,
) -> TenantContext {
    if let Some(ctx) = tenant_context_from_plugin_bearer(headers, jwt_secret) {
        return ctx;
    }
    tracing::debug!(
        "No valid plugin bearer token found; using unauthenticated dev context (default/system).          This fallback must only occur in development or service-to-service mode."
    );
    TenantContext::new(
        TenantId::new(DEFAULT_TENANT_SLUG.to_string()).expect("static tenant id"),
        UserId::new(SYSTEM_USER_ID.to_string()).expect("static user id"),
    )
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct GitHubUser {
    id: u64,
    login: String,
    email: Option<String>,
}

#[derive(Deserialize)]
struct GitHubOAuthTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct GitHubUserResponse {
    id: u64,
    login: String,
    email: Option<String>,
}

async fn fetch_github_user(
    github_token: &str,
    cfg: &config::PluginAuthConfig,
) -> anyhow::Result<GitHubUser> {
    let client = reqwest::Client::new();
    let api_base = cfg
        .github_api_base_url
        .as_deref()
        .unwrap_or(DEFAULT_GITHUB_API_BASE);
    let resp = client
        .get(format!("{api_base}/user"))
        .header("Authorization", format!("Bearer {github_token}"))
        .header("User-Agent", "aeterna-plugin-auth/1.0")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?;

    let user: GitHubUserResponse = resp.json().await?;
    let email = match user.email {
        Some(email) => Some(email),
        None => fetch_github_primary_email(&client, github_token, cfg).await?,
    };

    Ok(GitHubUser {
        id: user.id,
        login: user.login,
        email,
    })
}

fn mint_access_token(
    jwt_secret: &str,
    issuer: &str,
    tenant_id: &str,
    user: &GitHubUser,
    ttl_seconds: u64,
    audience: &str,
    kind: &str,
) -> anyhow::Result<String> {
    let now = Utc::now().timestamp();
    let claims = PluginTokenClaims {
        sub: user.login.clone(),
        idp_provider: PROVIDER_GITHUB.to_string(),
        tenant_id: tenant_id.to_string(),
        iss: issuer.to_string(),
        aud: vec![audience.to_string()],
        iat: now,
        exp: now + ttl_seconds as i64,
        jti: Uuid::new_v4().to_string(),
        github_id: user.id,
        email: user.email.clone(),
        kind: kind.to_string(),
        // Human OAuth flow → `"user"`. Authorization (§10.5) defers to
        // role checks for user tokens; `scopes` is intentionally empty.
        token_type: PluginTokenClaims::TOKEN_TYPE_USER.to_string(),
        scopes: Vec::new(),
    };

    let key = EncodingKey::from_secret(jwt_secret.as_bytes());
    Ok(encode(
        &Header::new(jsonwebtoken::Algorithm::HS256),
        &claims,
        &key,
    )?)
}

#[derive(Debug, Deserialize)]
struct GitHubEmailResponse {
    email: String,
    primary: bool,
    verified: bool,
}

async fn fetch_github_primary_email(
    client: &reqwest::Client,
    github_token: &str,
    cfg: &config::PluginAuthConfig,
) -> anyhow::Result<Option<String>> {
    let api_base = cfg
        .github_api_base_url
        .as_deref()
        .unwrap_or(DEFAULT_GITHUB_API_BASE);
    let resp = client
        .get(format!("{api_base}/user/emails"))
        .header("Authorization", format!("Bearer {github_token}"))
        .header("User-Agent", "aeterna-plugin-auth/1.0")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?;

    let emails: Vec<GitHubEmailResponse> = resp.json().await?;
    Ok(emails
        .into_iter()
        .find(|email| email.primary && email.verified)
        .map(|email| email.email))
}

/// Resolve a tenant ID for the given GitHub login using the plugin auth config.
///
/// Returns  if no tenant mapping is configured, which callers MUST treat
/// as a fail-closed condition (reject the request).
///
/// Resolution order:
/// 1.  map entry for this specific login (not yet supported in
///    config schema; reserved for future per-login routing).
/// 2.  from the plugin auth config.
/// 3.  environment variable (operator override for
///    simple single-tenant deployments without full config reload).
///
/// The synthetic hardcoded  string is intentionally NOT in this list.
pub(crate) fn resolve_tenant_for_github_user(
    _github_login: &str,
    cfg: &config::PluginAuthConfig,
) -> Option<String> {
    // Priority 1: explicit per-login mapping (config schema extension point).
    // Not yet wired; reserved.

    // Priority 2: default_tenant_id from config.
    if let Some(ref tenant_id) = cfg.default_tenant_id {
        let trimmed = tenant_id.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    // Priority 3: env override for operator convenience.
    if let Ok(env_tenant) = std::env::var("AETERNA_DEFAULT_TENANT_ID") {
        let trimmed = env_tenant.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }

    None
}

fn sanitize_identifier(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.chars().take(100).collect()
    }
}

const DEFAULT_GITHUB_OAUTH_BASE: &str = "https://github.com";

const OAUTH_STATE_TTL_SECS: usize = 600;

fn oauth_state_redis_key(state: &str) -> String {
    format!("oauth:state:{state}")
}

async fn oauth_state_insert(
    redis_conn: &mut redis::aio::ConnectionManager,
    state: &str,
) -> Result<(), ()> {
    let key = oauth_state_redis_key(state);
    let result: Option<String> = redis::cmd("SET")
        .arg(&key)
        .arg("1")
        .arg("EX")
        .arg(OAUTH_STATE_TTL_SECS)
        .arg("NX")
        .query_async(redis_conn)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Redis SET failed for OAuth state");
        })?;
    if result.is_some() {
        Ok(())
    } else {
        tracing::warn!(state = %state, "OAuth state key already exists in Redis");
        Err(())
    }
}

async fn oauth_state_consume(redis_conn: &mut redis::aio::ConnectionManager, state: &str) -> bool {
    let key = oauth_state_redis_key(state);
    let deleted: i64 = match redis::cmd("DEL").arg(&key).query_async(redis_conn).await {
        Ok(n) => n,
        Err(e) => {
            tracing::error!(error = %e, "Redis DEL failed for OAuth state");
            return false;
        }
    };
    deleted == 1
}

#[derive(Debug, Deserialize)]
struct WebCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

pub fn web_oauth_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/auth/web/authorize", get(web_authorize_handler))
        .route("/auth/web/callback", get(web_callback_handler))
        .with_state(state)
}

async fn web_authorize_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cfg = &state.plugin_auth_state.config;

    if !cfg.enabled {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "plugin_auth_disabled",
                "message": "Plugin authentication is not enabled on this server"
            })),
        ));
    }

    let client_id = match &cfg.github_client_id {
        Some(id) => id.clone(),
        None => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "configuration_error",
                    "message": "GitHub OAuth client ID is not configured"
                })),
            ));
        }
    };

    let oauth_base = cfg
        .github_oauth_base_url
        .as_deref()
        .unwrap_or(DEFAULT_GITHUB_OAUTH_BASE);

    let mut redis_conn = match state.redis_conn.as_ref() {
        Some(c) => c.as_ref().clone(),
        None => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "configuration_error",
                    "message": "Redis is not available"
                })),
            ));
        }
    };

    let csrf_state = Uuid::new_v4().to_string();
    if oauth_state_insert(&mut redis_conn, &csrf_state)
        .await
        .is_err()
    {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "state_error",
                "message": "Failed to store OAuth state"
            })),
        ));
    }

    let redirect_url = format!(
        "{oauth_base}/login/oauth/authorize?client_id={client_id}&state={csrf_state}&scope=read:user,user:email"
    );

    Ok(Redirect::to(&redirect_url))
}

async fn web_callback_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<WebCallbackQuery>,
) -> impl IntoResponse {
    let cfg = &state.plugin_auth_state.config;

    let redirect_base = cfg
        .redirect_base_url
        .as_deref()
        .unwrap_or("")
        .trim_end_matches('/');
    let error_redirect = format!("{redirect_base}/admin/login");

    if let Some(err) = params.error {
        let desc = params.error_description.unwrap_or_default();
        tracing::warn!(error = %err, description = %desc, "GitHub OAuth callback error");
        return Redirect::to(&format!("{error_redirect}?error={err}"));
    }

    let code = match params.code {
        Some(c) if !c.is_empty() => c,
        _ => {
            tracing::warn!("Web callback received without code");
            return Redirect::to(&format!("{error_redirect}?error=missing_code"));
        }
    };

    let incoming_state = match params.state {
        Some(s) if !s.is_empty() => s,
        _ => {
            tracing::warn!("Web callback received without state");
            return Redirect::to(&format!("{error_redirect}?error=missing_state"));
        }
    };

    let state_valid = if let Some(c) = state.redis_conn.as_ref() {
        let mut conn = c.as_ref().clone();
        oauth_state_consume(&mut conn, &incoming_state).await
    } else {
        tracing::error!("Redis not available for OAuth state validation");
        false
    };

    if !state_valid {
        tracing::warn!("Web callback state mismatch or expired");
        return Redirect::to(&format!("{error_redirect}?error=invalid_state"));
    }

    let client_id = if let Some(id) = &cfg.github_client_id {
        id.clone()
    } else {
        tracing::error!("GitHub OAuth client ID not configured");
        return Redirect::to(&format!("{error_redirect}?error=configuration_error"));
    };

    let client_secret = if let Some(s) = &cfg.github_client_secret {
        s.clone()
    } else {
        tracing::error!("GitHub OAuth client secret not configured");
        return Redirect::to(&format!("{error_redirect}?error=configuration_error"));
    };

    let jwt_secret = if let Some(s) = &cfg.jwt_secret {
        s.clone()
    } else {
        tracing::error!("JWT secret not configured");
        return Redirect::to(&format!("{error_redirect}?error=configuration_error"));
    };

    let github_access_token =
        match exchange_github_code(&code, &client_id, &client_secret, cfg).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("GitHub code exchange failed: {e}");
                return Redirect::to(&format!("{error_redirect}?error=token_exchange_failed"));
            }
        };

    let github_user = match fetch_github_user(&github_access_token, cfg).await {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!("GitHub user fetch failed: {e}");
            return Redirect::to(&format!("{error_redirect}?error=user_fetch_failed"));
        }
    };

    let (access_ttl, refresh_ttl) = admin_ui_ttls();
    let issuer = cfg
        .token_issuer
        .clone()
        .unwrap_or_else(|| "aeterna".to_string());

    let tenant_id = if let Some(t) = resolve_tenant_for_github_user(&github_user.login, cfg) {
        t
    } else {
        tracing::error!(login = %github_user.login, "No tenant configured for web OAuth login");
        return Redirect::to(&format!("{error_redirect}?error=tenant_not_configured"));
    };

    let access_token = match mint_access_token(
        &jwt_secret,
        &issuer,
        &tenant_id,
        &github_user,
        access_ttl,
        PluginTokenClaims::ADMIN_UI_AUDIENCE,
        PluginTokenClaims::ADMIN_UI_KIND,
    ) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to mint access token: {e}");
            return Redirect::to(&format!("{error_redirect}?error=token_mint_failed"));
        }
    };

    let refresh_token = Uuid::new_v4().to_string();
    state
        .plugin_auth_state
        .refresh_store
        .insert(
            refresh_token.clone(),
            tenant_id,
            github_user.login.clone(),
            github_user.id,
            github_user.email.clone(),
            PluginTokenClaims::ADMIN_UI_KIND.to_string(),
            refresh_ttl,
        )
        .await;

    let encoded_access = urlencoding_encode(&access_token);
    let encoded_refresh = urlencoding_encode(&refresh_token);
    let encoded_login = urlencoding_encode(&github_user.login);
    let expires_in = access_ttl;

    let admin_url = format!(
        "{redirect_base}/admin/login#access_token={encoded_access}&refresh_token={encoded_refresh}&expires_in={expires_in}&github_login={encoded_login}"
    );

    Redirect::to(&admin_url)
}

async fn exchange_github_code(
    code: &str,
    client_id: &str,
    client_secret: &str,
    cfg: &config::PluginAuthConfig,
) -> anyhow::Result<String> {
    let oauth_base = cfg
        .github_oauth_base_url
        .as_deref()
        .unwrap_or(DEFAULT_GITHUB_OAUTH_BASE);

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{oauth_base}/login/oauth/access_token"))
        .header("Accept", "application/json")
        .header("User-Agent", "aeterna-web-auth/1.0")
        .json(&serde_json::json!({
            "client_id": client_id,
            "client_secret": client_secret,
            "code": code,
        }))
        .send()
        .await?
        .error_for_status()?;

    let token_resp: GitHubOAuthTokenResponse = resp.json().await?;

    if let Some(err) = token_resp.error {
        let desc = token_resp.error_description.unwrap_or_default();
        anyhow::bail!("GitHub token exchange error: {err} — {desc}");
    }

    token_resp
        .access_token
        .ok_or_else(|| anyhow::anyhow!("GitHub response missing access_token"))
}

fn urlencoding_encode(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                vec![c]
            } else {
                format!("%{:02X}", c as u32).chars().collect()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn secret() -> String {
        "super-secret-test-key-at-least-32-chars".to_string()
    }

    fn user() -> GitHubUser {
        GitHubUser {
            id: 42,
            login: "testuser".to_string(),
            email: Some("testuser@example.com".to_string()),
        }
    }

    fn mint_plugin_token(tenant_id: &str) -> String {
        mint_access_token(
            &secret(),
            "aeterna",
            tenant_id,
            &user(),
            3600,
            PluginTokenClaims::AUDIENCE,
            PluginTokenClaims::KIND,
        )
        .unwrap()
    }

    fn mint_admin_ui_token(tenant_id: &str) -> String {
        mint_access_token(
            &secret(),
            "aeterna",
            tenant_id,
            &user(),
            1800,
            PluginTokenClaims::ADMIN_UI_AUDIENCE,
            PluginTokenClaims::ADMIN_UI_KIND,
        )
        .unwrap()
    }

    #[test]
    fn mint_and_validate_roundtrip() {
        let token = mint_plugin_token("default");
        let identity = validate_plugin_token(&token, &secret()).unwrap();
        assert_eq!(identity.tenant_id, "default");
        assert_eq!(identity.github_login, "testuser");
        assert_eq!(identity.github_id, 42);
        assert_eq!(identity.email.as_deref(), Some("testuser@example.com"));
        assert!(!identity.jti.is_empty());
    }

    // ── B2 §10.1 — scoped-token claim coverage ──────────────────────────

    /// OAuth flow mints tokens as `token_type="user"` with an empty
    /// `scopes` list. Middleware (§10.5) relies on this contract to
    /// distinguish user tokens (defer to role check) from service
    /// tokens (evaluate scopes).
    #[test]
    fn minted_user_token_has_user_type_and_empty_scopes() {
        let token = mint_plugin_token("default");
        let identity = validate_plugin_token(&token, &secret()).unwrap();
        assert_eq!(identity.kind, PluginTokenClaims::KIND);
        assert_eq!(identity.token_type, PluginTokenClaims::TOKEN_TYPE_USER);
        assert!(
            identity.scopes.is_empty(),
            "user tokens must not carry scopes; got {:?}",
            identity.scopes
        );
    }

    #[test]
    fn minted_admin_ui_token_round_trips_with_distinct_kind() {
        let token = mint_admin_ui_token("default");
        let identity = validate_plugin_token(&token, &secret()).unwrap();
        assert_eq!(identity.kind, PluginTokenClaims::ADMIN_UI_KIND);
        assert_eq!(identity.tenant_id, "default");
        assert_eq!(identity.github_login, "testuser");
    }

    /// Rolling deploy guarantee: a JWT minted by a pre-§10.1 build
    /// (no `token_type`, no `scopes` in payload) MUST still decode,
    /// and MUST surface the documented defaults — `"user"` and `[]`.
    /// Without this, any cluster with mixed pod versions would 401
    /// half its traffic during the rollout.
    #[test]
    fn validate_legacy_token_without_new_claims_defaults_to_user() {
        use jsonwebtoken::{EncodingKey, Header};

        // Minimal payload that mirrors the pre-§10.1 `PluginTokenClaims`
        // shape — deliberately omits `token_type` and `scopes`.
        #[derive(Serialize)]
        struct LegacyClaims<'a> {
            sub: &'a str,
            idp_provider: &'a str,
            tenant_id: &'a str,
            iss: &'a str,
            aud: Vec<&'a str>,
            iat: i64,
            exp: i64,
            jti: &'a str,
            github_id: u64,
            email: Option<&'a str>,
            kind: &'a str,
        }

        let now = Utc::now().timestamp();
        let legacy = LegacyClaims {
            sub: "legacy-user",
            idp_provider: PROVIDER_GITHUB,
            tenant_id: "default",
            iss: "aeterna",
            aud: vec![PluginTokenClaims::AUDIENCE],
            iat: now,
            exp: now + 3600,
            jti: "legacy-jti",
            github_id: 7,
            email: Some("legacy@example.com"),
            kind: PluginTokenClaims::KIND,
        };
        let token = jsonwebtoken::encode(
            &Header::new(jsonwebtoken::Algorithm::HS256),
            &legacy,
            &EncodingKey::from_secret(secret().as_bytes()),
        )
        .unwrap();

        let identity = validate_plugin_token(&token, &secret())
            .expect("legacy token without token_type/scopes must still decode");
        assert_eq!(identity.token_type, PluginTokenClaims::TOKEN_TYPE_USER);
        assert!(identity.scopes.is_empty());
        assert_eq!(identity.github_login, "legacy-user");
    }

    /// Forward-compat: if a token is minted with an explicit
    /// `token_type="service"` and non-empty `scopes`, both values
    /// survive the decode round-trip verbatim. §10.2 will be the
    /// first real caller; this locks the contract in now.
    #[test]
    fn service_token_with_scopes_round_trips() {
        use jsonwebtoken::{EncodingKey, Header};

        let now = Utc::now().timestamp();
        let claims = PluginTokenClaims {
            sub: "svc-ci-runner".to_string(),
            idp_provider: "service".to_string(),
            tenant_id: "tenant-a".to_string(),
            iss: "aeterna".to_string(),
            aud: vec![PluginTokenClaims::AUDIENCE.to_string()],
            iat: now,
            exp: now + 3600,
            jti: "svc-jti".to_string(),
            github_id: 0,
            email: None,
            kind: PluginTokenClaims::KIND.to_string(),
            token_type: PluginTokenClaims::TOKEN_TYPE_SERVICE.to_string(),
            scopes: vec!["tenants:read".to_string(), "agents:write".to_string()],
        };
        let token = jsonwebtoken::encode(
            &Header::new(jsonwebtoken::Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret().as_bytes()),
        )
        .unwrap();

        let identity = validate_plugin_token(&token, &secret()).unwrap();
        assert_eq!(identity.token_type, PluginTokenClaims::TOKEN_TYPE_SERVICE);
        assert_eq!(identity.scopes, vec!["tenants:read", "agents:write"]);
    }

    #[test]
    fn validate_rejects_wrong_secret() {
        let token = mint_plugin_token("default");
        assert!(validate_plugin_token(&token, "wrong-secret").is_none());
    }

    #[test]
    fn validate_rejects_tampered_token() {
        let token = mint_plugin_token("default");
        let tampered = format!("{token}x");
        assert!(validate_plugin_token(&tampered, &secret()).is_none());
    }

    #[test]
    fn validate_bearer_header_missing_returns_none() {
        let headers = HeaderMap::new();
        assert!(validate_plugin_bearer(&headers, &secret()).is_none());
    }

    #[test]
    fn validate_bearer_header_extracts_identity() {
        let token = mint_plugin_token("default");
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
        let identity = validate_plugin_bearer(&headers, &secret()).unwrap();
        assert_eq!(identity.github_login, "testuser");
    }

    #[test]
    fn bootstrap_request_uses_github_access_token_contract() {
        let req: PluginAuthBootstrapRequest = serde_json::from_value(serde_json::json!({
            "provider": "github",
            "github_access_token": "gho_123"
        }))
        .unwrap();

        assert_eq!(req.provider, "github");
        assert_eq!(req.github_access_token, "gho_123");
    }

    #[test]
    fn bootstrap_request_rejects_legacy_code_shape() {
        let req = serde_json::from_value::<PluginAuthBootstrapRequest>(serde_json::json!({
            "provider": "github",
            "code": "legacy-code"
        }));

        assert!(req.is_err());
    }

    #[test]
    fn tenant_context_uses_tenant_claim_from_bearer() {
        let token = mint_plugin_token("tenant-42");
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {token}").parse().unwrap());

        let ctx = tenant_context_from_plugin_bearer(&headers, &secret()).unwrap();
        assert_eq!(ctx.tenant_id.as_str(), "tenant-42");
        assert_eq!(ctx.user_id.as_str(), "testuser");
    }

    #[tokio::test]
    async fn refresh_store_roundtrip() {
        let store = RefreshTokenStore::new();
        store
            .insert(
                "tok".to_string(),
                "default".to_string(),
                "alice".to_string(),
                1,
                None,
                PluginTokenClaims::KIND.to_string(),
                3600,
            )
            .await;

        let entry = store
            .take_for_kind("tok", PluginTokenClaims::KIND)
            .await
            .unwrap();
        assert_eq!(entry.tenant_id, "default");
        assert_eq!(entry.github_login, "alice");

        // Single-use: second take must be Missing
        assert!(matches!(
            store.take_for_kind("tok", PluginTokenClaims::KIND).await,
            Err(RefreshTokenTakeResult::Missing)
        ));
    }

    #[tokio::test]
    async fn refresh_store_take_for_wrong_kind_preserves_token() {
        let store = RefreshTokenStore::new();
        store
            .insert(
                "tok".to_string(),
                "default".to_string(),
                "bob".to_string(),
                2,
                None,
                PluginTokenClaims::ADMIN_UI_KIND.to_string(),
                3600,
            )
            .await;

        assert!(matches!(
            store.take_for_kind("tok", PluginTokenClaims::KIND).await,
            Err(RefreshTokenTakeResult::WrongKind)
        ));
        assert!(
            store
                .take_for_kind("tok", PluginTokenClaims::ADMIN_UI_KIND)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn refresh_store_revoke() {
        let store = RefreshTokenStore::new();
        store
            .insert(
                "tok".to_string(),
                "default".to_string(),
                "bob".to_string(),
                2,
                None,
                PluginTokenClaims::KIND.to_string(),
                3600,
            )
            .await;
        store.revoke_for_kind("tok", PluginTokenClaims::KIND).await;
        assert!(matches!(
            store.take_for_kind("tok", PluginTokenClaims::KIND).await,
            Err(RefreshTokenTakeResult::Missing)
        ));
    }

    #[tokio::test]
    async fn refresh_store_expired_returns_none() {
        let store = RefreshTokenStore::new();
        // TTL 0 → expires_at == now → already expired
        store
            .insert(
                "tok".to_string(),
                "default".to_string(),
                "carol".to_string(),
                3,
                None,
                PluginTokenClaims::KIND.to_string(),
                0,
            )
            .await;
        assert!(matches!(
            store.take_for_kind("tok", PluginTokenClaims::KIND).await,
            Err(RefreshTokenTakeResult::Missing)
        ));
    }

    // ── Task 4.1: bootstrap fail-closed – resolve_tenant_for_github_user ──────

    #[test]
    #[serial]
    fn resolve_tenant_returns_none_when_no_config_and_no_env() {
        // SAFETY: test-only env manipulation, tests run single-threaded via --test-threads=1
        unsafe { std::env::remove_var("AETERNA_DEFAULT_TENANT_ID") };
        let cfg = config::PluginAuthConfig {
            enabled: true,
            jwt_secret: Some("s".to_string()),
            default_tenant_id: None,
            ..Default::default()
        };
        assert!(
            resolve_tenant_for_github_user("anyuser", &cfg).is_none(),
            "MUST return None when no config field and no env var are set"
        );
    }

    #[test]
    #[serial]
    fn resolve_tenant_prefers_config_field_over_env() {
        // SAFETY: test-only env manipulation, tests run single-threaded via --test-threads=1
        unsafe { std::env::set_var("AETERNA_DEFAULT_TENANT_ID", "env-tenant") };
        let cfg = config::PluginAuthConfig {
            enabled: true,
            jwt_secret: Some("s".to_string()),
            default_tenant_id: Some("config-tenant".to_string()),
            ..Default::default()
        };
        let result = resolve_tenant_for_github_user("alice", &cfg);
        unsafe { std::env::remove_var("AETERNA_DEFAULT_TENANT_ID") };
        assert_eq!(result.as_deref(), Some("config-tenant"));
    }

    #[test]
    #[serial]
    fn resolve_tenant_falls_back_to_env_when_config_absent() {
        // SAFETY: test-only env manipulation, tests run single-threaded via --test-threads=1
        unsafe { std::env::set_var("AETERNA_DEFAULT_TENANT_ID", "env-tenant") };
        let cfg = config::PluginAuthConfig {
            enabled: true,
            jwt_secret: Some("s".to_string()),
            default_tenant_id: None,
            ..Default::default()
        };
        let result = resolve_tenant_for_github_user("alice", &cfg);
        unsafe { std::env::remove_var("AETERNA_DEFAULT_TENANT_ID") };
        assert_eq!(result.as_deref(), Some("env-tenant"));
    }

    #[test]
    #[serial]
    fn resolve_tenant_ignores_blank_config_field() {
        // SAFETY: test-only env manipulation, tests run single-threaded via --test-threads=1
        unsafe { std::env::remove_var("AETERNA_DEFAULT_TENANT_ID") };
        let cfg = config::PluginAuthConfig {
            enabled: true,
            jwt_secret: Some("s".to_string()),
            default_tenant_id: Some("   ".to_string()),
            ..Default::default()
        };
        assert!(
            resolve_tenant_for_github_user("alice", &cfg).is_none(),
            "Whitespace-only tenant id MUST be treated as absent"
        );
    }

    // ── Task 4.1: tenant_context_from_plugin_bearer returns None on bad token ──

    #[test]
    fn tenant_context_from_plugin_bearer_returns_none_without_token() {
        let headers = HeaderMap::new();
        assert!(
            tenant_context_from_plugin_bearer(&headers, &secret()).is_none(),
            "MUST return None when no Authorization header is present"
        );
    }

    #[test]
    fn tenant_context_from_plugin_bearer_returns_none_on_wrong_secret() {
        let token = mint_plugin_token("t1");
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
        assert!(
            tenant_context_from_plugin_bearer(&headers, "wrong-secret").is_none(),
            "MUST return None when token was signed with a different secret"
        );
    }
}
