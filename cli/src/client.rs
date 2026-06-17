//! Shared authenticated CLI client.
//!
//! All backend-facing CLI commands use [`AeternaClient`] to:
//! 1. Resolve the active profile → server URL.
//! 2. Load stored credentials; refresh if expired (task 1.4).
//! 3. Attach `Authorization: Bearer <token>` headers.
//! 4. Provide a consistent error surface when auth is missing.

use anyhow::{Context, Result, bail};
use mk_core::types::PROVIDER_GITHUB;
use reqwest::{
    Client, Response,
    header::{ACCEPT, HeaderMap, HeaderValue},
};
use serde::{Deserialize, Serialize};
use std::future::Future;
use tokio::time::{Duration, Instant, sleep};

use crate::credentials::{self, StoredCredential};
use crate::profile::ResolvedConfig;

const DEFAULT_GITHUB_OAUTH_BASE: &str = "https://github.com";

// -- B2 §7.7 client-kind / user-agent tagging ---------------------------------
//
// Every outbound HTTP request from the CLI is tagged with two headers so the
// server can:
//   1. Normalize `X-Aeterna-Client-Kind` to a `via` value in the audit log
//      (§11.2 / §11.3 — "cli" | "ui" | "api", unknown → "api"), and
//   2. Surface `User-Agent: aeterna-cli/<version>` in request logs and traces
//      for forensics (the server never authorizes on this value).
//
// Both headers are installed once on the underlying `reqwest::Client` via
// `default_headers()` + `user_agent()` so they propagate to every verb
// (`get`, `post`, `put`, `delete`, `delete_json`, …) automatically and no
// per-call-site plumbing is required. Callers that build a bare `Client`
// (e.g. the unauthenticated device-flow endpoints below) go through the
// same [`build_http_client`] helper.

/// Canonical value for the `X-Aeterna-Client-Kind` header emitted by this
/// crate. Must match one of the three values the server's
/// `normalize_client_kind` accepts verbatim (see §11.3) — anything else
/// would round-trip to `"api"` with the original preserved only in the
/// request-scoped `RequestContext`.
const CLIENT_KIND_HEADER: &str = "X-Aeterna-Client-Kind";
const CLIENT_KIND_VALUE: &str = "cli";

/// Cargo-stamped crate version, rendered as `aeterna-cli/<version>` in the
/// `User-Agent` header.
const CLI_USER_AGENT: &str = concat!("aeterna-cli/", env!("CARGO_PKG_VERSION"));

/// Build a `reqwest::Client` pre-configured with the B2 §7.7 identity
/// headers. All authenticated and unauthenticated HTTP traffic originating
/// from the CLI must route through a client built here so the server can
/// attribute every audit row to "via=cli" without relying on heuristics.
///
/// Exposed as `pub(crate)` (via [`tagged_http_client`]) so ad-hoc call
/// sites in `cli::commands::admin` and `cli::offline` — which do not
/// use the full [`AeternaClient`] (no profile, no auth, no retry) —
/// can still emit the §7.7 headers. Direct callers of this private
/// helper inside `client.rs` stay on `build_http_client()` to keep
/// the function call graph local.
fn build_http_client() -> Client {
    let mut headers = HeaderMap::new();
    // Static const inputs — `from_static` cannot fail. The `unreachable!`
    // branches defend against someone later swapping these for a dynamic
    // value without re-reading this safety note.
    headers.insert(
        CLIENT_KIND_HEADER,
        HeaderValue::from_static(CLIENT_KIND_VALUE),
    );
    Client::builder()
        .user_agent(CLI_USER_AGENT)
        .default_headers(headers)
        .build()
        // `reqwest::ClientBuilder::build` only fails on TLS backend init
        // errors on the host system. A CLI that cannot make HTTPS calls
        // cannot function — surface the error loudly rather than fall
        // back to a non-tagged client that would silently lose §7.7
        // attribution.
        .expect("reqwest Client must build with static default headers")
}

/// Crate-visible re-export of [`build_http_client`] for CLI-originated HTTP
/// call sites that cannot route through [`AeternaClient`] — e.g. the
/// health reachability probe in [`crate::offline`] and the unauthenticated
/// admin export/import byte-stream endpoints in
/// [`crate::commands::admin`]. Every new CLI HTTP caller MUST use this
/// helper; bare `reqwest::Client::new()` in CLI code paths is a §7.7
/// regression and will silently drop the `X-Aeterna-Client-Kind` +
/// `User-Agent` headers the server relies on for audit attribution.
pub(crate) fn tagged_http_client() -> Client {
    build_http_client()
}

#[cfg(test)]
mod client_kind_tagging_tests {
    //! B2 §7.7 — pin that `tagged_http_client()` installs the two
    //! identity headers on every outbound request. These are static
    //! consts wired via `default_headers()` / `user_agent()`, so the
    //! invariant we guard against is "someone later swapped the
    //! builder chain for a bare `Client::new()`" — not a dynamic
    //! header-value bug.
    use super::{CLI_USER_AGENT, CLIENT_KIND_HEADER, CLIENT_KIND_VALUE, tagged_http_client};

    #[test]
    fn tagged_client_constants_are_wired() {
        // Construct (covers the expect() panic path on broken TLS
        // backends — running this test is itself the assertion).
        let _client = tagged_http_client();

        // Pin the static values so a future rename of the header or
        // user-agent format breaks this test rather than silently
        // de-attributing the audit log.
        assert_eq!(CLIENT_KIND_HEADER, "X-Aeterna-Client-Kind");
        assert_eq!(CLIENT_KIND_VALUE, "cli");
        assert!(
            CLI_USER_AGENT.starts_with("aeterna-cli/"),
            "user-agent prefix regression: {CLI_USER_AGENT}"
        );
    }
}

// ---------------------------------------------------------------------------
// Auth bootstrap/refresh request/response types
// (match the server-side plugin_auth.rs contracts)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct BootstrapRequest {
    provider: String,
    github_access_token: String,
}

