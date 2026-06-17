//! Tenant-provisioning consistency suite — CLI runner.
//!
//! §13.3 of `harden-tenant-provisioning`.
//!
//! Mirrors the §13.2 API runner but drives every fixture through the
//! real `aeterna` binary (`tenant apply` / `tenant render`) against a
//! background-bound HTTP server. The same `redact_volatile()` allow-list
//! and `assert_round_trip()` invariant — duplicated locally so the two
//! runners can evolve independently — pin the cross-runner contract:
//! whichever submission path is used, the rendered tenant must cover
//! every structural claim the input manifest made.
//!
//! Auth model:
//!
//! - The test server is built with `plugin_auth.enabled = true` and a
//!   pinned HS256 secret, so it accepts a real `Authorization: Bearer
//!   <jwt>` header (the only auth shape the CLI binary emits — the
//!   dev-headers path used by §13.2 is unreachable through the binary).
//! - A user row is seeded directly into `users` with
//!   `idp_provider = 'github'` + `idp_subject = <github_login>`, and a
//!   `platformadmin` role row is inserted at `INSTANCE_SCOPE_TENANT_ID`
//!   so `authenticated_platform_context` resolves the bearer's subject
//!   to a user with PlatformAdmin authority.
//! - The minted JWT is exported as `AETERNA_API_TOKEN` for the spawned
//!   CLI process (the env-var path skips the credentials-file machinery
//!   and uses the token verbatim, matching the documented service-token
//!   flow in `client::AeternaClient::from_profile`).
//!
//! The CLI binary itself stamps `X-Aeterna-Client-Kind: cli` via static
//! `default_headers()` (see `cli/src/client.rs::build_http_client`), so
//! this runner exercises the `client_kind = cli` branch end-to-end —
//! exactly what §13.2 cannot reach because that runner sets the header
//! to `api`.

mod common;

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use aeterna::server::plugin_auth::PluginTokenClaims;
use aeterna::server::{AppState, router};
use common::build_test_state_with_plugin_auth;
use jsonwebtoken::{EncodingKey, Header, encode};
use mk_core::traits::StorageBackend;
use mk_core::types::{
    INSTANCE_SCOPE_TENANT_ID, OrganizationalUnit, RecordSource, TenantId, UnitType,
};
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::oneshot;

const SCENARIO_DIR: &str = "../tests/tenant_provisioning/scenarios";

/// Pinned HS256 secret for the test server's plugin-auth state.
/// Mirrors the value used in `cli/tests/server_runtime_test.rs` so a
/// grep for "test JWT secret" finds both call sites at once. Must be
/// ≥ 32 chars (server-side validation gates on length).
const JWT_SECRET: &str = "super-secret-test-key-at-least-32-chars";

/// GitHub login encoded into the test JWT's `sub` claim. The seeded
/// `users` row carries this value in `idp_subject`, so
/// `resolve_user_id_by_idp_bootstrap("github", GITHUB_LOGIN)` returns
/// the seeded user_id.
const GITHUB_LOGIN: &str = "platform-admin-runner";

/// Volatile keys stripped before structural comparison. Identical to
/// the API-runner allow-list in
/// `tenant_provisioning_consistency_test.rs` — duplicated here so the
/// two runners can evolve independently without a shared module that
/// would force atomic edits across both.
const VOLATILE_KEYS: &[&str] = &[
    "id",
    "createdAt",
    "updatedAt",
    "deactivatedAt",
    "generation",
    "sourceOwner",
    "status",
    "success",
    "steps",
    "message",
    "tenantId",
    "unitId",
    "userId",
];

fn redact_volatile(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for k in VOLATILE_KEYS {
                map.remove(*k);
            }
            for (_, child) in map.iter_mut() {
                redact_volatile(child);
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                redact_volatile(item);
            }
        }
        _ => {}
    }
}

fn load_scenario(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(SCENARIO_DIR)
        .join(name);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {}", path.display(), e));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parsing {}: {}", path.display(), e))
}

