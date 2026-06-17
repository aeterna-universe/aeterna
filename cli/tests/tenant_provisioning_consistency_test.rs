//! Tenant-provisioning consistency suite — API runner.
//!
//! §13.2 + §13.5 of `harden-tenant-provisioning`.
//!
//! Drives every fixture in `tests/tenant_provisioning/scenarios/`
//! through the canonical `POST /api/v1/admin/tenants/provision`
//! endpoint, then renders the resulting tenant via
//! `GET /api/v1/admin/tenants/{slug}/manifest?redact=true` and asserts
//! that the rendered shape matches what was submitted (modulo an
//! allowlist of volatile fields: timestamps, generations, ids).
//!
//! The CLI runner (§13.3) and UI runner (§13.4) need a real binary
//! and a real browser; they will reuse the same fixture set and the
//! same `redact_volatile()` allowlist defined here.

mod common;

use aeterna::server::router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::build_test_state;
use serde_json::{Value, json};
use tower::ServiceExt;

const SCENARIO_DIR: &str = "../tests/tenant_provisioning/scenarios";

/// Header set the runner stamps on every request — mirrors the wire
/// contract pinned by `cli/src/server/request_context.rs`.
const CLIENT_KIND_HEADER: &str = "x-aeterna-client-kind";

/// Volatile keys stripped before structural comparison. These are
/// either server-assigned (timestamps, ids, generations) or part of
/// the response envelope (`success`, `status`).
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

/// Recursively delete keys from `VOLATILE_KEYS` so two manifests can be
/// compared structurally. Operates in-place on owned `Value`s.
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

fn admin_request(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-user-id", "platform-admin")
        .header("x-user-role", "platform_admin")
        .header(CLIENT_KIND_HEADER, "api");
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&v).unwrap())
        }
        None => Body::empty(),
    };
    req.body(body).unwrap()
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    if bytes.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(&bytes).unwrap()
}

/// Submit `manifest` via the canonical provision route and return the
/// (status, body) pair.
async fn provision(app: &axum::Router, manifest: &Value) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(admin_request(
            "POST",
            "/api/v1/admin/tenants/provision",
            Some(manifest.clone()),
        ))
        .await
        .unwrap();
    let status = resp.status();
    (status, body_json(resp).await)
}

async fn render_manifest(app: &axum::Router, slug: &str) -> Value {
    let resp = app
        .clone()
        .oneshot(admin_request(
            "GET",
            &format!("/api/v1/admin/tenants/{}/manifest?redact=true", slug),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "render manifest for {} should return 200",
        slug
    );
    body_json(resp).await
}

/// Assert that the manifest the server now stores covers, *at minimum*,
/// every structural claim the input manifest made. The renderer is
/// allowed to add server-managed fields (e.g. `metadata.generation`,
/// `tenant.status`) — those are stripped by `redact_volatile`.
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

    // config.fields: every input key/value must be present in the
    // rendered output. The renderer may add server-defaulted entries.
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

    // config.secretReferences: structural equality (every declared ref
    // must match by kind + kind-specific fields).
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

/// The full sequenced suite — each fixture builds on the previous one
/// against the same slug, exercising create / extend / modify / no-op /
/// prune in a single test process.
#[tokio::test]
async fn consistency_api_runner_full_sequence() {
    let Some((state, _tmp)) = build_test_state().await else {
        eprintln!("Skipping consistency suite: Docker not available");
        return;
    };
    let app = router::build_router(state);

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

        let (status, body) = provision(&app, &manifest).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "scenario {}: provision returned {} body={}",
            scenario,
            status,
            body
        );

        let scenario_status = body["status"].as_str().unwrap_or("").to_string();
        // Sequence invariant: 04-noop-reapply MUST be reported as a no-op.
        if scenario == "04-noop-reapply.json" {
            assert_eq!(
                scenario_status, "no_op",
                "scenario {}: expected no_op, got {} (last={:?})",
                scenario, scenario_status, last_status
            );
        }
        last_status = Some(scenario_status);

        let rendered = render_manifest(&app, &slug).await;
        assert_round_trip(&manifest, &rendered);
    }
}

/// Sanity check: the redact helper actually strips every key in the
/// allowlist, including nested ones.
#[test]
fn redact_volatile_strips_allowlist_recursively() {
    let mut v = json!({
        "id": "abc",
        "tenant": {
            "slug": "x",
            "createdAt": 123,
            "members": [
                {"userId": "u1", "email": "a@b"},
                {"userId": "u2", "email": "c@d"}
            ]
        }
    });
    redact_volatile(&mut v);
    assert!(v.get("id").is_none());
    assert!(v["tenant"].get("createdAt").is_none());
    let members = v["tenant"]["members"].as_array().unwrap();
    for m in members {
        assert!(m.get("userId").is_none());
        assert!(m.get("email").is_some());
    }
}

/// Independent fixture: the `?allowInline=true` flag is rejected when
/// the server config does not allow inline secrets. This pins the
/// security gate from §4.2 against the API runner path.
#[tokio::test]
async fn consistency_api_runner_inline_secret_rejected_by_default() {
    let Some((state, _tmp)) = build_test_state().await else {
        eprintln!("Skipping consistency suite: Docker not available");
        return;
    };
    let app = router::build_router(state);

    let manifest = json!({
        "apiVersion": "aeterna.io/v1",
        "kind": "TenantManifest",
        "metadata": { "labels": { "suite": "consistency" } },
        "tenant": { "slug": "inline-reject", "name": "Inline Reject" },
        "config": {
            "secretReferences": {
                "db.password": {
                    "logicalName": "db.password",
                    "kind": "inline",
                    "inline": "hunter2"
                }
            }
        }
    });

    let resp = app
        .clone()
        .oneshot(admin_request(
            "POST",
            "/api/v1/admin/tenants/provision?allowInline=true",
            Some(manifest),
        ))
        .await
        .unwrap();
    let status = resp.status();
    let body = body_json(resp).await;
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::FORBIDDEN,
        "inline secret should be rejected when server config does not opt in (got {} body={})",
        status,
        body
    );
}