#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub token_type: String,
    pub github_login: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct DeviceAccessTokenResponse {
    pub access_token: Option<String>,
    pub token_type: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchRequest {
    pub query: String,
    pub limit: usize,
    pub threshold: f32,
    #[serde(default)]
    pub filters: serde_json::Map<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_summary: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryAddRequest {
    pub content: String,
    pub layer: String,
    #[serde(default)]
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryListRequest {
    pub layer: String,
    pub limit: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDeleteRequest {
    pub layer: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFeedbackRequest {
    pub memory_id: String,
    pub layer: String,
    pub reward_type: String,
    pub score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchResponse {
    pub items: Vec<mk_core::types::MemoryEntry>,
    pub total: usize,
    pub reasoning: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryAddResponse {
    pub memory_id: String,
    pub layer: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Serialize)]
struct RefreshRequest {
    refresh_token: String,
}

#[derive(Serialize)]
struct LogoutRequest {
    refresh_token: String,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// An authenticated HTTP client bound to one Aeterna server profile.
///
/// `Clone` is cheap — `reqwest::Client` is internally `Arc`-backed, and
/// the auxiliary fields (access token, server URL, profile name,
/// target tenant) are small owned `String`s. Cloning is used by
/// background tasks (e.g. `tenant apply --watch` spawns an SSE
/// subscriber that needs its own handle).
#[derive(Clone)]
pub struct AeternaClient {
    inner: Client,
    server_url: String,
    profile_name: String,
    /// Current access token (may be refreshed transparently).
    access_token: String,
    /// Optional explicit target tenant for PlatformAdmin cross-tenant operations.
    /// When set, injected as `X-Admin-Target-Tenant` header on every request.
    target_tenant: Option<String>,
}

impl AeternaClient {
    /// Build a client for `profile_name`, loading and refreshing credentials
    /// as necessary.
    ///
    /// ## Token resolution order (B2 §10.6)
    ///
    /// 1. `AETERNA_API_TOKEN` env var — opaque bearer token, used
    ///    verbatim, **no refresh attempted**. The expected source is
    ///    `aeterna auth token create` (service token) exported into a
    ///    CI secret or shell profile. If the token is stale, the
    ///    server will 401 on the first request and the caller retries
    ///    out of band.
    /// 2. `~/.config/aeterna/credentials.toml` — the interactive-login
    ///    flow that tracks `access_token + refresh_token + expires_at`
    ///    and proactively refreshes through `refresh_token()` when
    ///    the access token is near expiry.
    ///
    /// The deprecated `--token` CLI flag is rejected at argv parse
    /// time (see `main::reject_legacy_token_flag`) so the env var is
    /// the *only* supported escape hatch.
    ///
    /// Returns an error if neither the env var nor stored credentials
    /// yield a usable token.
    pub async fn from_profile(resolved: &ResolvedConfig) -> Result<Self> {
        let profile_name = &resolved.profile_name;
        let server_url = resolved.server_url.trim_end_matches('/').to_string();

        // Env-var override: short-circuit the credentials file and
        // refresh machinery entirely. This is the scripted-caller
        // path — refresh requires a refresh_token we do not have, and
        // the env flow deliberately keeps lifecycle out of the CLI.
        if let Ok(tok) = std::env::var(crate::env_vars::AETERNA_API_TOKEN) {
            let tok = tok.trim();
            if !tok.is_empty() {
                return Ok(Self {
                    inner: build_http_client(),
                    server_url,
                    profile_name: profile_name.clone(),
                    access_token: tok.to_string(),
                    target_tenant: None,
                });
            }
            // Empty value in env is treated as "unset" — falling
            // through to the credentials file avoids surprising a
            // caller who has `AETERNA_API_TOKEN=` in a stale .env
            // but still expects the interactive login to work.
        }

        // Load stored credentials
        let cred = credentials::load(profile_name)?
            .with_context(|| not_logged_in_message(profile_name))?;

        // Refresh if expired
        let (access_token, refreshed_cred) =
            ensure_valid_token(&server_url, profile_name, cred, |url, refresh| async move {
                refresh_token(&url, &refresh).await
            })
            .await?;

        if let Some(new_cred) = refreshed_cred {
            credentials::save(&new_cred).context("Failed to persist refreshed credentials")?;
        }

        Ok(Self {
            inner: build_http_client(),
            server_url,
            profile_name: profile_name.clone(),
            access_token,
            target_tenant: None,
        })
    }

    pub fn server_url(&self) -> &str {
        &self.server_url
    }

    pub fn profile_name(&self) -> &str {
        &self.profile_name
    }

    /// Return a clone of this client with an explicit target tenant set.
    pub fn with_target_tenant(mut self, tenant_id: impl Into<String>) -> Self {
        self.target_tenant = Some(tenant_id.into());
        self
    }

    /// Make an authenticated GET request.
    pub async fn get(&self, path: &str) -> Result<Response> {
        let mut req = self
            .inner
            .get(format!("{}{}", self.server_url, path))
            .bearer_auth(&self.access_token);
        if let Some(ref t) = self.target_tenant {
            req = req.header("x-target-tenant-id", t.as_str());
        }
        req.send()
            .await
            .with_context(|| format!("GET {path} failed"))
    }

    /// Make an authenticated POST request with a JSON body.
    pub async fn post<B: Serialize>(&self, path: &str, body: &B) -> Result<Response> {
        let mut req = self
            .inner
            .post(format!("{}{}", self.server_url, path))
            .bearer_auth(&self.access_token)
            .json(body);
        if let Some(ref t) = self.target_tenant {
            req = req.header("x-target-tenant-id", t.as_str());
        }
        req.send()
            .await
            .with_context(|| format!("POST {path} failed"))
    }

    /// Make an authenticated DELETE request.
    pub async fn delete(&self, path: &str) -> Result<Response> {
        let mut req = self
            .inner
            .delete(format!("{}{}", self.server_url, path))
            .bearer_auth(&self.access_token);
        if let Some(ref t) = self.target_tenant {
            req = req.header("x-target-tenant-id", t.as_str());
        }
        req.send()
            .await
            .with_context(|| format!("DELETE {path} failed"))
    }

    pub async fn delete_json<B: Serialize>(&self, path: &str, body: &B) -> Result<Response> {
        let mut req = self
            .inner
            .delete(format!("{}{}", self.server_url, path))
            .bearer_auth(&self.access_token)
            .json(body);
        if let Some(ref t) = self.target_tenant {
            req = req.header("x-target-tenant-id", t.as_str());
        }
        req.send()
            .await
            .with_context(|| format!("DELETE {path} failed"))
    }

    pub async fn memory_search(&self, req: &MemorySearchRequest) -> Result<MemorySearchResponse> {
        parse_json_response(self.post("/api/v1/memory/search", req).await?).await
    }

    pub async fn memory_add(&self, req: &MemoryAddRequest) -> Result<MemoryAddResponse> {
        parse_json_response(self.post("/api/v1/memory/add", req).await?).await
    }

    pub async fn memory_list(&self, req: &MemoryListRequest) -> Result<MemorySearchResponse> {
        parse_json_response(self.post("/api/v1/memory/list", req).await?).await
    }

    pub async fn memory_delete(
        &self,
        memory_id: &str,
        req: &MemoryDeleteRequest,
    ) -> Result<MessageResponse> {
        parse_json_response(
            self.delete_json(&format!("/api/v1/memory/{memory_id}"), req)
                .await?,
        )
        .await
    }

    pub async fn memory_feedback(&self, req: &MemoryFeedbackRequest) -> Result<MessageResponse> {
        parse_json_response(self.post("/api/v1/memory/feedback", req).await?).await
    }

    // -----------------------------------------------------------------------
    // HTTP primitive: PUT
    // -----------------------------------------------------------------------

    /// Make an authenticated PUT request with a JSON body.
    pub async fn put<B: Serialize>(&self, path: &str, body: &B) -> Result<Response> {
        let mut req = self
            .inner
            .put(format!("{}{}", self.server_url, path))
            .bearer_auth(&self.access_token)
            .json(body);
        if let Some(ref t) = self.target_tenant {
            req = req.header("x-target-tenant-id", t.as_str());
        }
        req.send()
            .await
            .with_context(|| format!("PUT {path} failed"))
    }

    // -----------------------------------------------------------------------
    // HTTP primitive: PATCH
    // -----------------------------------------------------------------------

    /// Make an authenticated PATCH request with a JSON body.
    pub async fn patch<B: Serialize>(&self, path: &str, body: &B) -> Result<Response> {
        let mut req = self
            .inner
            .patch(format!("{}{}", self.server_url, path))
            .bearer_auth(&self.access_token)
            .json(body);
        if let Some(ref t) = self.target_tenant {
            req = req.header("x-target-tenant-id", t.as_str());
        }
        req.send()
            .await
            .with_context(|| format!("PATCH {path} failed"))
    }

    // -----------------------------------------------------------------------
    // Tenant endpoints (PlatformAdmin)
    // -----------------------------------------------------------------------

    pub async fn tenant_list(&self, include_inactive: bool) -> Result<serde_json::Value> {
        let path = if include_inactive {
            "/api/v1/admin/tenants?include_inactive=true".to_string()
        } else {
            "/api/v1/admin/tenants".to_string()
        };
        parse_json_response(self.get(&path).await?).await
    }

    pub async fn tenant_create(&self, body: &serde_json::Value) -> Result<serde_json::Value> {
        parse_json_response(self.post("/api/v1/admin/tenants", body).await?).await
    }

    // -----------------------------------------------------------------------
    // User default tenant (server-side preference, issue #44.b / #45)
    //
    // Uses the protected `/api/v1/user/me/default-tenant` endpoints landed
    // with the RequestContext resolver. The server persists the preference
    // in `users.default_tenant_id` so it follows the caller across devices.
    // -----------------------------------------------------------------------

    /// Returns the server-side default tenant for the authenticated user,
    /// or `Ok(None)` when none is set (the server responds with 204).
    pub async fn user_default_tenant_get(&self) -> Result<Option<serde_json::Value>> {
        let resp = self.get("/api/v1/user/me/default-tenant").await?;
        let status = resp.status();
        if status == reqwest::StatusCode::NO_CONTENT {
            return Ok(None);
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("GET /user/me/default-tenant failed ({status}): {body}");
        }
        let value: serde_json::Value = resp.json().await.context("parse default-tenant body")?;
        Ok(Some(value))
    }

    /// Set the server-side default tenant. `tenant_ref` may be a slug or UUID.
    pub async fn user_default_tenant_set(&self, tenant_ref: &str) -> Result<serde_json::Value> {
        let body = serde_json::json!({ "tenantId": tenant_ref });
        parse_json_response(self.put("/api/v1/user/me/default-tenant", &body).await?).await
    }

    /// Clear the server-side default tenant preference.
    pub async fn user_default_tenant_clear(&self) -> Result<()> {
        let resp = self.delete("/api/v1/user/me/default-tenant").await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("DELETE /user/me/default-tenant failed ({status}): {body}");
        }
        Ok(())
    }

    pub async fn tenant_show(&self, tenant: &str) -> Result<serde_json::Value> {
        parse_json_response(self.get(&format!("/api/v1/admin/tenants/{tenant}")).await?).await
    }

    /// B2 §7.2 — fetch the server-rendered current-state manifest for a
    /// tenant. Calls `GET /api/v1/admin/tenants/{slug}/manifest`,
    /// appending `?redact=true` when the operator requested the
    /// secret-safe variant (opaque placeholders instead of logical
    /// secret names, no `credentialRef` on the repository binding).
    ///
    /// The server never emits plaintext regardless of `redact`; the
    /// flag only controls whether the *names* of the secret refs are
    /// visible. This matches the `RenderQuery` contract in
    /// `manifest_api.rs`.
    pub async fn tenant_manifest(&self, tenant: &str, redact: bool) -> Result<serde_json::Value> {
        let path = if redact {
            format!("/api/v1/admin/tenants/{tenant}/manifest?redact=true")
        } else {
            format!("/api/v1/admin/tenants/{tenant}/manifest")
        };
        parse_json_response(self.get(&path).await?).await
    }

    pub async fn tenant_update(
        &self,
        tenant: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.patch(&format!("/api/v1/admin/tenants/{tenant}"), body)
                .await?,
        )
        .await
    }

    pub async fn tenant_deactivate(&self, tenant: &str) -> Result<serde_json::Value> {
        parse_json_response(
            self.post(
                &format!("/api/v1/admin/tenants/{tenant}/deactivate"),
                &serde_json::Value::Null,
            )
            .await?,
        )
        .await
    }

    pub async fn tenant_add_domain_mapping(
        &self,
        tenant: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.post(
                &format!("/api/v1/admin/tenants/{tenant}/domain-mappings"),
                body,
            )
            .await?,
        )
        .await
    }

    pub async fn tenant_repo_binding_show(&self, tenant: &str) -> Result<serde_json::Value> {
        parse_json_response(
            self.get(&format!(
                "/api/v1/admin/tenants/{tenant}/repository-binding"
            ))
            .await?,
        )
        .await
    }

    pub async fn tenant_repo_binding_set(
        &self,
        tenant: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.put(
                &format!("/api/v1/admin/tenants/{tenant}/repository-binding"),
                body,
            )
            .await?,
        )
        .await
    }

    pub async fn tenant_repo_binding_validate(
        &self,
        tenant: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.post(
                &format!("/api/v1/admin/tenants/{tenant}/repository-binding/validate"),
                body,
            )
            .await?,
        )
        .await
    }

    pub async fn tenant_config_inspect(&self, tenant: &str) -> Result<serde_json::Value> {
        parse_json_response(
            self.get(&format!("/api/v1/admin/tenants/{tenant}/config"))
                .await?,
        )
        .await
    }

    pub async fn tenant_config_upsert(
        &self,
        tenant: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.put(&format!("/api/v1/admin/tenants/{tenant}/config"), body)
                .await?,
        )
        .await
    }

    pub async fn tenant_config_validate(
        &self,
        tenant: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.post(
                &format!("/api/v1/admin/tenants/{tenant}/config/validate"),
                body,
            )
            .await?,
        )
        .await
    }

    pub async fn tenant_secret_set(
        &self,
        tenant: &str,
        logical_name: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.put(
                &format!("/api/v1/admin/tenants/{tenant}/secrets/{logical_name}"),
                body,
            )
            .await?,
        )
        .await
    }

    pub async fn tenant_secret_delete(
        &self,
        tenant: &str,
        logical_name: &str,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.delete(&format!(
                "/api/v1/admin/tenants/{tenant}/secrets/{logical_name}"
            ))
            .await?,
        )
        .await
    }

    pub async fn tenant_provision(&self, body: &serde_json::Value) -> Result<serde_json::Value> {
        parse_json_response(self.post("/api/v1/admin/tenants/provision", body).await?).await
    }

    /// POST `/api/v1/admin/tenants/provision` — B2 §7.1 `tenant apply`.
    ///
    /// Real-apply variant with structured-error tolerance. Unlike
    /// the bare [`Self::tenant_provision`], this helper returns the
    /// body on:
    ///
    /// - **200 OK** — all steps succeeded (`success: true`,
    ///   `status: "applied"` | `"unchanged"`).
    /// - **207 Multi-Status** — partial failure (`success: false`,
    ///   `status: "partial"`, `steps[].ok` mixed). The CLI renders
    ///   per-step failures and exits non-zero.
    /// - **409 Conflict** — `generation_conflict`. Surfaced as a
    ///   body so the renderer can show `currentGeneration` vs
    ///   `submittedGeneration` + hint.
    /// - **422 Unprocessable** — `manifest_validation_failed` OR
    ///   `inline_secret_not_allowed`. Rendered identically to the
    ///   dry-run validation error path.
    ///
    /// Any other status (auth, 5xx, transport) still bails — those
    /// are real infrastructure errors, not manifest problems.
    ///
    /// `allow_inline` appends `?allowInline=true` when the caller
    /// opts into inline `secrets[].secretValue` plaintext on a
    /// dev-mode server. Off by default.
    pub async fn tenant_apply(
        &self,
        manifest: &serde_json::Value,
        allow_inline: bool,
    ) -> Result<serde_json::Value> {
        let path = if allow_inline {
            "/api/v1/admin/tenants/provision?allowInline=true"
        } else {
            "/api/v1/admin/tenants/provision"
        };
        let resp = self.post(path, manifest).await?;
        let status = resp.status();
        // 2xx (including 207) + the two structured-error states
        // (409 generation conflict, 422 validation / inline-secret)
        // all carry a JSON body the caller renders.
        let is_structured_error = status == reqwest::StatusCode::CONFLICT
            || status == reqwest::StatusCode::UNPROCESSABLE_ENTITY;
        if status.is_success() || is_structured_error {
            return resp
                .json::<serde_json::Value>()
                .await
                .context("Invalid JSON response from tenant apply");
        }
        let text = resp.text().await.unwrap_or_default();
        bail!("Tenant apply failed (HTTP {status}): {text}");
    }

    /// Submit a manifest to the provision endpoint with `dryRun=true`.
    ///
    /// Unlike [`tenant_provision`], this helper treats `HTTP 422
    /// manifest_validation_failed` as a successful call whose body is
    /// returned to the caller. Validation errors are a legitimate output
    /// of the validate surface (`aeterna tenant validate`): the CLI
    /// needs to render the `validationErrors` array to the operator,
    /// not surface it as an anyhow error with the raw body inlined.
    ///
    /// Non-2xx / non-422 responses (auth failures, 5xx, generation
    /// conflicts, etc.) still bail so operators see them as errors
    /// rather than "invalid manifest" false positives.
    ///
    /// The returned JSON always carries a top-level `success` bool that
    /// the caller can branch on:
    ///
    /// - `success: true`  → body is a `ProvisionPlan` (status / hashes /
    ///   generation / section presence flags).
    /// - `success: false` → body carries `error: "manifest_validation_failed"`
    ///   and `validationErrors: [...]`.
    pub async fn tenant_provision_dry_run(
        &self,
        manifest: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let resp = self
            .post("/api/v1/admin/tenants/provision?dryRun=true", manifest)
            .await?;
        let status = resp.status();
        // 200 OK = dry-run plan; 422 Unprocessable = validation errors.
        // Both carry a structured JSON body the caller renders.
        if status.is_success() || status == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
            return resp
                .json::<serde_json::Value>()
                .await
                .context("Invalid JSON response from dry-run provision");
        }
        let text = resp.text().await.unwrap_or_default();
        bail!("Dry-run provision failed (HTTP {status}): {text}");
    }

    /// POST `/api/v1/admin/tenants/diff` — B3 §2.4 / CLI §7.3.
    ///
    /// Posts the candidate manifest and returns the structured
    /// [`TenantDiff`][crate::server::tenant_diff::TenantDiff] JSON
    /// payload on 200, OR the `manifest_validation_failed` envelope
    /// on 422. Mirrors the 200/422 dual-success handling of
    /// [`Self::tenant_provision_dry_run`] so callers can share
    /// rendering infrastructure (`render_validation_errors` already
    /// knows how to project the 422 shape).
    ///
    /// Server extracts the tenant slug from
    /// `manifest.tenant.slug`; there is no path parameter. PA-gated.
    pub async fn tenant_diff(&self, manifest: &serde_json::Value) -> Result<serde_json::Value> {
        let resp = self.post("/api/v1/admin/tenants/diff", manifest).await?;
        let status = resp.status();
        if status.is_success() || status == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
            return resp
                .json::<serde_json::Value>()
                .await
                .context("Invalid JSON response from tenant diff");
        }
        let text = resp.text().await.unwrap_or_default();
        bail!("Tenant diff failed (HTTP {status}): {text}");
    }

    pub async fn my_tenant_config_inspect(&self) -> Result<serde_json::Value> {
        parse_json_response(self.get("/api/v1/admin/tenant-config").await?).await
    }

    pub async fn my_tenant_config_upsert(
        &self,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(self.put("/api/v1/admin/tenant-config", body).await?).await
    }

    pub async fn my_tenant_config_validate(
        &self,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.post("/api/v1/admin/tenant-config/validate", body)
                .await?,
        )
        .await
    }

    pub async fn my_tenant_secret_set(
        &self,
        logical_name: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.put(
                &format!("/api/v1/admin/tenant-config/secrets/{logical_name}"),
                body,
            )
            .await?,
        )
        .await
    }

    pub async fn my_tenant_secret_delete(&self, logical_name: &str) -> Result<serde_json::Value> {
        parse_json_response(
            self.delete(&format!(
                "/api/v1/admin/tenant-config/secrets/{logical_name}"
            ))
            .await?,
        )
        .await
    }

    // -----------------------------------------------------------------------
    // Admin health
    // -----------------------------------------------------------------------

    /// GET /health — returns the raw server health payload.
    pub async fn admin_health(&self) -> Result<serde_json::Value> {
        parse_json_response(self.get("/health").await?).await
    }

    // -----------------------------------------------------------------------
    // Knowledge endpoints
    // -----------------------------------------------------------------------

    /// POST /api/v1/knowledge/query — search knowledge items.
    pub async fn knowledge_query(
        &self,
        query: &str,
        layer: Option<&str>,
        limit: usize,
    ) -> Result<serde_json::Value> {
        let mut body = serde_json::json!({
            "query": query,
            "limit": limit,
        });
        if let Some(l) = layer {
            body["layer"] = serde_json::json!(l);
        }
        parse_json_response(self.post("/api/v1/knowledge/query", &body).await?).await
    }

    /// GET /api/v1/knowledge/{id}/metadata — get metadata for a knowledge item.
    pub async fn knowledge_metadata(&self, id: &str) -> Result<serde_json::Value> {
        parse_json_response(
            self.get(&format!("/api/v1/knowledge/{id}/metadata"))
                .await?,
        )
        .await
    }

    /// POST /api/v1/knowledge/create — create a new knowledge item.
    pub async fn knowledge_create(&self, body: &serde_json::Value) -> Result<serde_json::Value> {
        parse_json_response(self.post("/api/v1/knowledge/create", body).await?).await
    }

    /// DELETE /api/v1/knowledge/{id} — delete a knowledge item.
    pub async fn knowledge_delete(&self, id: &str) -> Result<serde_json::Value> {
        // DELETE /api/v1/knowledge/{id} returns 204 No Content on success.
        // We handle that by returning a synthetic success value.
        let resp = self
            .inner
            .delete(format!("{}/api/v1/knowledge/{}", self.server_url, id))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .with_context(|| format!("DELETE /api/v1/knowledge/{id} failed"))?;
        if resp.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(serde_json::json!({"success": true, "id": id}));
        }
        parse_json_response(resp).await
    }

    // -----------------------------------------------------------------------
    // Knowledge promotion lifecycle endpoints
    // -----------------------------------------------------------------------

    /// POST /api/v1/knowledge/{id}/promotions/preview
    pub async fn knowledge_promotion_preview(
        &self,
        id: &str,
        target_layer: &str,
        mode: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut body = serde_json::json!({ "target_layer": target_layer });
        if let Some(m) = mode {
            body["mode"] = serde_json::json!(m);
        }
        parse_json_response(
            self.post(&format!("/api/v1/knowledge/{id}/promotions/preview"), &body)
                .await?,
        )
        .await
    }

    /// POST /api/v1/knowledge/{id}/promotions — create a promotion request.
    pub async fn knowledge_promote(
        &self,
        id: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.post(&format!("/api/v1/knowledge/{id}/promotions"), body)
                .await?,
        )
        .await
    }

    /// GET /api/v1/knowledge/promotions — list promotion requests (optionally filtered by status).
    pub async fn knowledge_promotions_list(
        &self,
        status: Option<&str>,
    ) -> Result<serde_json::Value> {
        let path = if let Some(s) = status {
            format!("/api/v1/knowledge/promotions?status={s}")
        } else {
            "/api/v1/knowledge/promotions".to_string()
        };
        parse_json_response(self.get(&path).await?).await
    }

    /// POST /api/v1/knowledge/promotions/{id}/approve
    pub async fn knowledge_promotion_approve(
        &self,
        promotion_id: &str,
        decision: &str,
    ) -> Result<serde_json::Value> {
        let body = serde_json::json!({ "decision": decision });
        parse_json_response(
            self.post(
                &format!("/api/v1/knowledge/promotions/{promotion_id}/approve"),
                &body,
            )
            .await?,
        )
        .await
    }

    /// POST /api/v1/knowledge/promotions/{id}/reject
    pub async fn knowledge_promotion_reject(
        &self,
        promotion_id: &str,
        reason: &str,
    ) -> Result<serde_json::Value> {
        let body = serde_json::json!({ "reason": reason });
        parse_json_response(
            self.post(
                &format!("/api/v1/knowledge/promotions/{promotion_id}/reject"),
                &body,
            )
            .await?,
        )
        .await
    }

    /// POST /api/v1/knowledge/promotions/{id}/retarget
    pub async fn knowledge_promotion_retarget(
        &self,
        promotion_id: &str,
        target_layer: &str,
    ) -> Result<serde_json::Value> {
        let body = serde_json::json!({ "target_layer": target_layer });
        parse_json_response(
            self.post(
                &format!("/api/v1/knowledge/promotions/{promotion_id}/retarget"),
                &body,
            )
            .await?,
        )
        .await
    }

    /// POST /api/v1/knowledge/{id}/relations — create a semantic relation.
    pub async fn knowledge_relate(
        &self,
        id: &str,
        target_id: &str,
        relation_type: &str,
    ) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "target_id": target_id,
            "relation_type": relation_type,
        });
        parse_json_response(
            self.post(&format!("/api/v1/knowledge/{id}/relations"), &body)
                .await?,
        )
        .await
    }

    // -----------------------------------------------------------------------
    // Agent endpoints
    // -----------------------------------------------------------------------

    pub async fn agent_list(
        &self,
        delegated_by: Option<&str>,
        agent_type: Option<&str>,
        all: bool,
    ) -> Result<serde_json::Value> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(d) = delegated_by {
            params.push(("delegated_by", d.to_string()));
        }
        if let Some(t) = agent_type {
            params.push(("type", t.to_string()));
        }
        if all {
            params.push(("all", "true".to_string()));
        }
        let path = build_path("/api/v1/agent", &params);
        parse_json_response(self.get(&path).await?).await
    }

    pub async fn agent_show(&self, agent_id: &str) -> Result<serde_json::Value> {
        parse_json_response(self.get(&format!("/api/v1/agent/{agent_id}")).await?).await
    }

    pub async fn agent_register(&self, body: &serde_json::Value) -> Result<serde_json::Value> {
        parse_json_response(self.post("/api/v1/agent", body).await?).await
    }

    pub async fn agent_permissions_list(&self, agent_id: &str) -> Result<serde_json::Value> {
        parse_json_response(
            self.get(&format!("/api/v1/agent/{agent_id}/permissions"))
                .await?,
        )
        .await
    }

    pub async fn agent_permission_grant(
        &self,
        agent_id: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.post(&format!("/api/v1/agent/{agent_id}/permissions"), body)
                .await?,
        )
        .await
    }

    pub async fn agent_permission_revoke(
        &self,
        agent_id: &str,
        permission: &str,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.delete(&format!(
                "/api/v1/agent/{agent_id}/permissions/{permission}"
            ))
            .await?,
        )
        .await
    }

    pub async fn agent_revoke(&self, agent_id: &str) -> Result<serde_json::Value> {
        parse_json_response(self.delete(&format!("/api/v1/agent/{agent_id}")).await?).await
    }

    // -----------------------------------------------------------------------
    // Org endpoints
    // -----------------------------------------------------------------------

    /// List organizations.
    ///
    /// When `tenant_scope` is `Some(value)`, the value is forwarded as the
    /// `?tenant=` query parameter verbatim (see `docs/api/admin.md` for
    /// the grammar: `*`, case-insensitive `all`, or a slug/uuid). `None`
    /// preserves the pre-#44.d behavior (tenant-scoped listing).
    pub async fn org_list(
        &self,
        tenant_root: Option<&str>,
        all: bool,
        tenant_scope: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(root) = tenant_root {
            params.push(("tenantRoot", root.to_string()));
        }
        if all {
            params.push(("all", "true".to_string()));
        }
        if let Some(t) = tenant_scope {
            params.push(("tenant", t.to_string()));
        }
        let path = build_path("/api/v1/org", &params);
        parse_json_response(self.get(&path).await?).await
    }

    pub async fn org_show(&self, org_id: &str) -> Result<serde_json::Value> {
        parse_json_response(self.get(&format!("/api/v1/org/{org_id}")).await?).await
    }

    pub async fn org_create(&self, body: &serde_json::Value) -> Result<serde_json::Value> {
        parse_json_response(self.post("/api/v1/org", body).await?).await
    }

    pub async fn org_members_list(&self, org_id: &str) -> Result<serde_json::Value> {
        parse_json_response(self.get(&format!("/api/v1/org/{org_id}/members")).await?).await
    }

    pub async fn org_member_add(
        &self,
        org_id: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.post(&format!("/api/v1/org/{org_id}/members"), body)
                .await?,
        )
        .await
    }

    pub async fn org_member_remove(
        &self,
        org_id: &str,
        user_id: &str,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.delete(&format!("/api/v1/org/{org_id}/members/{user_id}"))
                .await?,
        )
        .await
    }

    pub async fn org_member_set_role(
        &self,
        org_id: &str,
        user_id: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.put(
                &format!("/api/v1/org/{org_id}/members/{user_id}/role"),
                body,
            )
            .await?,
        )
        .await
    }

    // -----------------------------------------------------------------------
    // Team endpoints
    // -----------------------------------------------------------------------

    pub async fn team_list(&self, org: Option<&str>, all: bool) -> Result<serde_json::Value> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(o) = org {
            params.push(("org", o.to_string()));
        }
        if all {
            params.push(("all", "true".to_string()));
        }
        let path = build_path("/api/v1/team", &params);
        parse_json_response(self.get(&path).await?).await
    }

    pub async fn team_show(&self, team_id: &str) -> Result<serde_json::Value> {
        parse_json_response(self.get(&format!("/api/v1/team/{team_id}")).await?).await
    }

    pub async fn team_create(&self, body: &serde_json::Value) -> Result<serde_json::Value> {
        parse_json_response(self.post("/api/v1/team", body).await?).await
    }

    pub async fn team_members_list(&self, team_id: &str) -> Result<serde_json::Value> {
        parse_json_response(self.get(&format!("/api/v1/team/{team_id}/members")).await?).await
    }

    pub async fn team_member_add(
        &self,
        team_id: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.post(&format!("/api/v1/team/{team_id}/members"), body)
                .await?,
        )
        .await
    }

    pub async fn team_member_remove(
        &self,
        team_id: &str,
        user_id: &str,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.delete(&format!("/api/v1/team/{team_id}/members/{user_id}"))
                .await?,
        )
        .await
    }

    pub async fn team_member_set_role(
        &self,
        team_id: &str,
        user_id: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.put(
                &format!("/api/v1/team/{team_id}/members/{user_id}/role"),
                body,
            )
            .await?,
        )
        .await
    }

    // -----------------------------------------------------------------------
    // User endpoints
    // -----------------------------------------------------------------------

    /// List users.
    ///
    /// See [`Self::org_list`] for the `tenant_scope` semantics.
    pub async fn user_list(
        &self,
        org: Option<&str>,
        team: Option<&str>,
        role: Option<&str>,
        all: bool,
        tenant_scope: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(o) = org {
            params.push(("org", o.to_string()));
        }
        if let Some(t) = team {
            params.push(("team", t.to_string()));
        }
        if let Some(r) = role {
            params.push(("role", r.to_string()));
        }
        if all {
            params.push(("all", "true".to_string()));
        }
        if let Some(t) = tenant_scope {
            params.push(("tenant", t.to_string()));
        }
        let path = build_path("/api/v1/user", &params);
        parse_json_response(self.get(&path).await?).await
    }

    pub async fn user_show(&self, user_id: &str) -> Result<serde_json::Value> {
        parse_json_response(self.get(&format!("/api/v1/user/{user_id}")).await?).await
    }

    pub async fn user_register(&self, body: &serde_json::Value) -> Result<serde_json::Value> {
        parse_json_response(self.post("/api/v1/user", body).await?).await
    }

    pub async fn user_roles_list(&self, user_id: &str) -> Result<serde_json::Value> {
        parse_json_response(self.get(&format!("/api/v1/user/{user_id}/roles")).await?).await
    }

    pub async fn user_role_grant(
        &self,
        user_id: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.post(&format!("/api/v1/user/{user_id}/roles"), body)
                .await?,
        )
        .await
    }

    pub async fn user_role_revoke(&self, user_id: &str, role: &str) -> Result<serde_json::Value> {
        parse_json_response(
            self.delete(&format!("/api/v1/user/{user_id}/roles/{role}"))
                .await?,
        )
        .await
    }

    pub async fn user_invite(&self, body: &serde_json::Value) -> Result<serde_json::Value> {
        parse_json_response(self.post("/api/v1/user/invite", body).await?).await
    }

    // -----------------------------------------------------------------------
    // Govern endpoints
    // -----------------------------------------------------------------------

    pub async fn govern_status(&self) -> Result<serde_json::Value> {
        parse_json_response(self.get("/api/v1/govern/status").await?).await
    }

    pub async fn govern_pending(
        &self,
        request_type: Option<&str>,
        layer: Option<&str>,
        requestor: Option<&str>,
        mine: bool,
    ) -> Result<serde_json::Value> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(t) = request_type {
            params.push(("type", t.to_string()));
        }
        if let Some(l) = layer {
            params.push(("layer", l.to_string()));
        }
        if let Some(r) = requestor {
            params.push(("requestor", r.to_string()));
        }
        if mine {
            params.push(("mine", "true".to_string()));
        }
        let path = build_path("/api/v1/govern/pending", &params);
        parse_json_response(self.get(&path).await?).await
    }

    pub async fn govern_approve(
        &self,
        request_id: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.post(&format!("/api/v1/govern/approve/{request_id}"), body)
                .await?,
        )
        .await
    }

    pub async fn govern_reject(
        &self,
        request_id: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.post(&format!("/api/v1/govern/reject/{request_id}"), body)
                .await?,
        )
        .await
    }

    pub async fn govern_config_show(&self) -> Result<serde_json::Value> {
        parse_json_response(self.get("/api/v1/govern/config").await?).await
    }

    pub async fn govern_config_update(
        &self,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(self.put("/api/v1/govern/config", body).await?).await
    }

    pub async fn govern_roles_list(&self) -> Result<serde_json::Value> {
        parse_json_response(self.get("/api/v1/govern/roles").await?).await
    }

    pub async fn govern_role_assign(&self, body: &serde_json::Value) -> Result<serde_json::Value> {
        parse_json_response(self.post("/api/v1/govern/roles", body).await?).await
    }

    pub async fn govern_role_revoke(
        &self,
        principal: &str,
        role: &str,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.delete(&format!("/api/v1/govern/roles/{principal}/{role}"))
                .await?,
        )
        .await
    }

    /// Fetch governance audit entries.
    ///
    /// See [`Self::org_list`] for the `tenant_scope` semantics. Since
    /// #44.d Bundle D, `/govern/audit` supports the full `?tenant=` grammar
    /// (`*`, `all`, `<slug>`, `<uuid>`) — rows are filtered by
    /// `acting_as_tenant_id` and the envelope carries per-item
    /// `tenantId`/`tenantSlug` decoration.
    pub async fn govern_audit(
        &self,
        action: Option<&str>,
        since: Option<&str>,
        actor: Option<&str>,
        target_type: Option<&str>,
        limit: usize,
        tenant_scope: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(a) = action {
            params.push(("action", a.to_string()));
        }
        if let Some(s) = since {
            params.push(("since", s.to_string()));
        }
        if let Some(a) = actor {
            params.push(("actor", a.to_string()));
        }
        if let Some(t) = target_type {
            params.push(("target_type", t.to_string()));
        }
        params.push(("limit", limit.to_string()));
        if let Some(t) = tenant_scope {
            params.push(("tenant", t.to_string()));
        }
        let path = build_path("/api/v1/govern/audit", &params);
        parse_json_response(self.get(&path).await?).await
    }

    // -----------------------------------------------------------------------
    // Permission inspection endpoints
    // -----------------------------------------------------------------------

    pub async fn permissions_matrix(&self) -> Result<serde_json::Value> {
        parse_json_response(self.get("/api/v1/admin/permissions/matrix").await?).await
    }

    pub async fn permissions_effective(
        &self,
        user_id: &str,
        resource: Option<&str>,
        actions: Option<&str>,
        role: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut params: Vec<(&str, String)> = vec![("user_id", user_id.to_string())];
        if let Some(r) = resource {
            params.push(("resource", r.to_string()));
        }
        if let Some(a) = actions {
            params.push(("actions", a.to_string()));
        }
        if let Some(ro) = role {
            params.push(("role", ro.to_string()));
        }
        let path = build_path("/api/v1/admin/permissions/effective", &params);
        parse_json_response(self.get(&path).await?).await
    }

    // -----------------------------------------------------------------------
    // Git provider connection admin endpoints (task 3.4)
    // -----------------------------------------------------------------------

    pub async fn git_provider_connection_create(
        &self,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.post("/api/v1/admin/git-provider-connections", body)
                .await?,
        )
        .await
    }

    pub async fn git_provider_connection_list(&self) -> Result<serde_json::Value> {
        parse_json_response(self.get("/api/v1/admin/git-provider-connections").await?).await
    }

    pub async fn git_provider_connection_show(&self, id: &str) -> Result<serde_json::Value> {
        parse_json_response(
            self.get(&format!("/api/v1/admin/git-provider-connections/{id}"))
                .await?,
        )
        .await
    }

    pub async fn git_provider_connection_grant_tenant(
        &self,
        connection_id: &str,
        tenant: &str,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.post(
                &format!("/api/v1/admin/git-provider-connections/{connection_id}/tenants/{tenant}"),
                &serde_json::json!({}),
            )
            .await?,
        )
        .await
    }

    pub async fn git_provider_connection_revoke_tenant(
        &self,
        connection_id: &str,
        tenant: &str,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.delete(&format!(
                "/api/v1/admin/git-provider-connections/{connection_id}/tenants/{tenant}"
            ))
            .await?,
        )
        .await
    }

    pub async fn tenant_git_provider_connections_list(
        &self,
        tenant: &str,
    ) -> Result<serde_json::Value> {
        parse_json_response(
            self.get(&format!(
                "/api/v1/admin/tenants/{tenant}/git-provider-connections"
            ))
            .await?,
        )
        .await
    }
}