fn assert_round_trip(input: &Value, rendered: &Value) {
    let mut input_clean = input.clone();
    let mut rendered_clean = rendered.clone();
    redact_volatile(&mut input_clean);
    redact_volatile(&mut rendered_clean);

    let in_tenant = &input_clean["tenant"];
    let out_tenant = &rendered_clean["tenant"];
    assert_eq!(in_tenant["slug"], out_tenant["slug"], "slug round-trip");
    assert_eq!(in_tenant["name"], out_tenant["name"], "name round-trip");

    if let Some(domains) = input_clean["tenant"].get("domainMappings") {
        assert_eq!(
            domains, &rendered_clean["tenant"]["domainMappings"],
            "domain mappings round-trip"
        );
    }

    if let Some(in_fields) = input_clean["config"]
        .as_object()
        .and_then(|c| c.get("fields"))
        .and_then(|v| v.as_object())
    {
        let out_fields = rendered_clean["config"]["fields"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        for (k, v) in in_fields {
            assert_eq!(
                out_fields.get(k),
                Some(v),
                "config.fields[{}] round-trip",
                k
            );
        }
    }

    if let Some(in_refs) = input_clean["config"]
        .as_object()
        .and_then(|c| c.get("secretReferences"))
        .and_then(|v| v.as_object())
    {
        let out_refs = rendered_clean["config"]["secretReferences"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        for (k, v) in in_refs {
            assert_eq!(
                out_refs.get(k),
                Some(v),
                "config.secretReferences[{}] round-trip",
                k
            );
        }
    }
}

/// Mint an HS256 plugin-auth JWT bound to `GITHUB_LOGIN`. Mirrors the
/// `mint_test_plugin_bearer` helper in `server_runtime_test.rs` (kept
/// local to avoid pulling in the 5k-line test as a sibling module).
fn mint_jwt() -> String {
    let now = chrono::Utc::now().timestamp();
    let claims = PluginTokenClaims {
        sub: GITHUB_LOGIN.to_string(),
        idp_provider: "github".to_string(),
        tenant_id: "default".to_string(),
        iss: "aeterna-test".to_string(),
        aud: vec![PluginTokenClaims::AUDIENCE.to_string()],
        iat: now,
        exp: now + 3600,
        jti: "consistency-cli-runner".to_string(),
        github_id: 4242,
        email: Some(format!("{GITHUB_LOGIN}@example.com")),
        kind: PluginTokenClaims::KIND.to_string(),
        token_type: PluginTokenClaims::TOKEN_TYPE_USER.to_string(),
        scopes: Vec::new(),
    };
    encode(
        &Header::new(jsonwebtoken::Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(JWT_SECRET.as_bytes()),
    )
    .expect("HS256 encode of static claims must succeed")
}

/// Seed a `users` row + a root-scope `platformadmin` role assignment
/// so the bearer token's `sub` resolves to a user with PlatformAdmin
/// authority. Uses raw SQL because the storage crate intentionally
/// does not expose user-create / role-assign-at-instance-scope on its
/// public surface (those flows live in the IdP-sync service in
/// production).
async fn seed_platform_admin_user(state: &AppState) {
    let pool = state.postgres.pool();
    let user_id: String = sqlx::query_scalar(
        "INSERT INTO users (email, name, idp_provider, idp_subject, status, created_at, updated_at)
         VALUES ($1, $1, 'github', $2, 'active', NOW(), NOW())
         RETURNING id::text",
    )
    .bind(format!("{GITHUB_LOGIN}@example.com"))
    .bind(GITHUB_LOGIN)
    .fetch_one(pool)
    .await
    .expect("seed users row");

    // Create a unit at INSTANCE_SCOPE_TENANT_ID so the user_roles FK
    // (unit_id → organizational_units.id) is satisfied.
    let root_unit_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    state
        .postgres
        .create_unit(&OrganizationalUnit {
            id: root_unit_id.clone(),
            name: "Instance Scope".to_string(),
            unit_type: UnitType::Organization,
            parent_id: None,
            tenant_id: TenantId::new(INSTANCE_SCOPE_TENANT_ID.to_string())
                .expect("INSTANCE_SCOPE_TENANT_ID is a valid TenantId"),
            metadata: HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            source_owner: RecordSource::Admin,
        })
        .await
        .expect("create instance-scope unit");

    sqlx::query(
        "INSERT INTO user_roles (user_id, tenant_id, unit_id, role, created_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&user_id)
    .bind(INSTANCE_SCOPE_TENANT_ID)
    .bind(&root_unit_id)
    .bind("platformadmin")
    .bind(now)
    .execute(pool)
    .await
    .expect("seed root-scope platformadmin role");
}

/// Bind the test server on `127.0.0.1:0`, spawn `axum::serve` in a
/// background task, and return the bound URL plus a shutdown handle.
/// Falls back to `None` when Docker is unavailable so the test skips
/// gracefully — same shape as the rest of the testcontainer-backed
/// suite.
async fn start_test_server() -> Option<(
    String,
    Arc<AppState>,
    oneshot::Sender<()>,
    tempfile::TempDir,
)> {
    let plugin_auth_config = config::PluginAuthConfig {
        enabled: true,
        jwt_secret: Some(JWT_SECRET.to_string()),
        ..Default::default()
    };
    let (state, tmp) = build_test_state_with_plugin_auth(plugin_auth_config).await?;
    seed_platform_admin_user(&state).await;

    let app = router::build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("bound socket has an addr");
    let url = format!("http://{addr}");

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        // `with_graceful_shutdown` returns once shutdown_rx fires; we
        // ignore the result because the test process tears down the
        // tokio runtime as soon as the test function returns, which is
        // a benign close from the server's perspective.
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });

    // Wait for /health to come up. 50 × 100ms = 5s budget; in practice
    // the bound listener is ready before the first iteration completes.
    let client = reqwest::Client::new();
    for _ in 0..50 {
        if let Ok(resp) = client.get(format!("{url}/health")).send().await
            && resp.status().is_success()
        {
            return Some((url, state, shutdown_tx, tmp));
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("test server failed to come up at {url}");
}

/// Spawn `aeterna <args...>` against `server_url`, wait for it to exit,
/// and return `(exit_status, stdout, stderr)`. The CLI binary path is
/// resolved via `CARGO_BIN_EXE_aeterna`, which Cargo populates for
/// integration tests.
async fn run_cli(
    server_url: &str,
    jwt: &str,
    args: &[&str],
) -> (std::process::ExitStatus, String, String) {
    let bin = env!("CARGO_BIN_EXE_aeterna");
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .env("AETERNA_SERVER_URL", server_url)
        .env("AETERNA_API_TOKEN", jwt)
        // Point the CLI at a throwaway config dir so a developer's
        // real `~/.config/aeterna/credentials.toml` cannot influence
        // the test (the env-token path short-circuits credential
        // loading, but profile resolution still touches the config
        // dir for active-profile defaults).
        .env("AETERNA_CONFIG_DIR", std::env::temp_dir())
        .env("AETERNA_PROFILE", "consistency-cli-runner")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    let mut child = cmd.spawn().expect("spawn aeterna binary");
    let mut stdout_buf = String::new();
    let mut stderr_buf = String::new();
    let mut child_stdout = child.stdout.take().expect("piped stdout");
    let mut child_stderr = child.stderr.take().expect("piped stderr");
    let stdout_fut = async { child_stdout.read_to_string(&mut stdout_buf).await };
    let stderr_fut = async { child_stderr.read_to_string(&mut stderr_buf).await };
    let (out_res, err_res, status) = tokio::join!(stdout_fut, stderr_fut, child.wait());
    out_res.expect("read stdout");
    err_res.expect("read stderr");
    let status = status.expect("wait for aeterna");
    (status, stdout_buf, stderr_buf)
}

/// Sequenced consistency suite — drives every fixture through the
/// real `aeterna tenant apply` binary, then renders via
/// `aeterna tenant render --redact` and asserts structural round-trip.
///
/// Sequence invariant pinned identically to §13.2's API runner: the
/// fourth fixture (`04-noop-reapply`) is a byte-identical resubmit of
/// the third and MUST be reported with `status == "no_op"`. If the
/// CLI ever loses that status field (e.g. by re-shaping the response
/// JSON in a future release), the assertion below catches it before
/// the matrix CI job goes green on a silent regression.
#[tokio::test(flavor = "multi_thread")]
async fn consistency_cli_runner_full_sequence() {
    let Some((url, _state, shutdown_tx, _tmp)) = start_test_server().await else {
        eprintln!("Skipping consistency CLI suite: Docker not available");
        return;
    };
    let jwt = mint_jwt();

    let scenarios = [
        "01-bootstrap.json",
        "02-add-tenant-root.json",
        "03-rotate-reference.json",
        "04-noop-reapply.json",
        "05-prune.json",
    ];

    let mut last_status: Option<String> = None;
    for scenario in scenarios {
        let manifest = load_scenario(scenario);
        let slug = manifest["tenant"]["slug"]
            .as_str()
            .unwrap_or_else(|| panic!("{} missing tenant.slug", scenario))
            .to_string();
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(SCENARIO_DIR)
            .join(scenario);
        let fixture_path_str = fixture_path.to_str().expect("fixture path is utf-8");

        // tenant apply -f <fixture> --yes --json
        let (apply_status, apply_stdout, apply_stderr) = run_cli(
            &url,
            &jwt,
            &["tenant", "apply", "-f", fixture_path_str, "--yes", "--json"],
        )
        .await;
        assert!(
            apply_status.success(),
            "scenario {}: tenant apply failed (status={:?})\n--- stdout ---\n{}\n--- stderr ---\n{}",
            scenario,
            apply_status,
            apply_stdout,
            apply_stderr
        );

        let apply_body: Value = serde_json::from_str(apply_stdout.trim()).unwrap_or_else(|e| {
            panic!(
                "scenario {}: tenant apply stdout is not JSON: {}\n--- stdout ---\n{}",
                scenario, e, apply_stdout
            )
        });
        let scenario_status = apply_body["status"].as_str().unwrap_or("").to_string();
        if scenario == "04-noop-reapply.json" {
            assert_eq!(
                scenario_status, "no_op",
                "scenario {}: expected no_op, got {} (last={:?})\nbody={}",
                scenario, scenario_status, last_status, apply_body
            );
        }
        last_status = Some(scenario_status);

        // tenant render --slug <slug> --redact
        let (render_status, render_stdout, render_stderr) = run_cli(
            &url,
            &jwt,
            &["tenant", "render", "--slug", &slug, "--redact"],
        )
        .await;
        assert!(
            render_status.success(),
            "scenario {}: tenant render failed (status={:?})\n--- stdout ---\n{}\n--- stderr ---\n{}",
            scenario,
            render_status,
            render_stdout,
            render_stderr
        );

        let rendered: Value = serde_json::from_str(render_stdout.trim()).unwrap_or_else(|e| {
            panic!(
                "scenario {}: tenant render stdout is not JSON: {}\n--- stdout ---\n{}",
                scenario, e, render_stdout
            )
        });
        assert_round_trip(&manifest, &rendered);
    }

    // Best-effort shutdown so the background task drops its postgres
    // pool handle before the testcontainer's TempDir is dropped.
    let _ = shutdown_tx.send(());
}
