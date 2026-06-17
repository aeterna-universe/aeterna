use assert_cmd::{Command, cargo_bin_cmd};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Hermetic CLI command builder.
///
/// Scrubs the environment so the spawned `aeterna` subprocess cannot discover
/// the developer's real credentials in the platform config dir
/// (`~/.config/aeterna/` on Linux, `~/Library/Application Support/aeterna/`
/// on macOS). Without this scrubbing, any developer who has ever run
/// `aeterna auth login` will see ~85 e2e tests fail with HTTP 401 because
/// the CLI under test silently uses their personal token to hit a real
/// server, instead of taking the "no server configured" code path the tests
/// are asserting on. CI doesn't catch this because GitHub Actions runners
/// have no pre-existing aeterna config.
///
/// Tests that need a configured (mock) server must opt in via [`aeterna_mock`].
fn aeterna() -> Command {
    let mut cmd = cargo_bin_cmd!("aeterna");
    cmd.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        // Pin HOME to a sentinel path that has no aeterna config tree under
        // any platform-specific subpath the `dirs` crate might compute from
        // it (~/.config/aeterna, ~/Library/Application Support/aeterna,
        // %APPDATA%/aeterna, …).
        .env("HOME", "/nonexistent-aeterna-test-home");
    // Preserve TMPDIR so `tempfile` inside the CLI still works on macOS.
    if let Ok(v) = std::env::var("TMPDIR") {
        cmd.env("TMPDIR", v);
    }
    cmd
}

/// Holds the mock server and temp config dir (both must stay alive for the test).
struct MockEnv {
    _server: MockServer,
    _config_dir: tempfile::TempDir,
    url: String,
    config_path: std::path::PathBuf,
}

/// Return a CLI command wired to a mock server with fake credentials.
fn aeterna_mock(env: &MockEnv) -> Command {
    // `aeterna()` already env_clear'd and pinned HOME to a sentinel.
    // Layer the mock-specific overrides on top.
    let mut cmd = aeterna();
    cmd.env("AETERNA_SERVER_URL", &env.url)
        .env("AETERNA_PROFILE", "__mock__")
        .env("AETERNA_CONFIG_DIR", &env.config_path);
    cmd
}

/// Start a wiremock server with stubs for all API endpoints.
/// Returns the server (must stay alive) and its base URL.
async fn start_mock_api() -> (MockServer, String) {
    let server = MockServer::start().await;

    // Auth
    Mock::given(method("GET"))
        .and(path("/api/v1/auth/whoami"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "userId": "test-user", "email": "test@test.com"
        })))
        .mount(&server)
        .await;

    // Agent list
    Mock::given(method("GET"))
        .and(path("/api/v1/agent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "operation": "agent_list",
            "items": [{
                "agentId": "agent-test-123", "name": "Test Agent",
                "type": "autonomous", "delegatedBy": "alice", "status": "active"
            }], "total": 1, "limit": 50, "offset": 0
        })))
        .mount(&server)
        .await;

    // Agent show
    Mock::given(method("GET"))
        .and(path_regex(r"^/api/v1/agent/[^/]+$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "agentId": "agent-test-123", "name": "Test Agent",
            "type": "autonomous", "delegatedBy": "alice",
            "permissions": ["memory:read", "memory:write"],
            "config": { "model": "gpt-4", "maxTokens": 4096 },
            "status": "active"
        })))
        .mount(&server)
        .await;

    // Agent permissions GET
    Mock::given(method("GET"))
        .and(path_regex(r"^/api/v1/agent/[^/]+/permissions$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "operation": "agent_permissions_list",
            "items": [{"permission": "memory:read", "scope": "*"}],
            "total": 1, "limit": 50, "offset": 0
        })))
        .mount(&server)
        .await;

    // Agent permissions POST (grant)
    Mock::given(method("POST"))
        .and(path_regex(r"^/api/v1/agent/[^/]+/permissions$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "operation": "agent_permission_grant",
            "agentId": "agent-test-123", "permission": "memory:write", "success": true
        })))
        .mount(&server)
        .await;

    // Agent permissions DELETE (revoke)
    Mock::given(method("DELETE"))
        .and(path_regex(r"^/api/v1/agent/[^/]+/permissions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "operation": "agent_permission_revoke", "success": true
        })))
        .mount(&server)
        .await;

    // Agent DELETE (revoke agent)
    Mock::given(method("DELETE"))
        .and(path_regex(r"^/api/v1/agent/[^/]+$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "operation": "agent_revoke", "agentId": "agent-test-123", "success": true
        })))
        .mount(&server)
        .await;

    // Sync
    Mock::given(path_regex(r"^/api/v1/sync"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": false, "error": "server_not_connected",
            "dry_run": false, "direction": "ALL"
        })))
        .mount(&server)
        .await;

    // Knowledge
    Mock::given(path_regex(r"^/api/v1/knowledge"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "not_connected", "items": []
        })))
        .mount(&server)
        .await;

    // Catch-all
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server)
        .await;

    let url = server.uri();
    (server, url)
}

/// Blocking helper: start mock server with fake profile and credentials.
fn mock_server() -> MockEnv {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (server, url) = rt.block_on(start_mock_api());

    // Create temp AETERNA_CONFIG_DIR with fake profile + credentials
    let config_dir = tempfile::tempdir().expect("create temp dir");
    let aeterna_dir = config_dir.path();

    // config.toml with __mock__ profile pointing at mock server
    std::fs::write(
        aeterna_dir.join("config.toml"),
        format!(
            "default_profile = \"__mock__\"\n\n[profiles.__mock__]\nname = \"__mock__\"\nserver_url = \"{url}\"\n"
        ),
    )
    .unwrap();

    // credentials.toml with a fake non-expired token (HashMap keyed by profile name)
    let expires_at = chrono::Utc::now().timestamp() + 3600;
    std::fs::write(
        aeterna_dir.join("credentials.toml"),
        format!(
            "[credentials.__mock__]\nprofile_name = \"__mock__\"\naccess_token = \"fake-jwt-token\"\nrefresh_token = \"fake-refresh\"\nexpires_at = {expires_at}\n"
        ),
    )
    .unwrap();

    let config_path = config_dir.path().to_path_buf();
    MockEnv {
        _server: server,
        _config_dir: config_dir,
        url,
        config_path,
    }
}

mod help_and_version {
    use super::*;
    use predicates::prelude::predicate;

    #[test]
    fn test_help_flag() {
        aeterna()
            .arg("--help")
            .assert()
            .success()
            .stdout(predicate::str::contains("Usage:"))
            .stdout(predicate::str::contains("Commands:"));
    }

    #[test]
    fn test_short_help_flag() {
        aeterna()
            .arg("-h")
            .assert()
            .success()
            .stdout(predicate::str::contains("Usage:"));
    }

    #[test]
    fn test_version_flag() {
        aeterna()
            .arg("--version")
            .assert()
            .success()
            .stdout(predicate::str::contains("aeterna"));
    }

    #[test]
    fn test_short_version_flag() {
        aeterna()
            .arg("-V")
            .assert()
            .success()
            .stdout(predicate::str::contains("aeterna"));
    }

    #[test]
    fn test_no_args_shows_help() {
        aeterna()
            .assert()
            .failure()
            .stderr(predicate::str::contains("Usage:"));
    }
}

mod memory_subcommand {
    use super::*;
    use predicates::prelude::PredicateBooleanExt;
    use predicates::prelude::predicate;