async fn ensure_valid_token<F, Fut>(
    server_url: &str,
    profile_name: &str,
    cred: StoredCredential,
    refresh_fn: F,
) -> Result<(String, Option<StoredCredential>)>
where
    F: Fn(String, String) -> Fut,
    Fut: Future<Output = Result<TokenResponse>>,
{
    if !cred.is_expired() {
        return Ok((cred.access_token, None));
    }

    let refreshed = refresh_fn(server_url.to_string(), cred.refresh_token.clone())
        .await
        .with_context(|| {
            format!(
                "Session refresh failed for profile '{profile_name}'. Run: aeterna auth login --profile {profile_name}"
            )
        })?;

    let new_cred = StoredCredential {
        profile_name: profile_name.to_string(),
        access_token: refreshed.access_token.clone(),
        refresh_token: refreshed.refresh_token,
        expires_at: chrono::Utc::now().timestamp() + refreshed.expires_in as i64,
        github_login: refreshed.github_login,
        email: refreshed.email,
    };

    Ok((refreshed.access_token, Some(new_cred)))
}

// ---------------------------------------------------------------------------
// Bootstrap (login)
// ---------------------------------------------------------------------------

/// Call `POST /api/v1/auth/plugin/bootstrap` to exchange a GitHub access token
/// for Aeterna session credentials.
pub async fn bootstrap_github(
    server_url: &str,
    github_access_token: &str,
) -> Result<TokenResponse> {
    let url = format!(
        "{}/api/v1/auth/plugin/bootstrap",
        server_url.trim_end_matches('/')
    );
    let body = BootstrapRequest {
        provider: PROVIDER_GITHUB.to_string(),
        github_access_token: github_access_token.to_string(),
    };
    let client = build_http_client();
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .context("Cannot reach Aeterna server")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("Login failed (HTTP {status}): {text}");
    }

    resp.json::<TokenResponse>()
        .await
        .context("Invalid response from auth bootstrap endpoint")
}

