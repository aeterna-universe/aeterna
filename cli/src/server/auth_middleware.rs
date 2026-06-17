//! Global authentication middleware for protected routes.
//!
//! Implements an Axum tower layer that validates `Authorization: Bearer <token>`
//! on protected route groups (`/api/v1/*`, `/mcp/*`) and injects a
//! [`TenantContext`] request extension for downstream handlers.
//!
//! Behavior:
//! - **Auth disabled** (`pluginAuth.enabled: false`): passes through with a
//!   synthetic `default/system` context (backward-compatible dev mode).
//! - **Auth enabled**: validates the JWT bearer token, extracts identity from
//!   claims, and injects `TenantContext`. Returns 401 for missing, invalid, or
//!   expired tokens.
//!
//! # Handler integration
//!
//! Protected handlers can read the pre-validated context from request extensions:
//!
//! ```rust,ignore
//! async fn my_handler(req: axum::extract::Request) -> impl IntoResponse {
//!     let ctx = req.extensions().get::<TenantContext>().unwrap();
//!     // ...
//! }
//! ```
//!
//! Existing handlers that call `authenticated_tenant_context()` or
//! `tenant_scoped_context()` continue to work correctly — they re-derive the
//! same context the middleware already validated. This redundancy is intentional
//! during the migration period so that handlers remain self-contained and
//! testable without the middleware layer.

use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::future::BoxFuture;
use mk_core::types::{
    DEFAULT_TENANT_SLUG, RoleIdentifier, SYSTEM_USER_ID, TenantContext, TenantId, UserId,
};
use tower::{Layer, Service};

use super::PluginAuthState;
use super::lookup_roles_for_idp;
use super::plugin_auth::{PluginIdentity, validate_plugin_bearer};
use super::request_context::RequestContext;

// ---------------------------------------------------------------------------
// Layer
// ---------------------------------------------------------------------------

/// Tower layer that wraps services with [`AuthenticationService`].
///
/// Attach this to route groups that require authentication:
///
/// ```ignore
/// Router::new()
///     .nest("/api/v1", protected_routes)
///     .layer(AuthenticationLayer::new(plugin_auth_state.clone()));
/// ```
#[derive(Clone)]
pub struct AuthenticationLayer {
    auth_state: Arc<PluginAuthState>,
}

impl AuthenticationLayer {
    pub fn new(auth_state: Arc<PluginAuthState>) -> Self {
        Self { auth_state }
    }
}