    #[test]
    fn test_memory_help() {
        aeterna()
            .args(["memory", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("memor"))
            .stdout(predicate::str::contains("add"))
            .stdout(predicate::str::contains("search"))
            .stdout(predicate::str::contains("list"));
    }

    #[test]
    fn test_memory_add_help() {
        aeterna()
            .args(["memory", "add", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Add a new memory"))
            .stdout(predicate::str::contains("--layer"));
    }

    #[test]
    fn test_memory_add_dry_run() {
        aeterna()
            .args(["memory", "add", "Test memory content", "--dry-run"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Dry Run"));
    }

    #[test]
    fn test_memory_add_dry_run_json() {
        aeterna()
            .args([
                "memory",
                "add",
                "Test memory content",
                "--dry-run",
                "--json",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("\"dryRun\""))
            .stdout(predicate::str::contains("\"content\""));
    }

    #[test]
    fn test_memory_add_with_layer() {
        aeterna()
            .args([
                "memory",
                "add",
                "Project specific memory",
                "--layer",
                "project",
                "--dry-run",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("project"));
    }

    #[test]
    fn test_memory_add_invalid_layer() {
        aeterna()
            .args(["memory", "add", "Test", "--layer", "invalid-layer"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("invalid").or(predicate::str::contains("Invalid")));
    }

    #[test]
    fn test_memory_search_help() {
        aeterna()
            .args(["memory", "search", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Search memories"))
            .stdout(predicate::str::contains("--limit"));
    }

    #[test]
    fn test_memory_search_dry_run() {
        aeterna()
            .args(["memory", "search", "test query", "--dry-run"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Dry Run"));
    }

    #[test]
    fn test_memory_list_help() {
        aeterna()
            .args(["memory", "list", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("List memories"));
    }

    #[test]
    fn test_memory_delete_help() {
        aeterna()
            .args(["memory", "delete", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Delete"));
    }

    #[test]
    fn test_memory_delete_requires_confirmation() {
        aeterna()
            .args(["memory", "delete", "mem-123", "--layer", "project"])
            .assert()
            .success()
            .stderr(predicate::str::contains("skip this confirmation"));
    }

    #[test]
    fn test_memory_delete_invalid_layer() {
        aeterna()
            .args(["memory", "delete", "mem-123", "--layer", "invalid", "--yes"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("invalid").or(predicate::str::contains("Invalid")));
    }

    #[test]
    fn test_memory_delete_yes_without_server_fails() {
        aeterna()
            .args(["memory", "delete", "mem-123", "--layer", "project", "--yes"])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("not connected")
                    .or(predicate::str::contains("AETERNA_SERVER_URL")),
            );
    }

    #[test]
    fn test_memory_feedback_help() {
        aeterna()
            .args(["memory", "feedback", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("feedback"));
    }

    #[test]
    fn test_memory_feedback_invalid_type() {
        aeterna()
            .args([
                "memory",
                "feedback",
                "mem-123",
                "--layer",
                "project",
                "--feedback-type",
                "amazing",
                "--score",
                "0.5",
            ])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("Invalid feedback")
                    .or(predicate::str::contains("invalid")),
            );
    }

    #[test]
    fn test_memory_feedback_invalid_score() {
        aeterna()
            .args([
                "memory",
                "feedback",
                "mem-123",
                "--layer",
                "project",
                "--feedback-type",
                "helpful",
                "--score",
                "1.5",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("score").or(predicate::str::contains("Invalid")));
    }

    #[test]
    fn test_memory_feedback_valid_input_without_server_fails() {
        aeterna()
            .args([
                "memory",
                "feedback",
                "mem-123",
                "--layer",
                "project",
                "--feedback-type",
                "helpful",
                "--score",
                "0.5",
            ])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("not connected")
                    .or(predicate::str::contains("AETERNA_SERVER_URL")),
            );
    }

    #[test]
    fn test_memory_promote_help() {
        aeterna()
            .args(["memory", "promote", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Promote"));
    }

    #[test]
    fn test_memory_promote_invalid_target_layer() {
        aeterna()
            .args([
                "memory",
                "promote",
                "mem-123",
                "--from-layer",
                "project",
                "--to-layer",
                "invalid",
                "--dry-run",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("invalid").or(predicate::str::contains("Invalid")));
    }

    #[test]
    fn test_memory_promote_rejects_narrower_layer() {
        aeterna()
            .args([
                "memory",
                "promote",
                "mem-123",
                "--from-layer",
                "team",
                "--to-layer",
                "project",
                "--dry-run",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("promote").or(predicate::str::contains("narrower")));
    }

    #[test]
    fn test_memory_promote_dry_run_json() {
        aeterna()
            .args([
                "memory",
                "promote",
                "mem-123",
                "--from-layer",
                "project",
                "--to-layer",
                "team",
                "--dry-run",
                "--json",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("\"dryRun\""))
            .stdout(predicate::str::contains("\"memoryId\""));
    }
}

mod knowledge_subcommand {
    use super::*;
    use predicates::prelude::PredicateBooleanExt;
    use predicates::prelude::predicate;

    #[test]
    fn test_knowledge_help() {
        aeterna()
            .args(["knowledge", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("knowledge"))
            .stdout(predicate::str::contains("search"))
            .stdout(predicate::str::contains("get"));
    }

    #[test]
    fn test_knowledge_search_help() {
        aeterna()
            .args(["knowledge", "search", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Search"));
    }

    #[test]
    fn test_knowledge_search_dry_run() {
        aeterna()
            .args(["knowledge", "search", "architecture decisions", "--dry-run"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Dry Run"));
    }

    #[test]
    fn test_knowledge_get_help() {
        aeterna()
            .args(["knowledge", "get", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Get"));
    }

    #[test]
    fn test_knowledge_get_invalid_layer() {
        aeterna()
            .args(["knowledge", "get", "adrs/adr-001.md", "--layer", "invalid"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("invalid").or(predicate::str::contains("Invalid")));
    }

    #[test]
    fn test_knowledge_get_json_not_connected() {
        let env = mock_server();
        aeterna_mock(&env)
            .args(["knowledge", "get", "adrs/adr-001.md", "--json"])
            .assert()
            .success()
            .stdout(predicate::str::contains("\"status\""))
            .stdout(predicate::str::contains("not_connected"));
    }

    #[test]
    fn test_knowledge_list_invalid_layer() {
        aeterna()
            .args(["knowledge", "list", "--layer", "invalid"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("invalid").or(predicate::str::contains("Invalid")));
    }

    #[test]
    fn test_knowledge_check_help() {
        aeterna()
            .args(["knowledge", "check", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Check"));
    }

    #[test]
    fn test_knowledge_check_dry_run_json() {
        aeterna()
            .args([
                "knowledge",
                "check",
                "--dependency",
                "openssl",
                "--dry-run",
                "--json",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("\"dryRun\""))
            .stdout(predicate::str::contains("knowledge_check"));
    }

    #[test]
    fn test_knowledge_propose_invalid_type() {
        aeterna()
            .args([
                "knowledge",
                "propose",
                "Use PostgreSQL for primary data",
                "--knowledge-type",
                "memo",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("invalid").or(predicate::str::contains("Invalid")));
    }

    #[test]
    fn test_knowledge_propose_invalid_layer() {
        aeterna()
            .args([
                "knowledge",
                "propose",
                "Use PostgreSQL for primary data",
                "--layer",
                "session",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("invalid").or(predicate::str::contains("Invalid")));
    }

    #[test]
    fn test_knowledge_propose_requires_confirmation() {
        aeterna()
            .args(["knowledge", "propose", "Use PostgreSQL for primary data"])
            .assert()
            .success()
            .stderr(predicate::str::contains(
                "Use --yes to skip this confirmation",
            ));
    }

    #[test]
    fn test_knowledge_propose_yes_json_not_connected() {
        let env = mock_server();
        aeterna_mock(&env)
            .args([
                "knowledge",
                "propose",
                "Use PostgreSQL for primary data",
                "--yes",
                "--json",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("\"status\""))
            .stdout(predicate::str::contains("not_connected"));
    }

    #[test]
    fn test_knowledge_propose_help() {
        aeterna()
            .args(["knowledge", "propose", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Propose"));
    }
}

mod user_subcommand {
    use super::*;
    use predicates::prelude::PredicateBooleanExt;
    use predicates::prelude::predicate;

    #[test]
    fn test_user_help() {
        aeterna()
            .args(["user", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("user"))
            .stdout(predicate::str::contains("register"))
            .stdout(predicate::str::contains("whoami"));
    }

    #[test]
    fn test_user_whoami() {
        aeterna()
            .args(["user", "whoami"])
            .assert()
            .success()
            .stdout(predicate::str::contains("User"));
    }

    #[test]
    fn test_user_whoami_json() {
        aeterna()
            .args(["user", "whoami", "--json"])
            .assert()
            .success()
            .stdout(predicate::str::contains("\"user\""))
            .stdout(predicate::str::contains("\"tenant\""));
    }

    #[test]
    fn test_user_register_help() {
        aeterna()
            .args(["user", "register", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Register"));
    }

    #[test]
    fn test_user_register_dry_run() {
        aeterna()
            .args(["user", "register", "--dry-run"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Dry Run"));
    }

    #[test]
    fn test_user_list_help() {
        aeterna()
            .args(["user", "list", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("List"));
    }

    #[test]
    fn test_user_roles_help() {
        aeterna()
            .args(["user", "roles", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("role"));
    }

    #[test]
    fn test_user_roles_invalid_grant_role() {
        aeterna()
            .args([
                "user",
                "roles",
                "--user",
                "alice@example.com",
                "--grant",
                "superuser",
            ])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("Invalid role").or(predicate::str::contains("invalid")),
            );
    }

    #[test]
    fn test_user_roles_grant_json() {
        aeterna()
            .args([
                "user",
                "roles",
                "--user",
                "alice@example.com",
                "--grant",
                "developer",
                "--scope",
                "team",
                "--json",
            ])
            .assert()
            .failure()
            .stdout(predicate::str::contains("server_not_connected"))
            .stdout(predicate::str::contains("user_role_grant"))
            .stdout(predicate::str::contains("alice@example.com"));
    }

    #[test]
    fn test_user_roles_revoke_json() {
        aeterna()
            .args([
                "user",
                "roles",
                "--user",
                "alice@example.com",
                "--revoke",
                "developer",
                "--scope",
                "team",
                "--json",
            ])
            .assert()
            .failure()
            .stdout(predicate::str::contains("server_not_connected"))
            .stdout(predicate::str::contains("user_role_revoke"))
            .stdout(predicate::str::contains("alice@example.com"));
    }

    #[test]
    fn test_user_roles_list_json() {
        aeterna()
            .args(["user", "roles", "--user", "alice@example.com", "--json"])
            .assert()
            .failure()
            .stdout(predicate::str::contains("user_roles_list"))
            .stdout(predicate::str::contains("server_not_connected"))
            .stdout(predicate::str::contains("alice@example.com"));
    }

    #[test]
    fn test_user_invite_help() {
        aeterna()
            .args(["user", "invite", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Invite"));
    }

    #[test]
    fn test_user_invite_invalid_email() {
        aeterna()
            .args([
                "user",
                "invite",
                "not-an-email",
                "--org",
                "test-org",
                "--yes",
            ])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("Invalid email").or(predicate::str::contains("invalid")),
            );
    }

    #[test]
    fn test_user_invite_invalid_role() {
        aeterna()
            .args([
                "user",
                "invite",
                "test@example.com",
                "--org",
                "test-org",
                "--role",
                "superuser",
                "--yes",
            ])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("Invalid role").or(predicate::str::contains("invalid")),
            );
    }
}

mod admin_subcommand {
    use super::*;
    use predicates::prelude::PredicateBooleanExt;
    use predicates::prelude::predicate;

    #[test]
    fn test_admin_help() {
        aeterna()
            .args(["admin", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("admin"))
            .stdout(predicate::str::contains("health"))
            .stdout(predicate::str::contains("validate"))
            .stdout(predicate::str::contains("migrate"));
    }

    #[test]
    fn test_admin_health_help() {
        aeterna()
            .args(["admin", "health", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("health"))
            .stdout(predicate::str::contains("--component"));
    }

    #[test]
    fn test_admin_validate_help() {
        aeterna()
            .args(["admin", "validate", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Validate"))
            .stdout(predicate::str::contains("--strict"));
    }

    #[test]
    fn test_admin_validate_json_success_for_config() {
        let output = aeterna()
            .args(["admin", "validate", "--target", "config", "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(
            json.get("valid").and_then(serde_json::Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn test_admin_validate_strict_fails_for_policies() {
        aeterna()
            .args(["admin", "validate", "--target", "policies", "--strict"])
            .assert()
            .success()
            .stdout(
                predicate::str::contains("INVALID")
                    .or(predicate::str::contains("warnings treated as errors")),
            );
    }

    #[test]
    fn test_admin_migrate_help() {
        aeterna()
            .args(["admin", "migrate", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("migration"))
            .stdout(predicate::str::contains("--dry-run"));
    }

    #[test]
    fn test_admin_migrate_status() {
        aeterna()
            .args(["admin", "migrate", "status"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Migration"));
    }

    #[test]
    fn test_admin_drift_help() {
        aeterna()
            .args(["admin", "drift", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("drift"))
            .stdout(predicate::str::contains("--fix"));
    }

    #[test]
    fn test_admin_drift_fix() {
        aeterna()
            .args(["admin", "drift", "--fix"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Applying fixes"))
            .stdout(predicate::str::contains("Fixed"));
    }

    #[test]
    fn test_admin_drift_json() {
        let output = aeterna()
            .args(["admin", "drift", "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert!(json.get("drifts").is_some());
    }

    #[test]
    fn test_admin_export_help() {
        aeterna()
            .args(["admin", "export", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Export"))
            .stdout(predicate::str::contains("--format"));
    }

    #[test]
    fn test_admin_export_json_fails_without_server() {
        let output = aeterna()
            .args(["admin", "export", "--json"])
            .output()
            .expect("process ran");
        assert!(!output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("server_not_connected") || stdout.contains("not connected"),
            "expected server_not_connected in output, got: {stdout}",
        );
    }

    #[test]
    fn test_admin_import_help() {
        aeterna()
            .args(["admin", "import", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Import"))
            .stdout(predicate::str::contains("--mode"));
    }

    #[test]
    fn test_admin_import_missing_file_fails() {
        aeterna()
            .args(["admin", "import", "/definitely/missing/aeterna-import.json"])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("Import file not found")
                    .or(predicate::str::contains("does not exist")),
            );
    }
}

mod policy_subcommand {
    use super::*;
    use predicates::prelude::PredicateBooleanExt;
    use predicates::prelude::predicate;

    #[test]
    fn test_policy_help() {
        aeterna()
            .args(["policy", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("policy"))
            .stdout(predicate::str::contains("create"))
            .stdout(predicate::str::contains("list"))
            .stdout(predicate::str::contains("explain"));
    }

    #[test]
    fn test_policy_create_help() {
        aeterna()
            .args(["policy", "create", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Create"))
            .stdout(predicate::str::contains("--description"))
            .stdout(predicate::str::contains("--template"));
    }

    #[test]
    fn test_policy_create_dry_run_with_description() {
        aeterna()
            .args([
                "policy",
                "create",
                "--description",
                "Block critical CVEs",
                "--dry-run",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("Dry Run"));
    }

    #[test]
    fn test_policy_create_dry_run_with_template() {
        aeterna()
            .args([
                "policy",
                "create",
                "--template",
                "security-baseline",
                "--dry-run",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("Dry Run"))
            .stdout(predicate::str::contains("security-baseline"));
    }

    #[test]
    fn test_policy_create_missing_description_or_template() {
        aeterna()
            .args(["policy", "create"])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("description").or(predicate::str::contains("template")),
            );
    }

    #[test]
    fn test_policy_create_invalid_layer() {
        aeterna()
            .args([
                "policy",
                "create",
                "--description",
                "Test",
                "--layer",
                "invalid",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("invalid").or(predicate::str::contains("Invalid")));
    }

    #[test]
    fn test_policy_create_invalid_mode() {
        aeterna()
            .args([
                "policy",
                "create",
                "--description",
                "Test",
                "--mode",
                "required",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("invalid").or(predicate::str::contains("Invalid")));
    }

    #[test]
    fn test_policy_create_invalid_severity() {
        aeterna()
            .args([
                "policy",
                "create",
                "--description",
                "Test",
                "--severity",
                "critical",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("invalid").or(predicate::str::contains("Invalid")));
    }

    #[test]
    fn test_policy_list_help() {
        aeterna()
            .args(["policy", "list", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("List"))
            .stdout(predicate::str::contains("--layer"));
    }

    #[test]
    fn test_policy_explain_help() {
        aeterna()
            .args(["policy", "explain", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Explain"))
            .stdout(predicate::str::contains("--verbose"));
    }

    #[test]
    fn test_policy_simulate_help() {
        aeterna()
            .args(["policy", "simulate", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Simulate"))
            .stdout(predicate::str::contains("--scenario"));
    }

    #[test]
    fn test_policy_simulate_invalid_scenario() {
        aeterna()
            .args([
                "policy",
                "simulate",
                "policy-1",
                "--scenario",
                "deploy-app",
                "--input",
                "foo",
            ])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("Invalid scenario")
                    .or(predicate::str::contains("invalid")),
            );
    }

    #[test]
    fn test_policy_simulate_dry_run_json() {
        aeterna()
            .args([
                "policy",
                "simulate",
                "policy-1",
                "--scenario",
                "dependency-add",
                "--input",
                "openssl",
                "--dry-run",
                "--json",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("\"dryRun\""))
            .stdout(predicate::str::contains("policy_simulate"));
    }

    #[test]
    fn test_policy_simulate_json_not_connected() {
        aeterna()
            .args([
                "policy",
                "simulate",
                "policy-1",
                "--scenario",
                "dependency-add",
                "--input",
                "openssl",
                "--json",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("not_connected"))
            .stdout(predicate::str::contains("policy_simulate"));
    }

    #[test]
    fn test_policy_validate_help() {
        aeterna()
            .args(["policy", "validate", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Validate"))
            .stdout(predicate::str::contains("--strict"));
    }

    #[test]
    fn test_policy_draft_help() {
        aeterna()
            .args(["policy", "draft", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("draft"))
            .stdout(predicate::str::contains("--list"));
    }

    #[test]
    fn test_policy_draft_list_json() {
        aeterna()
            .args(["policy", "draft", "--list", "--json"])
            .assert()
            .success()
            .stdout(predicate::str::contains("policy_draft_list"))
            .stdout(predicate::str::contains("not_connected"));
    }

    #[test]
    fn test_policy_draft_submit_json() {
        aeterna()
            .args(["policy", "draft", "--submit", "draft-123", "--json"])
            .assert()
            .success()
            .stdout(predicate::str::contains("policy_draft_submit"))
            .stdout(predicate::str::contains("draft-123"));
    }

    #[test]
    fn test_policy_draft_missing_args_fails() {
        aeterna()
            .args(["policy", "draft"])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("No draft ID or action specified")
                    .or(predicate::str::contains("draft")),
            );
    }
}

mod govern_subcommand {
    use super::*;
    use predicates::prelude::PredicateBooleanExt;
    use predicates::prelude::predicate;

    #[test]
    fn test_govern_help() {
        aeterna()
            .args(["govern", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("govern"))
            .stdout(predicate::str::contains("status"))
            .stdout(predicate::str::contains("pending"))
            .stdout(predicate::str::contains("approve"));
    }

    #[test]
    fn test_govern_status_help() {
        aeterna()
            .args(["govern", "status", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("status"));
    }

    #[test]
    fn test_govern_pending_help() {
        aeterna()
            .args(["govern", "pending", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("pending"))
            .stdout(predicate::str::contains("request-type").or(predicate::str::contains("-t")));
    }

    #[test]
    fn test_govern_approve_help() {
        aeterna()
            .args(["govern", "approve", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Approve"));
    }

    #[test]
    fn test_govern_reject_help() {
        aeterna()
            .args(["govern", "reject", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Reject"));
    }

    #[test]
    fn test_govern_configure_help() {
        aeterna()
            .args(["govern", "configure", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Configure"))
            .stdout(predicate::str::contains("--template"));
    }

    #[test]
    fn test_govern_roles_help() {
        aeterna()
            .args(["govern", "roles", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("role"));
    }

    #[test]
    fn test_govern_audit_help() {
        aeterna()
            .args(["govern", "audit", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("audit"))
            .stdout(predicate::str::contains("--export"));
    }

    #[test]
    fn test_govern_status() {
        aeterna()
            .args(["govern", "status"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot show governance status"));
    }

    #[test]
    fn test_govern_status_json() {
        let output = aeterna()
            .args(["govern", "status", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "govern_status");
    }

    #[test]
    fn test_govern_status_verbose() {
        aeterna()
            .args(["govern", "status", "--verbose"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot show governance status"));
    }

    #[test]
    fn test_govern_pending() {
        aeterna()
            .args(["govern", "pending"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot list pending requests"));
    }

    #[test]
    fn test_govern_pending_json() {
        let output = aeterna()
            .args(["govern", "pending", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "govern_pending");
    }

    #[test]
    fn test_govern_pending_filter_by_type() {
        aeterna()
            .args(["govern", "pending", "-t", "policy"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot list pending requests"));
    }

    #[test]
    fn test_govern_pending_filter_by_layer() {
        aeterna()
            .args(["govern", "pending", "--layer", "org"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot list pending requests"));
    }

    #[test]
    fn test_govern_pending_filter_by_requestor() {
        aeterna()
            .args(["govern", "pending", "--requestor", "alice"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot list pending requests"));
    }

    #[test]
    fn test_govern_approve() {
        aeterna()
            .args(["govern", "approve", "req_test123", "--yes"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot approve request"));
    }

    #[test]
    fn test_govern_approve_json() {
        let output = aeterna()
            .args(["govern", "approve", "req_test123", "--yes", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "govern_approve");
        assert_eq!(json["request_id"], "req_test123");
    }

    #[test]
    fn test_govern_approve_with_comment() {
        aeterna()
            .args([
                "govern",
                "approve",
                "req_test123",
                "--yes",
                "--comment",
                "LGTM",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot approve request"));
    }

    #[test]
    fn test_govern_reject() {
        aeterna()
            .args([
                "govern",
                "reject",
                "req_test123",
                "--reason",
                "Needs security review",
                "--yes",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot reject request"));
    }

    #[test]
    fn test_govern_reject_json() {
        let output = aeterna()
            .args([
                "govern",
                "reject",
                "req_test123",
                "--reason",
                "Needs review",
                "--yes",
                "--json",
            ])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "govern_reject");
        assert_eq!(json["request_id"], "req_test123");
    }

    #[test]
    fn test_govern_reject_requires_reason() {
        aeterna()
            .args(["govern", "reject", "req_test123", "--reason", "", "--yes"])
            .assert()
            .failure();
    }

    #[test]
    fn test_govern_configure_show() {
        aeterna()
            .args(["govern", "configure", "--show"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot show governance config"));
    }

    #[test]
    fn test_govern_configure_show_json() {
        let output = aeterna()
            .args(["govern", "configure", "--show", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "govern_config_show");
    }

    #[test]
    fn test_govern_configure_list_templates() {
        aeterna()
            .args(["govern", "configure", "--list-templates"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Available Governance Templates"))
            .stdout(predicate::str::contains("standard"))
            .stdout(predicate::str::contains("strict"))
            .stdout(predicate::str::contains("permissive"));
    }

    #[test]
    fn test_govern_configure_template_standard() {
        aeterna()
            .args(["govern", "configure", "--template", "standard"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot update governance config"));
    }

    #[test]
    fn test_govern_configure_template_strict() {
        aeterna()
            .args(["govern", "configure", "--template", "strict"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot update governance config"));
    }

    #[test]
    fn test_govern_configure_template_permissive() {
        aeterna()
            .args(["govern", "configure", "--template", "permissive"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot update governance config"));
    }

    #[test]
    fn test_govern_configure_approval_mode() {
        aeterna()
            .args(["govern", "configure", "--approval-mode", "quorum"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot update governance config"));
    }

    #[test]
    fn test_govern_configure_min_approvers() {
        aeterna()
            .args(["govern", "configure", "--min-approvers", "3"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot update governance config"));
    }

    #[test]
    fn test_govern_configure_timeout_hours() {
        aeterna()
            .args(["govern", "configure", "--timeout-hours", "48"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot update governance config"));
    }

    #[test]
    fn test_govern_configure_auto_approve_enabled() {
        aeterna()
            .args(["govern", "configure", "--auto-approve", "true"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot update governance config"));
    }

    #[test]
    fn test_govern_configure_escalation_contact() {
        aeterna()
            .args([
                "govern",
                "configure",
                "--escalation-contact",
                "security-team@example.com",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot update governance config"));
    }

    #[test]
    fn test_govern_configure_json() {
        let output = aeterna()
            .args(["govern", "configure", "--approval-mode", "single", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "govern_config_update");
    }

    #[test]
    fn test_govern_roles_list() {
        aeterna()
            .args(["govern", "roles", "list"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot list governance roles"));
    }

    #[test]
    fn test_govern_roles_list_json() {
        let output = aeterna()
            .args(["govern", "roles", "list", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "govern_roles_list");
    }

    #[test]
    fn test_govern_roles_assign() {
        aeterna()
            .args([
                "govern",
                "roles",
                "assign",
                "--principal",
                "alice",
                "--role",
                "approver",
                "--scope",
                "org",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot assign governance role"));
    }

    #[test]
    fn test_govern_roles_revoke() {
        aeterna()
            .args([
                "govern",
                "roles",
                "revoke",
                "--principal",
                "alice",
                "--role",
                "approver",
                "--scope",
                "org",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Cannot revoke governance role"));
    }

    #[test]
    fn test_govern_roles_assign_json() {
        let output = aeterna()
            .args([
                "govern",
                "roles",
                "assign",
                "--principal",
                "bob",
                "--role",
                "admin",
                "--scope",
                "tenant",
                "--json",
            ])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "govern_role_assign");
    }

    #[test]
    fn test_govern_roles_revoke_json() {
        let output = aeterna()
            .args([
                "govern",
                "roles",
                "revoke",
                "--principal",
                "bob",
                "--role",
                "admin",
                "--scope",
                "tenant",
                "--json",
            ])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "govern_role_revoke");
    }

    #[test]
    fn test_govern_audit() {
        aeterna()
            .args(["govern", "audit"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot list governance audit entries",
            ));
    }

    #[test]
    fn test_govern_audit_json() {
        let output = aeterna()
            .args(["govern", "audit", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "govern_audit");
    }

    #[test]
    fn test_govern_audit_filter_by_action() {
        aeterna()
            .args(["govern", "audit", "--action", "approve"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot list governance audit entries",
            ));
    }

    #[test]
    fn test_govern_audit_filter_by_since() {
        aeterna()
            .args(["govern", "audit", "--since", "24h"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot list governance audit entries",
            ));
    }

    #[test]
    fn test_govern_audit_filter_by_actor() {
        aeterna()
            .args(["govern", "audit", "--actor", "alice"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot list governance audit entries",
            ));
    }

    #[test]
    fn test_govern_audit_filter_by_target_type() {
        aeterna()
            .args(["govern", "audit", "--target-type", "policy"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot list governance audit entries",
            ));
    }

    #[test]
    fn test_govern_audit_limit() {
        aeterna()
            .args(["govern", "audit", "--limit", "10"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot list governance audit entries",
            ));
    }

    #[test]
    fn test_govern_audit_export_csv() {
        aeterna()
            .args(["govern", "audit", "--export", "csv"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot list governance audit entries",
            ));
    }

    #[test]
    fn test_govern_audit_export_json_format() {
        aeterna()
            .args(["govern", "audit", "--export", "json"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot list governance audit entries",
            ));
    }
}

mod tenant_subcommand {
    use super::*;
    use predicates::prelude::predicate;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_tenant_help() {
        aeterna()
            .args(["tenant", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("tenant"))
            .stdout(predicate::str::contains("create"))
            .stdout(predicate::str::contains("repo-binding"));
    }

    #[test]
    fn test_tenant_list_json_not_connected() {
        let output = aeterna()
            .args(["tenant", "list", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "tenant_list");
    }

    #[test]
    fn test_tenant_show_json_not_connected() {
        let output = aeterna()
            .args(["tenant", "show", "acme", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "tenant_show");
        assert_eq!(json["tenant"], "acme");
    }

    #[test]
    fn test_tenant_deactivate_requires_yes() {
        aeterna()
            .args(["tenant", "deactivate", "acme"])
            .assert()
            .success()
            .stderr(predicate::str::contains("Use --yes to confirm"));
    }

    #[test]
    fn test_tenant_deactivate_yes_json_not_connected() {
        let output = aeterna()
            .args(["tenant", "deactivate", "acme", "--yes", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "tenant_deactivate");
        assert_eq!(json["tenant"], "acme");
    }

    #[test]
    fn test_tenant_use_writes_context() {
        let temp = TempDir::new().unwrap();
        aeterna()
            .current_dir(temp.path())
            .args(["tenant", "use", "acme"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Updated .aeterna/context.toml"));

        let content =
            fs::read_to_string(temp.path().join(".aeterna").join("context.toml")).unwrap();
        assert!(content.contains("tenant_id = \"acme\""));
    }

    #[test]
    fn test_tenant_repo_binding_show_json_not_connected() {
        let output = aeterna()
            .args(["tenant", "repo-binding", "show", "acme", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "tenant_repo_binding_show");
        assert_eq!(json["tenant"], "acme");
    }

    #[test]
    fn test_tenant_config_inspect_json_not_connected_for_platform_flow() {
        let output = aeterna()
            .args(["tenant", "config", "inspect", "--tenant", "acme", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "tenant_config_inspect");
        assert_eq!(json["tenant"], "acme");
    }

    #[test]
    fn test_tenant_connection_help_includes_subcommands() {
        aeterna()
            .args(["tenant", "connection", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("list"))
            .stdout(predicate::str::contains("grant"))
            .stdout(predicate::str::contains("revoke"));
    }

    #[test]
    fn test_tenant_connection_list_json_not_connected() {
        let output = aeterna()
            .args(["tenant", "connection", "list", "acme", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "connection_list");
        assert_eq!(json["tenant"], "acme");
    }

    #[test]
    fn test_tenant_connection_grant_json_not_connected() {
        let output = aeterna()
            .args([
                "tenant",
                "connection",
                "grant",
                "acme",
                "--connection",
                "conn-123",
                "--json",
            ])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "connection_grant");
        assert_eq!(json["tenant"], "acme");
        assert_eq!(json["connection"], "conn-123");
    }

    #[test]
    fn test_tenant_connection_revoke_json_not_connected() {
        let output = aeterna()
            .args([
                "tenant",
                "connection",
                "revoke",
                "acme",
                "--connection",
                "conn-123",
                "--json",
            ])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "connection_revoke");
        assert_eq!(json["tenant"], "acme");
        assert_eq!(json["connection"], "conn-123");
    }
}

mod agent_subcommand {
    use super::*;
    use predicates::prelude::predicate;

    #[test]
    fn test_agent_help() {
        aeterna()
            .args(["agent", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("agent"))
            .stdout(predicate::str::contains("register"))
            .stdout(predicate::str::contains("list"))
            .stdout(predicate::str::contains("show"));
    }

    #[test]
    fn test_agent_register_help() {
        aeterna()
            .args(["agent", "register", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Register"))
            .stdout(predicate::str::contains("--agent-type"));
    }

    #[test]
    fn test_agent_register_dry_run() {
        aeterna()
            .args(["agent", "register", "test-agent", "--dry-run"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Dry Run"));
    }

    #[test]
    fn test_agent_register_dry_run_json() {
        let output = aeterna()
            .args(["agent", "register", "test-agent", "--dry-run", "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(
            json.get("dryRun").and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert!(json.get("agent").is_some());
    }

    #[test]
    fn test_agent_register_dry_run_with_description() {
        aeterna()
            .args([
                "agent",
                "register",
                "test-agent",
                "--dry-run",
                "--description",
                "Test agent for CI",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("Dry Run"))
            .stdout(predicate::str::contains("Test agent for CI"));
    }

    #[test]
    fn test_agent_register_dry_run_with_type() {
        aeterna()
            .args([
                "agent",
                "register",
                "opencode-agent",
                "--dry-run",
                "--agent-type",
                "opencode",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("opencode"));
    }

    #[test]
    fn test_agent_register_invalid_type() {
        aeterna()
            .args([
                "agent",
                "register",
                "test-agent",
                "--agent-type",
                "invalid-type",
            ])
            .assert()
            .failure();
    }

    #[test]
    fn test_agent_register_with_delegated_by() {
        aeterna()
            .args([
                "agent",
                "register",
                "my-agent",
                "--dry-run",
                "--delegated-by",
                "alice@acme.com",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("alice@acme.com"));
    }

    #[test]
    fn test_agent_list_help() {
        aeterna()
            .args(["agent", "list", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("List"))
            .stdout(predicate::str::contains("--all"));
    }

    #[test]
    fn test_agent_list() {
        let env = mock_server();
        aeterna_mock(&env)
            .args(["agent", "list"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Agents"));
    }

    #[test]
    fn test_agent_list_json() {
        let env = mock_server();
        let output = aeterna_mock(&env)
            .args(["agent", "list", "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert!(json.get("operation").is_some());
    }

    #[test]
    fn test_agent_list_all() {
        let env = mock_server();
        aeterna_mock(&env)
            .args(["agent", "list", "--all"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Agents"));
    }

    #[test]
    fn test_agent_list_filter_by_delegated_by() {
        let env = mock_server();
        aeterna_mock(&env)
            .args(["agent", "list", "--delegated-by", "alice"])
            .assert()
            .success()
            .stdout(predicate::str::contains("alice"));
    }

    #[test]
    fn test_agent_list_filter_by_type() {
        let env = mock_server();
        aeterna_mock(&env)
            .args(["agent", "list", "--agent-type", "autonomous"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Agents"));
    }

    #[test]
    fn test_agent_show_help() {
        aeterna()
            .args(["agent", "show", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Show"))
            .stdout(predicate::str::contains("--verbose"));
    }

    #[test]
    fn test_agent_show() {
        let env = mock_server();
        aeterna_mock(&env)
            .args(["agent", "show", "agent-test-123"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Agent: agent-test-123"));
    }

    #[test]
    fn test_agent_show_json() {
        let env = mock_server();
        let output = aeterna_mock(&env)
            .args(["agent", "show", "agent-test-123", "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(
            json.get("agentId").and_then(|v| v.as_str()),
            Some("agent-test-123")
        );
    }

    #[test]
    fn test_agent_show_verbose() {
        let env = mock_server();
        aeterna_mock(&env)
            .args(["agent", "show", "agent-test-123", "--verbose"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Agent: agent-test-123"))
            .stdout(predicate::str::contains("agentId"));
    }

    #[test]
    fn test_agent_permissions_help() {
        aeterna()
            .args(["agent", "permissions", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("permission"))
            .stdout(predicate::str::contains("--grant"));
    }

    #[test]
    fn test_agent_permissions_list() {
        let env = mock_server();
        aeterna_mock(&env)
            .args(["agent", "permissions", "agent-test-123", "--list"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Permissions"));
    }

    #[test]
    fn test_agent_permissions_list_json() {
        let env = mock_server();
        let output = aeterna_mock(&env)
            .args(["agent", "permissions", "agent-test-123", "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert!(json.get("operation").is_some());
    }

    #[test]
    fn test_agent_permissions_grant() {
        let env = mock_server();
        aeterna_mock(&env)
            .args([
                "agent",
                "permissions",
                "agent-test-123",
                "--grant",
                "memory:read",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("Grant Agent Permission"));
    }

    #[test]
    fn test_agent_permissions_grant_json() {
        let env = mock_server();
        let output = aeterna_mock(&env)
            .args([
                "agent",
                "permissions",
                "agent-test-123",
                "--grant",
                "memory:write",
                "--json",
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(
            json.get("operation").and_then(|v| v.as_str()),
            Some("agent_permission_grant")
        );
    }

    #[test]
    fn test_agent_permissions_grant_with_scope() {
        let env = mock_server();
        aeterna_mock(&env)
            .args([
                "agent",
                "permissions",
                "agent-test-123",
                "--grant",
                "knowledge:read",
                "--scope",
                "org",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("Scope"));
    }

    #[test]
    fn test_agent_permissions_grant_invalid() {
        aeterna()
            .args([
                "agent",
                "permissions",
                "agent-test-123",
                "--grant",
                "invalid:permission",
            ])
            .assert()
            .failure();
    }

    #[test]
    fn test_agent_permissions_revoke() {
        let env = mock_server();
        aeterna_mock(&env)
            .args([
                "agent",
                "permissions",
                "agent-test-123",
                "--revoke",
                "memory:write",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("Revoke Agent Permission"));
    }

    #[test]
    fn test_agent_permissions_revoke_json() {
        let env = mock_server();
        let output = aeterna_mock(&env)
            .args([
                "agent",
                "permissions",
                "agent-test-123",
                "--revoke",
                "memory:write",
                "--json",
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(
            json.get("operation").and_then(|v| v.as_str()),
            Some("agent_permission_revoke")
        );
    }

    #[test]
    fn test_agent_revoke_help() {
        aeterna()
            .args(["agent", "revoke", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Revoke"))
            .stdout(predicate::str::contains("--force"));
    }

    #[test]
    fn test_agent_revoke() {
        let env = mock_server();
        aeterna_mock(&env)
            .args(["agent", "revoke", "agent-test-123"])
            .assert()
            .success()
            .stderr(predicate::str::contains("--force"));
    }

    #[test]
    fn test_agent_revoke_json() {
        let env = mock_server();
        let output = aeterna_mock(&env)
            .args(["agent", "revoke", "agent-test-123", "--force", "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert!(json.get("operation").is_some() || json.get("agentId").is_some());
    }

    #[test]
    fn test_agent_revoke_force() {
        let env = mock_server();
        aeterna_mock(&env)
            .args(["agent", "revoke", "agent-test-123", "--force"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Revoked Agent"));
    }
}

mod org_subcommand {
    use super::*;
    use predicates::prelude::predicate;

    #[test]
    fn test_org_help() {
        aeterna()
            .args(["org", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("org"))
            .stdout(predicate::str::contains("create"))
            .stdout(predicate::str::contains("list"))
            .stdout(predicate::str::contains("show"));
    }

    #[test]
    fn test_org_create_help() {
        aeterna()
            .args(["org", "create", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Create"))
            .stdout(predicate::str::contains("--description"));
    }

    #[test]
    fn test_org_create_dry_run() {
        aeterna()
            .args(["org", "create", "platform-engineering", "--dry-run"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Dry Run"));
    }

    #[test]
    fn test_org_create_dry_run_json() {
        let output = aeterna()
            .args(["org", "create", "platform-eng", "--dry-run", "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(
            json.get("dryRun").and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert!(json.get("org").is_some());
    }

    #[test]
    fn test_org_create_dry_run_with_description() {
        aeterna()
            .args([
                "org",
                "create",
                "product-eng",
                "--dry-run",
                "--description",
                "Product Engineering team",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("Dry Run"))
            .stdout(predicate::str::contains("Product Engineering"));
    }

    #[test]
    fn test_org_create_dry_run_with_company() {
        aeterna()
            .args([
                "org",
                "create",
                "security",
                "--dry-run",
                "--tenant-root",
                "acme-corp",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("acme-corp"));
    }

    #[test]
    fn test_org_list_help() {
        aeterna()
            .args(["org", "list", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("List"))
            .stdout(predicate::str::contains("--all"));
    }

    #[test]
    fn test_org_list() {
        aeterna()
            .args(["org", "list"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot list organizations: server not connected",
            ));
    }

    #[test]
    fn test_org_list_json() {
        let output = aeterna()
            .args(["org", "list", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "org_list");
    }

    #[test]
    fn test_org_list_all() {
        aeterna()
            .args(["org", "list", "--all"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot list organizations: server not connected",
            ));
    }

    #[test]
    fn test_org_list_filter_by_company() {
        aeterna()
            .args(["org", "list", "--tenant-root", "acme-corp"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot list organizations: server not connected",
            ));
    }

    #[test]
    fn test_org_show_help() {
        aeterna()
            .args(["org", "show", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Show"))
            .stdout(predicate::str::contains("--verbose"));
    }

    #[test]
    fn test_org_show() {
        aeterna()
            .args(["org", "show", "platform-eng"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot show organization 'platform-eng': server not connected",
            ));
    }

    #[test]
    fn test_org_show_json() {
        let output = aeterna()
            .args(["org", "show", "platform-eng", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "org_show");
        assert_eq!(json["orgId"], "platform-eng");
    }

    #[test]
    fn test_org_show_verbose() {
        aeterna()
            .args(["org", "show", "platform-eng", "--verbose"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot show organization 'platform-eng': server not connected",
            ));
    }

    #[test]
    fn test_org_show_no_arg_uses_context() {
        aeterna()
            .args(["org", "show"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("No organization specified"));
    }

    #[test]
    fn test_org_members_help() {
        aeterna()
            .args(["org", "members", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("member"))
            .stdout(predicate::str::contains("--add"));
    }

    #[test]
    fn test_org_members_list() {
        aeterna()
            .args(["org", "members", "--org", "platform-eng"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot list members for organization 'platform-eng': server not connected",
            ));
    }

    #[test]
    fn test_org_members_list_json() {
        let output = aeterna()
            .args(["org", "members", "--org", "platform-eng", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "org_members_list");
        assert_eq!(json["orgId"], "platform-eng");
    }

    #[test]
    fn test_org_members_add() {
        aeterna()
            .args([
                "org",
                "members",
                "--org",
                "platform-eng",
                "--add",
                "alice@acme.com",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot add member to organization 'platform-eng': server not connected",
            ));
    }

    #[test]
    fn test_org_members_add_json() {
        let output = aeterna()
            .args([
                "org",
                "members",
                "--org",
                "platform-eng",
                "--add",
                "bob@acme.com",
                "--json",
            ])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "org_member_add");
        assert_eq!(json["orgId"], "platform-eng");
        assert_eq!(json["userId"], "bob@acme.com");
    }

    #[test]
    fn test_org_members_add_with_role() {
        aeterna()
            .args([
                "org",
                "members",
                "--org",
                "platform-eng",
                "--add",
                "carol@acme.com",
                "--role",
                "techlead",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot add member to organization 'platform-eng': server not connected",
            ));
    }

    #[test]
    fn test_org_members_add_invalid_role() {
        aeterna()
            .args([
                "org",
                "members",
                "--org",
                "platform-eng",
                "--add",
                "dave@acme.com",
                "--role",
                "invalid-role",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Invalid role"));
    }

    #[test]
    fn test_org_members_add_with_org() {
        aeterna()
            .args([
                "org",
                "members",
                "--org",
                "platform-eng",
                "--add",
                "eve@acme.com",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot add member to organization 'platform-eng': server not connected",
            ));
    }

    #[test]
    fn test_org_members_remove() {
        aeterna()
            .args([
                "org",
                "members",
                "--org",
                "platform-eng",
                "--remove",
                "alice@acme.com",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot remove member from organization 'platform-eng': server not connected",
            ));
    }

    #[test]
    fn test_org_members_remove_json() {
        let output = aeterna()
            .args([
                "org",
                "members",
                "--org",
                "platform-eng",
                "--remove",
                "bob@acme.com",
                "--json",
            ])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "org_member_remove");
        assert_eq!(json["orgId"], "platform-eng");
        assert_eq!(json["userId"], "bob@acme.com");
    }

    #[test]
    fn test_org_members_set_role() {
        aeterna()
            .args([
                "org",
                "members",
                "--org",
                "platform-eng",
                "--set-role",
                "alice@acme.com",
                "--role",
                "architect",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot update member role in organization 'platform-eng': server not connected",
            ));
    }

    #[test]
    fn test_org_members_set_role_json() {
        let output = aeterna()
            .args([
                "org",
                "members",
                "--org",
                "platform-eng",
                "--set-role",
                "bob@acme.com",
                "--role",
                "admin",
                "--json",
            ])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "org_member_set_role");
        assert_eq!(json["orgId"], "platform-eng");
        assert_eq!(json["userId"], "bob@acme.com");
    }

    #[test]
    fn test_org_members_set_role_missing_role() {
        aeterna()
            .args([
                "org",
                "members",
                "--org",
                "platform-eng",
                "--set-role",
                "alice@acme.com",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Missing --role"));
    }

    #[test]
    fn test_org_use_help() {
        aeterna()
            .args(["org", "use", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Set default"));
    }

    #[test]
    fn test_org_use() {
        aeterna()
            .args(["org", "use", "platform-eng"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Set Default Organization"))
            .stdout(predicate::str::contains("platform-eng"));
    }
}

mod team_subcommand {
    use super::*;
    use predicates::prelude::predicate;

    #[test]
    fn test_team_help() {
        aeterna()
            .args(["team", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("team"))
            .stdout(predicate::str::contains("create"))
            .stdout(predicate::str::contains("list"));
    }

    #[test]
    fn test_team_create_help() {
        aeterna()
            .args(["team", "create", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Create"))
            .stdout(predicate::str::contains("--description"))
            .stdout(predicate::str::contains("--dry-run"));
    }

    #[test]
    fn test_team_create_dry_run() {
        aeterna()
            .args(["team", "create", "api-team", "--dry-run"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Team Create (Dry Run)"))
            .stdout(predicate::str::contains("api-team"))
            .stderr(predicate::str::contains("Dry run mode - team not created."));
    }

    #[test]
    fn test_team_create_dry_run_json() {
        let output = aeterna()
            .args(["team", "create", "api-team", "--dry-run", "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["dryRun"], true);
        assert_eq!(json["operation"], "team_create");
        assert_eq!(json["team"]["name"], "api-team");
    }

    #[test]
    fn test_team_create_dry_run_with_description() {
        aeterna()
            .args([
                "team",
                "create",
                "api-team",
                "--description",
                "API development team",
                "--dry-run",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("api-team"))
            .stdout(predicate::str::contains("API development team"));
    }

    #[test]
    fn test_team_create_dry_run_with_org() {
        aeterna()
            .args([
                "team",
                "create",
                "api-team",
                "--org",
                "platform-eng",
                "--dry-run",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("api-team"))
            .stdout(predicate::str::contains("platform-eng"));
    }

    #[test]
    fn test_team_create_dry_run_with_org_json() {
        let output = aeterna()
            .args([
                "team",
                "create",
                "api-team",
                "--org",
                "platform-eng",
                "--dry-run",
                "--json",
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["team"]["orgId"], "platform-eng");
    }

    #[test]
    fn test_team_list_help() {
        aeterna()
            .args(["team", "list", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("List"))
            .stdout(predicate::str::contains("--org"))
            .stdout(predicate::str::contains("--all"));
    }

    #[test]
    fn test_team_list() {
        aeterna()
            .args(["team", "list"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot list teams: server not connected",
            ));
    }

    #[test]
    fn test_team_list_json() {
        let output = aeterna()
            .args(["team", "list", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "team_list");
    }

    #[test]
    fn test_team_list_all() {
        aeterna()
            .args(["team", "list", "--all"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot list teams: server not connected",
            ));
    }

    #[test]
    fn test_team_list_filter_by_org() {
        aeterna()
            .args(["team", "list", "--org", "platform-eng"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot list teams: server not connected",
            ));
    }

    #[test]
    fn test_team_list_filter_by_org_json() {
        let output = aeterna()
            .args(["team", "list", "--org", "platform-eng", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "team_list");
    }

    #[test]
    fn test_team_show_help() {
        aeterna()
            .args(["team", "show", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Show"))
            .stdout(predicate::str::contains("--verbose"))
            .stdout(predicate::str::contains("--json"));
    }

    #[test]
    fn test_team_show() {
        aeterna()
            .args(["team", "show", "api-team"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot show team 'api-team': server not connected",
            ));
    }

    #[test]
    fn test_team_show_json() {
        let output = aeterna()
            .args(["team", "show", "api-team", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "team_show");
        assert_eq!(json["teamId"], "api-team");
    }

    #[test]
    fn test_team_show_verbose() {
        aeterna()
            .args(["team", "show", "api-team", "--verbose"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot show team 'api-team': server not connected",
            ));
    }

    #[test]
    fn test_team_show_no_arg_uses_context() {
        aeterna()
            .args(["team", "show"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("No team specified"));
    }

    #[test]
    fn test_team_members_help() {
        aeterna()
            .args(["team", "members", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Manage team members"))
            .stdout(predicate::str::contains("--add"))
            .stdout(predicate::str::contains("--remove"))
            .stdout(predicate::str::contains("--set-role"));
    }

    #[test]
    fn test_team_members_list() {
        aeterna()
            .args(["team", "members", "--team", "api-team"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot list members for team 'api-team': server not connected",
            ));
    }

    #[test]
    fn test_team_members_list_json() {
        let output = aeterna()
            .args(["team", "members", "--team", "api-team", "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "team_members_list");
        assert_eq!(json["teamId"], "api-team");
    }

    #[test]
    fn test_team_members_list_with_team() {
        aeterna()
            .args(["team", "members", "--team", "api-team"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot list members for team 'api-team': server not connected",
            ));
    }

    #[test]
    fn test_team_members_add() {
        aeterna()
            .args([
                "team",
                "members",
                "--team",
                "api-team",
                "--add",
                "alice@example.com",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot add member to team 'api-team': server not connected",
            ));
    }

    #[test]
    fn test_team_members_add_json() {
        let output = aeterna()
            .args([
                "team",
                "members",
                "--team",
                "api-team",
                "--add",
                "alice@example.com",
                "--json",
            ])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "team_member_add");
        assert_eq!(json["teamId"], "api-team");
        assert_eq!(json["userId"], "alice@example.com");
    }

    #[test]
    fn test_team_members_add_with_role() {
        aeterna()
            .args([
                "team",
                "members",
                "--team",
                "api-team",
                "--add",
                "alice@example.com",
                "--role",
                "techlead",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot add member to team 'api-team': server not connected",
            ));
    }

    #[test]
    fn test_team_members_add_with_team() {
        aeterna()
            .args([
                "team",
                "members",
                "--team",
                "api-team",
                "--add",
                "alice@example.com",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot add member to team 'api-team': server not connected",
            ));
    }

    #[test]
    fn test_team_members_add_invalid_role() {
        aeterna()
            .args([
                "team",
                "members",
                "--team",
                "api-team",
                "--add",
                "alice@example.com",
                "--role",
                "superuser",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Invalid team role"))
            .stderr(predicate::str::contains("developer, techlead, architect"));
    }

    #[test]
    fn test_team_members_remove() {
        aeterna()
            .args([
                "team",
                "members",
                "--team",
                "api-team",
                "--remove",
                "bob@example.com",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot remove member from team 'api-team': server not connected",
            ));
    }

    #[test]
    fn test_team_members_remove_json() {
        let output = aeterna()
            .args([
                "team",
                "members",
                "--team",
                "api-team",
                "--remove",
                "bob@example.com",
                "--json",
            ])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "team_member_remove");
        assert_eq!(json["teamId"], "api-team");
        assert_eq!(json["userId"], "bob@example.com");
    }

    #[test]
    fn test_team_members_set_role() {
        aeterna()
            .args([
                "team",
                "members",
                "--team",
                "api-team",
                "--set-role",
                "alice@example.com",
                "--role",
                "architect",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Cannot update member role in team 'api-team': server not connected",
            ));
    }

    #[test]
    fn test_team_members_set_role_json() {
        let output = aeterna()
            .args([
                "team",
                "members",
                "--team",
                "api-team",
                "--set-role",
                "alice@example.com",
                "--role",
                "techlead",
                "--json",
            ])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert_eq!(json["error"], "server_not_connected");
        assert_eq!(json["operation"], "team_member_set_role");
        assert_eq!(json["teamId"], "api-team");
        assert_eq!(json["userId"], "alice@example.com");
    }

    #[test]
    fn test_team_members_set_role_missing_role() {
        aeterna()
            .args([
                "team",
                "members",
                "--team",
                "api-team",
                "--set-role",
                "alice@example.com",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Missing --role"));
    }

    #[test]
    fn test_team_use_help() {
        aeterna()
            .args(["team", "use", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Set default team"));
    }

    #[test]
    fn test_team_use() {
        aeterna()
            .args(["team", "use", "api-team"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Set Default Team"))
            .stdout(predicate::str::contains("api-team"));
    }
}

mod context_subcommand {
    use super::*;
    use predicates::prelude::PredicateBooleanExt;
    use predicates::prelude::predicate;

    #[test]
    fn test_context_help() {
        aeterna()
            .args(["context", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("context"))
            .stdout(predicate::str::contains("show"))
            .stdout(predicate::str::contains("set"));
    }

    #[test]
    fn test_context_show() {
        aeterna()
            .args(["context", "show"])
            .assert()
            .success()
            .stdout(
                predicate::str::contains("tenant")
                    .or(predicate::str::contains("Tenant"))
                    .or(predicate::str::contains("Context")),
            );
    }

    #[test]
    fn test_context_show_json() {
        aeterna()
            .args(["context", "show", "--json"])
            .assert()
            .success()
            .stdout(predicate::str::contains("tenant_id").or(predicate::str::contains("tenantId")));
    }

    #[test]
    fn test_context_set_help() {
        aeterna()
            .args(["context", "set", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Set"));
    }

    #[test]
    fn test_context_clear_help() {
        aeterna()
            .args(["context", "clear", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Clear"));
    }
}

mod error_handling {
    use super::*;
    use predicates::prelude::predicate;

    #[test]
    fn test_unknown_command() {
        aeterna()
            .arg("nonexistent-command")
            .assert()
            .failure()
            .stderr(predicate::str::contains("error"));
    }

    #[test]
    fn test_unknown_subcommand() {
        aeterna()
            .args(["memory", "nonexistent"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("error"));
    }

    #[test]
    fn test_missing_required_argument() {
        aeterna()
            .args(["memory", "delete"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("required"));
    }

    #[test]
    fn test_invalid_flag_value() {
        aeterna()
            .args(["memory", "search", "query", "--limit", "not-a-number"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("invalid"));
    }
}

mod json_output {
    use super::*;

    #[test]
    fn test_memory_add_json_structure() {
        let output = aeterna()
            .args(["memory", "add", "Test content", "--dry-run", "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert!(json.get("dryRun").is_some());
        assert!(json.get("content").is_some());
    }

    #[test]
    fn test_user_whoami_json_structure() {
        let output = aeterna()
            .args(["user", "whoami", "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert!(json.get("user").is_some());
    }

    #[test]
    fn test_context_show_json_structure() {
        let output = aeterna()
            .args(["context", "show", "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        let has_valid_structure =
            json.is_array() || json.get("tenantId").is_some() || json.get("tenant_id").is_some();
        assert!(
            has_valid_structure,
            "Expected array or object with tenant info"
        );
    }
}

mod layer_validation {
    use super::*;

    #[test]
    fn test_memory_add_valid_layers() {
        let valid_layers = [
            "agent", "user", "session", "project", "team", "org", "tenant",
        ];
        for layer in valid_layers {
            aeterna()
                .args(["memory", "add", "Test", "--layer", layer, "--dry-run"])
                .assert()
                .success();
        }
    }

    #[test]
    fn test_policy_create_valid_layers() {
        let valid_layers = ["tenant", "org", "team", "project"];
        for layer in valid_layers {
            aeterna()
                .args([
                    "policy",
                    "create",
                    "--description",
                    "Test",
                    "--layer",
                    layer,
                    "--dry-run",
                ])
                .assert()
                .success();
        }
    }
}

mod check_subcommand {
    use super::*;
    use predicates::prelude::predicate;

    #[test]
    fn test_check_help() {
        aeterna()
            .args(["check", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("check"))
            .stdout(predicate::str::contains("--target"))
            .stdout(predicate::str::contains("--strict"));
    }

    #[test]
    fn test_check_default() {
        let output = aeterna().args(["check"]).output().expect("process ran");
        assert!(
            !output.status.success(),
            "check without a live backend must exit non-zero"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{stderr}{stdout}");

        assert!(combined.contains("Constraint Validation"));
        assert!(
            combined.contains("Cannot connect to Aeterna server")
                || combined.contains("live Aeterna server")
        );
    }

    #[test]
    fn test_check_json() {
        let output = aeterna()
            .args(["check", "--json"])
            .output()
            .expect("process ran");

        assert!(!output.status.success());

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("Valid JSON");
        assert!(json.get("success").is_some());
        assert!(json.get("context").is_some());
        assert!(json.get("summary").is_some());
        assert_eq!(json["success"], false);
        assert_eq!(json["error"], "server_not_connected");
    }

    #[test]
    fn test_check_target_policies() {
        let output = aeterna()
            .args(["check", "--target", "policies"])
            .output()
            .expect("process ran");
        assert!(!output.status.success());
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(combined.contains("POLICIES"));
    }

    #[test]
    fn test_check_target_dependencies() {
        let output = aeterna()
            .args(["check", "--target", "dependencies"])
            .output()
            .expect("process ran");
        assert!(!output.status.success());
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(combined.contains("DEPENDENCIES"));
    }

    #[test]
    fn test_check_target_architecture() {
        let output = aeterna()
            .args(["check", "--target", "architecture"])
            .output()
            .expect("process ran");
        assert!(!output.status.success());
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(combined.contains("ARCHITECTURE"));
    }

    #[test]
    fn test_check_target_security() {
        let output = aeterna()
            .args(["check", "--target", "security"])
            .output()
            .expect("process ran");
        assert!(!output.status.success());
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(combined.contains("SECURITY"));
    }

    #[test]
    fn test_check_target_all() {
        let output = aeterna()
            .args(["check", "--target", "all"])
            .output()
            .expect("process ran");
        assert!(!output.status.success());
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(combined.contains("ALL"));
    }

    #[test]
    fn test_check_strict() {
        aeterna().args(["check", "--strict"]).assert().failure();
    }

    #[test]
    fn test_check_violations_only() {
        aeterna()
            .args(["check", "--violations-only"])
            .assert()
            .failure();
    }

    #[test]
    fn test_check_json_with_target() {
        let output = aeterna()
            .args(["check", "--json", "--target", "security"])
            .output()
            .expect("process ran");

        assert!(!output.status.success());

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("Valid JSON");
        assert_eq!(json["target"], "security");
    }

    #[test]
    fn test_check_json_strict_flag() {
        let output = aeterna()
            .args(["check", "--json", "--strict"])
            .output()
            .expect("process ran");

        assert!(!output.status.success());

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("Valid JSON");
        assert_eq!(json["strict"], true);
    }

    #[test]
    fn test_check_with_path() {
        aeterna().args(["check", "."]).assert().failure();
    }
}

mod sync_subcommand {
    use super::*;
    use predicates::prelude::PredicateBooleanExt;
    use predicates::prelude::predicate;

    #[test]
    fn test_sync_help() {
        aeterna()
            .args(["sync", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("sync"))
            .stdout(predicate::str::contains("--direction"))
            .stdout(predicate::str::contains("--dry-run"));
    }

    #[test]
    fn test_sync_default() {
        let env = mock_server();
        aeterna_mock(&env)
            .args(["sync"])
            .assert()
            .failure()
            .stdout(predicate::str::contains("Memory-Knowledge Sync"));
    }

    #[test]
    fn test_sync_dry_run() {
        let env = mock_server();
        aeterna_mock(&env)
            .args(["sync", "--dry-run"])
            .assert()
            .failure()
            .stdout(
                predicate::str::contains("Sync")
                    .or(predicate::str::contains("sync"))
                    .or(predicate::str::contains("Analyzing")),
            );
    }

    #[test]
    fn test_sync_json() {
        let env = mock_server();
        let output = aeterna_mock(&env)
            .args(["sync", "--json"])
            .output()
            .expect("process ran");

        assert!(!output.status.success());

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("Valid JSON");
        assert_eq!(json["success"], false);
        // The sync command returns "unsupported" locally (not a server round-trip)
        assert!(json["error"].is_string());
    }

    #[test]
    fn test_sync_json_dry_run() {
        let env = mock_server();
        let output = aeterna_mock(&env)
            .args(["sync", "--json", "--dry-run"])
            .output()
            .expect("process ran");

        assert!(!output.status.success());

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("Valid JSON");
        // dry_run may or may not be set depending on how far the command gets
        assert!(json.is_object());
    }

    #[test]
    fn test_sync_direction_all() {
        let env = mock_server();
        aeterna_mock(&env)
            .args(["sync", "--direction", "all"])
            .assert()
            .failure()
            .stdout(predicate::str::contains("ALL"));
    }

    #[test]
    fn test_sync_direction_memory_to_knowledge() {
        let env = mock_server();
        aeterna_mock(&env)
            .args(["sync", "--direction", "memory-to-knowledge"])
            .assert()
            .failure()
            .stdout(predicate::str::contains("MEMORY-TO-KNOWLEDGE"));
    }

    #[test]
    fn test_sync_verbose() {
        let env = mock_server();
        aeterna_mock(&env)
            .args(["sync", "--verbose"])
            .assert()
            .failure()
            .stdout(predicate::str::contains("Analyzing sync state"));
    }

    #[test]
    fn test_sync_force() {
        let env = mock_server();
        aeterna_mock(&env)
            .args(["sync", "--force"])
            .assert()
            .failure();
    }
}

mod codesearch_runtime_subcommand {
    use super::*;
    use predicates::prelude::PredicateBooleanExt;
    use predicates::prelude::predicate;

    #[test]
    fn test_codesearch_help() {
        aeterna()
            .args(["code-search", "--help"])
            .assert()
            .success()
            .stdout(
                predicate::str::contains("code search").or(predicate::str::contains("Search code")),
            );
    }
}

mod status_subcommand {
    use super::*;
    use predicates::prelude::PredicateBooleanExt;
    use predicates::prelude::predicate;

    #[test]
    fn test_status_help() {
        aeterna()
            .args(["status", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("status"));
    }

    #[test]
    fn test_status_default() {
        aeterna().args(["status"]).assert().success().stdout(
            predicate::str::contains("Status")
                .or(predicate::str::contains("status"))
                .or(predicate::str::contains("Connection")),
        );
    }

    #[test]
    fn test_status_json() {
        let output = aeterna()
            .args(["status", "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert!(json.get("tenant_id").is_some() || json.get("hints").is_some());
    }

    #[test]
    fn test_status_verbose() {
        aeterna().args(["status", "--verbose"]).assert().success();
    }
}

mod init_subcommand {
    use super::*;
    use predicates::prelude::predicate;

    #[test]
    fn test_init_help() {
        aeterna()
            .args(["init", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("init"))
            .stdout(predicate::str::contains("--tenant-id"))
            .stdout(predicate::str::contains("--force"));
    }

    #[test]
    fn test_init_preset_flag_in_help() {
        aeterna()
            .args(["init", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--preset"));
    }

    #[test]
    fn test_init_path_flag_in_help() {
        aeterna()
            .args(["init", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--path"));
    }
}

mod hints_subcommand {
    use super::*;
    use predicates::prelude::PredicateBooleanExt;
    use predicates::prelude::predicate;

    #[test]
    fn test_hints_help() {
        aeterna()
            .args(["hints", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("hints"))
            .stdout(predicate::str::contains("list"))
            .stdout(predicate::str::contains("explain"))
            .stdout(predicate::str::contains("parse"));
    }

    #[test]
    fn test_hints_list() {
        aeterna().args(["hints", "list"]).assert().success().stdout(
            predicate::str::contains("Available Presets")
                .or(predicate::str::contains("minimal"))
                .or(predicate::str::contains("standard")),
        );
    }

    #[test]
    fn test_hints_list_json() {
        let output = aeterna()
            .args(["hints", "list", "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert!(json.is_array() || json.is_object());
    }

    #[test]
    fn test_hints_explain() {
        aeterna()
            .args(["hints", "explain", "minimal"])
            .assert()
            .success()
            .stdout(
                predicate::str::contains("Preset")
                    .or(predicate::str::contains("minimal"))
                    .or(predicate::str::contains("Hints")),
            );
    }

    #[test]
    fn test_hints_explain_json() {
        let output = aeterna()
            .args(["hints", "explain", "standard", "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert!(json.is_object());
    }

    #[test]
    fn test_hints_parse() {
        aeterna()
            .args(["hints", "parse", "fast,no-llm"])
            .assert()
            .success()
            .stdout(
                predicate::str::contains("Parsed")
                    .or(predicate::str::contains("fast"))
                    .or(predicate::str::contains("Hints")),
            );
    }

    #[test]
    fn test_hints_parse_json() {
        let output = aeterna()
            .args(["hints", "parse", "minimal,verbose", "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).expect("Valid JSON");
        assert!(json.is_object());
    }
}

mod completion_subcommand {
    use super::*;
    use predicates::prelude::PredicateBooleanExt;
    use predicates::prelude::predicate;

    #[test]
    fn test_completion_help() {
        aeterna()
            .args(["completion", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("completion"))
            .stdout(predicate::str::contains("shell"));
    }

    #[test]
    fn test_completion_bash() {
        aeterna()
            .args(["completion", "bash"])
            .assert()
            .success()
            .stdout(predicate::str::contains("complete").or(predicate::str::contains("_aeterna")));
    }

    #[test]
    fn test_completion_zsh() {
        aeterna()
            .args(["completion", "zsh"])
            .assert()
            .success()
            .stdout(predicate::str::contains("compdef").or(predicate::str::contains("_aeterna")));
    }

    #[test]
    fn test_completion_fish() {
        aeterna()
            .args(["completion", "fish"])
            .assert()
            .success()
            .stdout(predicate::str::contains("complete"));
    }
}

/// Task 1.3 — Tests for exact shipped entrypoint and migration commands.
///
/// These tests validate that:
/// - `aeterna serve` is a registered, reachable subcommand (not "unrecognized").
/// - `aeterna serve --help` exits 0 and shows expected flags.
/// - `aeterna admin migrate --help` exits 0 and shows expected flags.
/// - `aeterna admin migrate up` exits non-zero when PostgreSQL is unavailable
///   (production correctness: no silent stub success).
/// - The `--help` output for `serve` contains `--port` and `--bind` so operators
///   can verify the container entrypoint flags.
mod serve_and_migration_entrypoints {
    use super::*;
    use predicates::prelude::PredicateBooleanExt;
    use predicates::prelude::predicate;

    /// The container `CMD ["serve"]` must not fail with "unrecognized subcommand".
    /// It should exit with an error (because deps are unavailable in CI), but
    /// the error must come from our code, not from clap's unknown-command path.
    #[test]
    fn test_serve_is_a_registered_subcommand() {
        // `serve` without config/env will fail because PostgreSQL is not
        // available in CI; we only care that it is *not* an unrecognized subcommand.
        let output = aeterna().arg("serve").output().expect("process ran");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("unrecognized subcommand"),
            "'serve' must be a registered subcommand but got: {stderr}"
        );
        assert!(
            !stderr.contains("error: 'serve' is not a"),
            "'serve' must be a registered subcommand but got: {stderr}"
        );
    }

    #[test]
    fn test_serve_help_exits_success() {
        aeterna()
            .args(["serve", "--help"])
            .assert()
            .success()
            .stdout(
                predicate::str::contains("HTTP API server").or(predicate::str::contains("serve")),
            );
    }

    #[test]
    fn test_serve_help_shows_port_flag() {
        aeterna()
            .args(["serve", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--port"));
    }

    #[test]
    fn test_serve_help_shows_bind_flag() {
        aeterna()
            .args(["serve", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--bind"));
    }

    #[test]
    fn test_serve_help_shows_config_flag() {
        aeterna()
            .args(["serve", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--config").or(predicate::str::contains("config")));
    }

    /// The Helm migration job runs `aeterna admin migrate up`.
    /// Verify the command is reachable and `--help` exits 0.
    #[test]
    fn test_admin_migrate_help_exits_success() {
        aeterna()
            .args(["admin", "migrate", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("migration").or(predicate::str::contains("migrate")));
    }

    #[test]
    fn test_admin_migrate_up_help_shows_dry_run_flag() {
        aeterna()
            .args(["admin", "migrate", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--dry-run"));
    }

    #[test]
    fn test_admin_migrate_up_fails_clearly_when_database_is_unavailable() {
        let output = aeterna()
            .args(["admin", "migrate", "up"])
            .output()
            .expect("process ran");

        assert!(
            !output.status.success(),
            "admin migrate up without database must exit non-zero"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{stderr}{stdout}");

        assert!(
            combined.contains("Database Migration") || combined.contains("migration"),
            "Expected migration output, got: {combined}"
        );
        assert!(
            combined.contains("database not connected")
                || combined.contains("DATABASE_URL")
                || combined.contains("dry-run"),
            "Expected clear database-unavailable guidance, got: {combined}"
        );
    }

    /// `aeterna admin migrate status` should run cleanly.
    #[test]
    fn test_admin_migrate_status_runs() {
        aeterna()
            .args(["admin", "migrate", "status"])
            .assert()
            .success()
            .stdout(
                predicate::str::contains("Migration").or(predicate::str::contains("migration")),
            );
    }

    /// The serve command without a valid config directory must exit non-zero
    /// with a clear human-readable message (not a panic or "unrecognized subcommand").
    #[test]
    fn test_serve_without_config_exits_nonzero_with_clear_message() {
        let output = aeterna()
            .args(["serve", "--config", "/nonexistent/path/aeterna/config"])
            .output()
            .expect("process ran");
        assert!(
            !output.status.success(),
            "serve with missing config must exit non-zero"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{stderr}{stdout}");
        assert!(
            combined.contains("Configuration")
                || combined.contains("config")
                || combined.contains("not found"),
            "Expected a clear config-not-found message, got: {combined}"
        );
    }
}