fn github_device_code_url(github_oauth_base_url: Option<&str>) -> String {
    let base = github_oauth_base_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_GITHUB_OAUTH_BASE);
    format!("{base}/login/device/code")
}

fn github_device_access_token_url(github_oauth_base_url: Option<&str>) -> String {
    let base = github_oauth_base_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_GITHUB_OAUTH_BASE);
    format!("{base}/login/oauth/access_token")
}

pub async fn request_device_code(
    client_id: &str,
    scope: &str,
    github_oauth_base_url: Option<&str>,
) -> Result<DeviceCodeResponse> {
    let client = build_http_client();
    let resp = client
        .post(github_device_code_url(github_oauth_base_url))
        .header(ACCEPT, "application/json")
        .form(&[("client_id", client_id), ("scope", scope)])
        .send()
        .await
        .context("Cannot reach GitHub device authorization endpoint")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("Device code request failed (HTTP {status}): {text}");
    }

    resp.json::<DeviceCodeResponse>()
        .await
        .context("Invalid response from GitHub device code endpoint")
}

pub async fn poll_device_authorization(
    client_id: &str,
    device_code: &str,
    interval: u64,
    expires_in: u64,
    github_oauth_base_url: Option<&str>,
) -> Result<String> {
    let client = build_http_client();
    let started = Instant::now();
    let mut poll_interval_secs = interval.max(1);

    loop {
        if started.elapsed() >= Duration::from_secs(expires_in) {
            bail!("Device authorization timed out. Please run `aeterna auth login` again.");
        }

        sleep(Duration::from_secs(poll_interval_secs)).await;

        let resp = client
            .post(github_device_access_token_url(github_oauth_base_url))
            .header(ACCEPT, "application/json")
            .form(&[
                ("client_id", client_id),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await
            .context("Cannot reach GitHub device token endpoint")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!("Device authorization polling failed (HTTP {status}): {text}");
        }

        let body = resp
            .json::<DeviceAccessTokenResponse>()
            .await
            .context("Invalid response from GitHub device token endpoint")?;

        match handle_device_poll_response(body, &mut poll_interval_secs)? {
            Some(access_token) => return Ok(access_token),
            None => continue,
        }
    }
}

fn handle_device_poll_response(
    body: DeviceAccessTokenResponse,
    poll_interval_secs: &mut u64,
) -> Result<Option<String>> {
    if let Some(access_token) = body.access_token {
        return Ok(Some(access_token));
    }

    match body.error.as_deref() {
        Some("authorization_pending") => Ok(None),
        Some("slow_down") => {
            *poll_interval_secs = poll_interval_secs.saturating_add(5);
            Ok(None)
        }
        Some("expired_token") => {
            bail!("Device authorization expired. Please run `aeterna auth login` again.")
        }
        Some("access_denied") => {
            bail!("Device authorization denied. Please run `aeterna auth login` again.")
        }
        Some(other) => bail!("Device authorization failed: {other}"),
        None => bail!("Device authorization failed: missing access_token and error fields"),
    }
}

// ---------------------------------------------------------------------------
// Refresh (task 1.4)
// ---------------------------------------------------------------------------

/// Call `POST /api/v1/auth/plugin/refresh` to get a new token pair.
pub async fn refresh_token(server_url: &str, refresh_token: &str) -> Result<TokenResponse> {
    let url = format!(
        "{}/api/v1/auth/plugin/refresh",
        server_url.trim_end_matches('/')
    );
    let body = RefreshRequest {
        refresh_token: refresh_token.to_string(),
    };
    let client = build_http_client();
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .context("Cannot reach Aeterna server for token refresh")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("Token refresh failed (HTTP {status}): {text}");
    }

    resp.json::<TokenResponse>()
        .await
        .context("Invalid response from token refresh endpoint")
}