impl<S> Layer<S> for AuthenticationLayer {
    type Service = AuthenticationService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthenticationService {
            inner,
            auth_state: self.auth_state.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// Tower service that authenticates requests before forwarding to the inner
/// service.
///
/// Inserts a [`TenantContext`] into the request extensions on success.
#[derive(Clone)]
pub struct AuthenticationService<S> {
    inner: S,
    auth_state: Arc<PluginAuthState>,
}

impl<S> Service<Request<Body>> for AuthenticationService<S>
where
    S: Service<Request<Body>, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        // Clone inner service per tower::Service contract (poll_ready consumed).
        let mut inner = self.inner.clone();
        std::mem::swap(&mut self.inner, &mut inner);

        let auth_state = self.auth_state.clone();

        Box::pin(async move {
            // B2 §11.2 / §11.3 — extract the client-kind header ONCE here,
            // normalize it, and stash the result as a request extension so
            // every downstream handler (audit-writing or not) sees the same
            // value. Runs *before* auth branching so even 401-path requests
            // still carry a `via` in their logs. Cheap: header lookup + a
            // handful of string comparisons.
            let request_ctx = RequestContext::from_headers(req.headers());
            req.extensions_mut().insert(request_ctx);

            // ── Pass-through when auth is disabled ──────────────────────
            if !auth_state.config.enabled {
                // Development / service-to-service mode: inject synthetic context
                // derived from legacy headers (X-Tenant-ID, X-User-ID) or defaults.
                let ctx = dev_context_from_headers(req.headers());
                req.extensions_mut().insert(ctx);
                return inner.call(req).await;
            }

            // ── Auth enabled: validate bearer token ────────────────────
            let jwt_secret = if let Some(s) = &auth_state.config.jwt_secret {
                s.clone()
            } else {
                tracing::error!("Plugin auth enabled but JWT secret is not configured");
                return Ok(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "configuration_error",
                    "Authentication is misconfigured",
                ));
            };

            let identity = match validate_plugin_bearer(req.headers(), &jwt_secret) {
                Some(id) => id,
                None => {
                    return Ok(error_response(
                        StatusCode::UNAUTHORIZED,
                        "unauthorized",
                        "Valid bearer token required",
                    ));
                }
            };

            // Build TenantContext from validated identity.
            match tenant_context_from_identity(&auth_state, &identity, req.headers()).await {
                Some(ctx) => {
                    req.extensions_mut().insert(ctx);
                    inner.call(req).await
                }
                None => Ok(error_response(
                    StatusCode::UNAUTHORIZED,
                    "invalid_identity",
                    "Token contains invalid tenant or user identity",
                )),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `TenantContext` from a validated plugin identity and optional
/// header overrides (e.g. `X-Target-Tenant-ID` for PlatformAdmin cross-tenant
/// operations, `X-User-Role` for explicit role assertion).
async fn tenant_context_from_identity(
    auth_state: &PluginAuthState,
    identity: &PluginIdentity,
    headers: &axum::http::HeaderMap,
) -> Option<TenantContext> {
    let tenant_id = TenantId::new(sanitize(&identity.tenant_id, 100))?;

    let (user_id_str, roles) = match &auth_state.postgres {
        Some(postgres) => {
            let resolved = postgres
                .resolve_user_id_by_idp_bootstrap(&identity.idp_provider, &identity.github_login)
                .await
                .ok()??;
            let roles = lookup_roles_for_idp(
                postgres.as_ref(),
                &identity.idp_provider,
                &identity.github_login,
                tenant_id.as_str(),
            )
            .await
            .ok()?;
            (resolved, roles)
        }
        None => (sanitize(&identity.github_login, 100), Vec::new()),
    };

    let user_id = UserId::new(user_id_str)?;

    let target_tenant_id = extract_deprecated_target_tenant(headers);

    Some(TenantContext {
        tenant_id,
        user_id,
        agent_id: None,
        roles,
        target_tenant_id,
    })
}

/// Extract the legacy `X-Target-Tenant-Id` header and emit a deprecation
/// warning if present.
///
/// #44.d §8 — `X-Target-Tenant-Id` is the pre-RFC header used for ad-hoc
/// PlatformAdmin impersonation. It has been superseded by the `?tenant=`
/// query parameter grammar (see `docs/api/admin.md`) which is:
/// - explicit at the call site (URL, not header)
/// - gated consistently across all list endpoints (`forbidden_scope` /
///   `scope_not_implemented` / `tenant_not_found`)
/// - locked by the §4.1 envelope contract test
///
/// We still HONOR the header to avoid breaking any in-flight client, but
/// emit a `tracing::warn!` with `target = "compat"` so operators can find
/// stragglers in log aggregation and migrate them off. Removal is planned
/// for a future minor — see #44.e.
pub(super) fn extract_deprecated_target_tenant(
    headers: &axum::http::HeaderMap,
) -> Option<TenantId> {
    let raw = headers
        .get("x-target-tenant-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())?;

    tracing::warn!(
        target: "compat",
        header = "X-Target-Tenant-Id",
        replacement = "?tenant=<slug-or-*>",
        raw_value_prefix = %raw.chars().take(16).collect::<String>(),
        "deprecated cross-tenant header — clients should migrate to the ?tenant= query parameter; removal planned in a future minor"
    );

    TenantId::new(raw.chars().take(100).collect())
}

/// Build a synthetic `TenantContext` from legacy headers for development mode.
///
/// Mirrors the pre-existing `authenticated_tenant_context` behavior when auth
/// is disabled: trust `X-Tenant-ID` and `X-User-ID` headers, falling back to
/// `default` / `system`.
fn dev_context_from_headers(headers: &axum::http::HeaderMap) -> TenantContext {
    let tenant_str = headers
        .get("x-tenant-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_TENANT_SLUG);
    let user_str = headers
        .get("x-user-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .unwrap_or(SYSTEM_USER_ID);

    let roles = headers
        .get("x-user-role")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(|s| vec![RoleIdentifier::from_str_flexible(s)])
        .unwrap_or_default();

    let target_tenant_id = extract_deprecated_target_tenant(headers);

    TenantContext {
        tenant_id: TenantId::new(tenant_str.chars().take(100).collect())
            .expect("default tenant id is valid"),
        user_id: UserId::new(user_str.chars().take(100).collect())
            .expect("default user id is valid"),
        agent_id: None,
        roles,
        target_tenant_id,
    }
}

fn sanitize(value: &str, max_len: usize) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        trimmed.chars().take(max_len).collect()
    }
}

fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        axum::Json(serde_json::json!({
            "error": code,
            "message": message
        })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, header::AUTHORIZATION};
    use axum::routing::get;
    use mk_core::traits::StorageBackend;
    use mk_core::types::{
        INSTANCE_SCOPE_TENANT_ID, OrganizationalUnit, RecordSource, TenantId, UnitType,
    };
    use storage::postgres::PostgresBackend;
    use testing::{postgres, unique_id};
    use tower::ServiceExt;

    use crate::server::plugin_auth::{RefreshTokenStore, RefreshTokenStoreBackend};

    fn test_auth_state(
        enabled: bool,
        jwt_secret: Option<String>,
        postgres: Option<Arc<PostgresBackend>>,
    ) -> Arc<PluginAuthState> {
        Arc::new(PluginAuthState {
            config: config::PluginAuthConfig {
                enabled,
                jwt_secret,
                ..Default::default()
            },
            postgres,
            refresh_store: RefreshTokenStoreBackend::InMemory(RefreshTokenStore::new()),
        })
    }

    async fn create_test_backend() -> Option<Arc<PostgresBackend>> {
        let fixture = postgres().await?;
        let backend = Arc::new(PostgresBackend::new(fixture.url()).await.ok()?);
        backend.initialize_schema().await.ok()?;
        Some(backend)
    }

    /// Handler that reads the TenantContext from extensions.
    async fn echo_context(req: Request<Body>) -> impl IntoResponse {
        match req.extensions().get::<TenantContext>() {
            Some(ctx) => {
                let body = serde_json::json!({
                    "tenant_id": ctx.tenant_id.as_str(),
                    "user_id": ctx.user_id.as_str(),
                    "roles": ctx
                        .roles
                        .iter()
                        .map(std::string::ToString::to_string)
                        .collect::<Vec<_>>(),
                });
                (StatusCode::OK, axum::Json(body)).into_response()
            }
            None => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "No TenantContext in extensions",
            )
                .into_response(),
        }
    }

    fn app_with_auth(auth_state: Arc<PluginAuthState>) -> Router {
        Router::new()
            .route("/protected", get(echo_context))
            .layer(AuthenticationLayer::new(auth_state))
    }

    // ── Auth disabled (pass-through) ──────────────────────────────────

    #[tokio::test]
    async fn auth_disabled_passes_through_with_default_context() {
        let app = app_with_auth(test_auth_state(false, None, None));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["tenant_id"], "default");
        assert_eq!(json["user_id"], "system");
    }

    #[tokio::test]
    async fn auth_disabled_uses_legacy_headers() {
        let app = app_with_auth(test_auth_state(false, None, None));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("x-tenant-id", "acme")
                    .header("x-user-id", "alice")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["tenant_id"], "acme");
        assert_eq!(json["user_id"], "alice");
    }

    // ── Auth enabled: missing token ───────────────────────────────────

    #[tokio::test]
    async fn auth_enabled_returns_401_without_token() {
        let app = app_with_auth(test_auth_state(
            true,
            Some("test-secret-at-least-32-characters-long".to_string()),
            None,
        ));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ── Auth enabled: invalid token ───────────────────────────────────

    #[tokio::test]
    async fn auth_enabled_returns_401_with_invalid_token() {
        let app = app_with_auth(test_auth_state(
            true,
            Some("test-secret-at-least-32-characters-long".to_string()),
            None,
        ));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header(AUTHORIZATION, "Bearer invalid-jwt-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ── Auth enabled: valid token ─────────────────────────────────────

    #[tokio::test]
    async fn auth_enabled_passes_through_with_valid_token() {
        use crate::server::plugin_auth::PluginTokenClaims;
        use jsonwebtoken::{EncodingKey, Header};

        let Some(backend) = create_test_backend().await else {
            eprintln!("Skipping PostgreSQL test: Docker not available");
            return;
        };

        let secret = "test-secret-at-least-32-characters-long";
        let now = chrono::Utc::now().timestamp();
        let tenant = TenantId::new("tenant-1".to_string()).unwrap();
        let tenant_unit_id = unique_id("org");
        let root_unit_id = unique_id("instance");

        backend
            .create_unit(&OrganizationalUnit {
                id: tenant_unit_id.clone(),
                name: "Tenant Root Org".to_string(),
                unit_type: UnitType::Organization,
                parent_id: None,
                tenant_id: tenant.clone(),
                metadata: std::collections::HashMap::new(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                source_owner: RecordSource::Admin,
            })
            .await
            .unwrap();

        backend
            .create_unit(&OrganizationalUnit {
                id: root_unit_id.clone(),
                name: "Instance Scope".to_string(),
                unit_type: UnitType::Organization,
                parent_id: None,
                tenant_id: INSTANCE_SCOPE_TENANT_ID.parse().unwrap(),
                metadata: std::collections::HashMap::new(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                source_owner: RecordSource::Admin,
            })
            .await
            .unwrap();

        let user_id: String = sqlx::query_scalar(
            "INSERT INTO users (email, name, idp_provider, idp_subject, status, created_at, updated_at)
             VALUES ($1, $1, 'github', $2, 'active', NOW(), NOW())
             RETURNING id::text",
        )
        .bind(format!("{}@example.com", unique_id("auth-user")))
        .bind("testuser")
        .fetch_one(backend.pool())
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO user_roles (user_id, tenant_id, unit_id, role, created_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&user_id)
        .bind(INSTANCE_SCOPE_TENANT_ID)
        .bind(&root_unit_id)
        .bind("platformadmin")
        .bind(chrono::Utc::now())
        .execute(backend.pool())
        .await
        .unwrap();

        let claims = PluginTokenClaims {
            sub: "testuser".to_string(),
            idp_provider: "github".to_string(),
            tenant_id: tenant.as_str().to_string(),
            iss: "aeterna".to_string(),
            aud: vec![PluginTokenClaims::AUDIENCE.to_string()],
            iat: now,
            exp: now + 3600,
            jti: uuid::Uuid::new_v4().to_string(),
            github_id: 42,
            email: Some("test@example.com".to_string()),
            kind: PluginTokenClaims::KIND.to_string(),
            token_type: PluginTokenClaims::TOKEN_TYPE_USER.to_string(),
            scopes: Vec::new(),
        };
        let token = jsonwebtoken::encode(
            &Header::new(jsonwebtoken::Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();

        let app = app_with_auth(test_auth_state(
            true,
            Some(secret.to_string()),
            Some(backend),
        ));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header(AUTHORIZATION, format!("Bearer {token}"))
                    .header("x-user-role", "viewer")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["tenant_id"], "tenant-1");
        assert_eq!(json["user_id"], "testuser");
        assert_eq!(json["roles"], serde_json::json!(["PlatformAdmin"]));
    }

    // ── Auth enabled: missing JWT secret config ───────────────────────

    #[tokio::test]
    async fn auth_enabled_returns_500_without_jwt_secret() {
        let app = app_with_auth(test_auth_state(true, None, None));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header(AUTHORIZATION, "Bearer some-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ── dev_context_from_headers unit tests ───────────────────────────

    #[test]
    fn dev_context_defaults_to_default_system() {
        let headers = axum::http::HeaderMap::new();
        let ctx = dev_context_from_headers(&headers);
        assert_eq!(ctx.tenant_id.as_str(), "default");
        assert_eq!(ctx.user_id.as_str(), "system");
        assert!(ctx.roles.is_empty());
        assert!(ctx.target_tenant_id.is_none());
    }

    #[test]
    fn dev_context_reads_custom_headers() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-tenant-id", "my-tenant".parse().unwrap());
        headers.insert("x-user-id", "alice".parse().unwrap());
        headers.insert("x-user-role", "admin".parse().unwrap());

        let ctx = dev_context_from_headers(&headers);
        assert_eq!(ctx.tenant_id.as_str(), "my-tenant");
        assert_eq!(ctx.user_id.as_str(), "alice");
        assert!(!ctx.roles.is_empty());
    }

    #[test]
    fn error_response_returns_json_body() {
        let resp = error_response(StatusCode::UNAUTHORIZED, "test_error", "Test message");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