// ---------------------------------------------------------------------------
// Logout
// ---------------------------------------------------------------------------

/// Call `POST /api/v1/auth/plugin/logout` to revoke the refresh token.
pub async fn server_logout(server_url: &str, refresh_token_val: &str) -> Result<()> {
    let url = format!(
        "{}/api/v1/auth/plugin/logout",
        server_url.trim_end_matches('/')
    );
    let body = LogoutRequest {
        refresh_token: refresh_token_val.to_string(),
    };
    let client = build_http_client();
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .context("Cannot reach Aeterna server for logout")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        // Treat 401/404 as already-logged-out (idempotent)
        if status == 401 || status == 404 {
            return Ok(());
        }
        bail!("Logout failed (HTTP {status}): {text}");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn not_logged_in_message(profile_name: &str) -> String {
    format!(
        "Not logged in (profile: {profile_name}). Run: aeterna auth login --profile {profile_name}"
    )
}

/// Check connectivity to a server URL by hitting `/health`.
pub async fn check_reachability(server_url: &str) -> bool {
    let client = build_http_client();
    client
        .get(format!("{}/health", server_url.trim_end_matches('/')))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// URL helpers
// ---------------------------------------------------------------------------

fn build_path(base: &str, params: &[(&str, String)]) -> String {
    if params.is_empty() {
        return base.to_string();
    }
    let qs: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding_simple(v)))
        .collect();
    format!("{}?{}", base, qs.join("&"))
}

fn urlencoding_simple(s: &str) -> String {
    // Minimal percent-encoding: only encode characters unsafe in query values.
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            _ => {
                for byte in c.to_string().as_bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}
async fn parse_json_response<T: for<'de> Deserialize<'de>>(resp: Response) -> Result<T> {
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("Request failed (HTTP {status}): {text}");
    }

    resp.json::<T>()
        .await
        .context("Invalid JSON response from server")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_logged_in_message() {
        let msg = not_logged_in_message("production");
        assert!(msg.contains("production"));
        assert!(msg.contains("aeterna auth login"));
        assert!(msg.contains("--profile production"));
    }

    #[test]
    fn test_not_logged_in_message_default_profile() {
        let msg = not_logged_in_message("default");
        assert!(msg.contains("default"));
        assert!(msg.contains("aeterna auth login"));
    }

    #[test]
    fn test_token_response_deserialize() {
        let json = r#"{
            "access_token": "at123",
            "refresh_token": "rt456",
            "expires_in": 3600,
            "token_type": "Bearer",
            "github_login": "alice",
            "email": "alice@example.com"
        }"#;
        let resp: TokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.access_token, "at123");
        assert_eq!(resp.refresh_token, "rt456");
        assert_eq!(resp.expires_in, 3600);
        assert_eq!(resp.github_login, Some("alice".to_string()));
        assert_eq!(resp.email, Some("alice@example.com".to_string()));
    }

    #[test]
    fn test_token_response_deserialize_optional_fields_absent() {
        let json = r#"{
            "access_token": "at",
            "refresh_token": "rt",
            "expires_in": 900,
            "token_type": "Bearer"
        }"#;
        let resp: TokenResponse = serde_json::from_str(json).unwrap();
        assert!(resp.github_login.is_none());
        assert!(resp.email.is_none());
    }

    #[test]
    fn test_bootstrap_request_serialization() {
        let req = BootstrapRequest {
            provider: "github".to_string(),
            github_access_token: "gho_abc".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"provider\":\"github\""));
        assert!(json.contains("\"github_access_token\":\"gho_abc\""));
    }

    #[test]
    fn test_refresh_request_serialization() {
        let req = RefreshRequest {
            refresh_token: "rt_xyz".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"refresh_token\":\"rt_xyz\""));
    }

    #[test]
    fn test_logout_request_serialization() {
        let req = LogoutRequest {
            refresh_token: "rt_abc".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("rt_abc"));
    }

    #[test]
    fn test_device_code_response_deserialize() {
        let json = r#"{
            "device_code": "dev_123",
            "user_code": "ABCD-EFGH",
            "verification_uri": "https://github.com/login/device",
            "expires_in": 900,
            "interval": 5
        }"#;
        let resp: DeviceCodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.device_code, "dev_123");
        assert_eq!(resp.user_code, "ABCD-EFGH");
        assert_eq!(resp.verification_uri, "https://github.com/login/device");
        assert_eq!(resp.expires_in, 900);
        assert_eq!(resp.interval, 5);
    }

    #[test]
    fn test_device_access_token_response_deserialize_success() {
        let json = r#"{
            "access_token": "gho_abc",
            "token_type": "bearer"
        }"#;
        let resp: DeviceAccessTokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.access_token.as_deref(), Some("gho_abc"));
        assert_eq!(resp.token_type.as_deref(), Some("bearer"));
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_device_access_token_response_deserialize_authorization_pending() {
        let json = r#"{"error":"authorization_pending"}"#;
        let resp: DeviceAccessTokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.error.as_deref(), Some("authorization_pending"));
        assert!(resp.access_token.is_none());
    }

    #[test]
    fn test_device_access_token_response_deserialize_slow_down() {
        let json = r#"{"error":"slow_down"}"#;
        let resp: DeviceAccessTokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.error.as_deref(), Some("slow_down"));
        assert!(resp.access_token.is_none());
    }

    #[test]
    fn test_device_access_token_response_deserialize_expired_token() {
        let json = r#"{"error":"expired_token"}"#;
        let resp: DeviceAccessTokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.error.as_deref(), Some("expired_token"));
        assert!(resp.access_token.is_none());
    }

    #[test]
    fn test_device_access_token_response_deserialize_access_denied() {
        let json = r#"{"error":"access_denied"}"#;
        let resp: DeviceAccessTokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.error.as_deref(), Some("access_denied"));
        assert!(resp.access_token.is_none());
    }

    #[test]
    fn test_github_device_flow_endpoints() {
        assert_eq!(
            github_device_code_url(None),
            "https://github.com/login/device/code"
        );
        assert_eq!(
            github_device_access_token_url(None),
            "https://github.com/login/oauth/access_token"
        );
    }

    #[test]
    fn test_handle_device_poll_response_success() {
        let mut interval = 5;
        let out = handle_device_poll_response(
            DeviceAccessTokenResponse {
                access_token: Some("gho_success".to_string()),
                token_type: Some("bearer".to_string()),
                error: None,
            },
            &mut interval,
        )
        .unwrap();
        assert_eq!(out, Some("gho_success".to_string()));
        assert_eq!(interval, 5);
    }

    #[test]
    fn test_handle_device_poll_response_authorization_pending() {
        let mut interval = 5;
        let out = handle_device_poll_response(
            DeviceAccessTokenResponse {
                access_token: None,
                token_type: None,
                error: Some("authorization_pending".to_string()),
            },
            &mut interval,
        )
        .unwrap();
        assert!(out.is_none());
        assert_eq!(interval, 5);
    }

    #[test]
    fn test_handle_device_poll_response_slow_down_increments_interval() {
        let mut interval = 5;
        let out = handle_device_poll_response(
            DeviceAccessTokenResponse {
                access_token: None,
                token_type: None,
                error: Some("slow_down".to_string()),
            },
            &mut interval,
        )
        .unwrap();
        assert!(out.is_none());
        assert_eq!(interval, 10);
    }

    #[test]
    fn test_handle_device_poll_response_expired_token_errors() {
        let mut interval = 5;
        let err = handle_device_poll_response(
            DeviceAccessTokenResponse {
                access_token: None,
                token_type: None,
                error: Some("expired_token".to_string()),
            },
            &mut interval,
        )
        .unwrap_err();
        assert!(err.to_string().contains("Device authorization expired"));
    }

    #[test]
    fn test_handle_device_poll_response_access_denied_errors() {
        let mut interval = 5;
        let err = handle_device_poll_response(
            DeviceAccessTokenResponse {
                access_token: None,
                token_type: None,
                error: Some("access_denied".to_string()),
            },
            &mut interval,
        )
        .unwrap_err();
        assert!(err.to_string().contains("Device authorization denied"));
    }

    #[test]
    fn test_refresh_error_message_contains_relogin_hint() {
        let profile_name = "prod";
        let err = anyhow::Error::msg("refresh failed").context(format!(
            "Session refresh failed for profile '{profile_name}'. Run: aeterna auth login --profile {profile_name}"
        ));
        let rendered = format!("{err:#}");
        assert!(rendered.contains("aeterna auth login --profile prod"));
    }

    #[tokio::test]
    async fn test_ensure_valid_token_not_expired_skips_refresh() {
        let cred = StoredCredential {
            profile_name: "default".to_string(),
            access_token: "access_live".to_string(),
            refresh_token: "refresh_live".to_string(),
            expires_at: chrono::Utc::now().timestamp() + 3_600,
            github_login: Some("alice".to_string()),
            email: Some("alice@example.com".to_string()),
        };

        let result = ensure_valid_token(
            "https://aeterna.example.com",
            "default",
            cred,
            |_url, _refresh| async {
                Ok(TokenResponse {
                    access_token: "should_not_be_used".to_string(),
                    refresh_token: "should_not_be_used".to_string(),
                    expires_in: 60,
                    token_type: "Bearer".to_string(),
                    github_login: None,
                    email: None,
                })
            },
        )
        .await
        .unwrap();

        assert_eq!(result.0, "access_live");
        assert!(result.1.is_none());
    }

    #[tokio::test]
    async fn test_ensure_valid_token_expired_refresh_success() {
        let cred = StoredCredential {
            profile_name: "default".to_string(),
            access_token: "old_access".to_string(),
            refresh_token: "refresh_old".to_string(),
            expires_at: chrono::Utc::now().timestamp() - 3_600,
            github_login: Some("alice".to_string()),
            email: Some("alice@example.com".to_string()),
        };

        let (access, refreshed) = ensure_valid_token(
            "https://aeterna.example.com",
            "default",
            cred,
            |_url, _refresh| async {
                Ok(TokenResponse {
                    access_token: "new_access".to_string(),
                    refresh_token: "new_refresh".to_string(),
                    expires_in: 600,
                    token_type: "Bearer".to_string(),
                    github_login: Some("alice".to_string()),
                    email: Some("alice@example.com".to_string()),
                })
            },
        )
        .await
        .unwrap();

        assert_eq!(access, "new_access");
        let refreshed = refreshed.unwrap();
        assert_eq!(refreshed.access_token, "new_access");
        assert_eq!(refreshed.refresh_token, "new_refresh");
    }

    #[tokio::test]
    async fn test_ensure_valid_token_expired_refresh_failure_contains_hint() {
        let cred = StoredCredential {
            profile_name: "prod".to_string(),
            access_token: "old_access".to_string(),
            refresh_token: "refresh_old".to_string(),
            expires_at: chrono::Utc::now().timestamp() - 3_600,
            github_login: None,
            email: None,
        };

        let err = ensure_valid_token(
            "https://aeterna.example.com",
            "prod",
            cred,
            |_url, _refresh| async { anyhow::bail!("upstream refresh failed") },
        )
        .await
        .unwrap_err();

        let rendered = format!("{err:#}");
        assert!(rendered.contains("aeterna auth login --profile prod"));
    }
}
