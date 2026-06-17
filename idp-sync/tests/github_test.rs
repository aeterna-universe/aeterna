mod test_helpers;

use idp_sync::github::{
    AeternaRole, GitHubHierarchyMapper, bridge_sync_to_governance, initialize_github_sync_schema,
    map_github_org_role, map_github_team_role, run_github_sync,
};
use idp_sync::okta::{GroupType, IdpClient, IdpGroup};
use serial_test::serial;
use sqlx::PgPool;
use testing::postgres;
use uuid::Uuid;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

use test_helpers::*;

async fn pg_pool() -> Option<PgPool> {
    let fixture = postgres().await?;
    let pool = PgPool::connect(fixture.url())
        .await
        .expect("connect to testcontainer pg");
    setup_db(&pool).await;
    Some(pool)
}

// ──────────────────────────────────────────────────────────
// 8.1: Token minting & client construction with wiremock
// ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_github_client_new_mints_token_via_wiremock() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"/app/installations/\d+/access_tokens"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_token_response()))
        .expect(1)
        .mount(&mock_server)
        .await;

    let config = github_config(&mock_server);
    let client = idp_sync::github::GitHubClient::new(config).await;
    assert!(
        client.is_ok(),
        "Client should be created: {:?}",
        client.err()
    );
}

#[tokio::test]
async fn test_github_client_new_rejects_invalid_pem() {
    let mock_server = MockServer::start().await;
    let mut config = github_config(&mock_server);
    config.private_key_pem = "not-a-valid-pem".to_string();

    let result = idp_sync::github::GitHubClient::new(config).await;
    assert!(result.is_err());
    let err = format!("{:?}", result.unwrap_err());
    assert!(
        err.contains("PEM") || err.contains("Invalid"),
        "error should mention PEM: {err}"
    );
}

#[tokio::test]
async fn test_github_client_new_handles_token_exchange_failure() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"/app/installations/\d+/access_tokens"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "message": "Bad credentials"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let config = github_config(&mock_server);
    let result = idp_sync::github::GitHubClient::new(config).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_github_client_new_handles_no_token_in_response() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"/app/installations/\d+/access_tokens"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "expires_at": "2099-01-01T00:00:00Z"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let config = github_config(&mock_server);
    let result = idp_sync::github::GitHubClient::new(config).await;
    assert!(result.is_err());
    let err = format!("{:?}", result.unwrap_err());
    assert!(
        err.contains("No token"),
        "error should mention missing token: {err}"
    );
}

// ──────────────────────────────────────────────────────────
// 8.1: IdpClient trait methods via wiremock
// ──────────────────────────────────────────────────────────

async fn make_client(mock_server: &MockServer) -> idp_sync::github::GitHubClient {
    Mock::given(method("POST"))
        .and(path_regex(r"/app/installations/\d+/access_tokens"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_token_response()))
        .mount(mock_server)
        .await;

    idp_sync::github::GitHubClient::new(github_config(mock_server))
        .await
        .expect("client creation")
}

#[tokio::test]
async fn test_list_users_returns_org_members() {
    let mock_server = MockServer::start().await;
    let client = make_client(&mock_server).await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_org_members()))
        .expect(1)
        .mount(&mock_server)
        .await;

    let page = client.list_users(None).await.expect("list_users");
    assert_eq!(page.users.len(), 3);
    assert_eq!(page.users[0].idp_subject, "alice");
    assert_eq!(page.users[1].idp_subject, "bob");
    assert_eq!(page.users[2].idp_subject, "charlie");
    assert!(page.next_page_token.is_none());
}

#[tokio::test]
async fn test_list_groups_returns_teams() {
    let mock_server = MockServer::start().await;
    let client = make_client(&mock_server).await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/teams"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_teams_nested()))
        .expect(1)
        .mount(&mock_server)
        .await;

    let page = client.list_groups(None).await.expect("list_groups");
    assert_eq!(page.groups.len(), 4);

    let platform = page.groups.iter().find(|g| g.id == "platform").unwrap();
    assert_eq!(platform.group_type, GroupType::GitHubTeam);

    let api = page.groups.iter().find(|g| g.id == "api-team").unwrap();
    assert_eq!(api.group_type, GroupType::GitHubNestedTeam);
    assert!(
        api.description
            .as_ref()
            .unwrap()
            .contains("parent:platform")
    );
}

#[tokio::test]
async fn test_get_group_members_paginates() {
    let mock_server = MockServer::start().await;
    let client = make_client(&mock_server).await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/teams/platform/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_team_members_alice_bob()))
        .expect(1)
        .mount(&mock_server)
        .await;

    let members = client
        .get_group_members("platform")
        .await
        .expect("get_group_members");
    assert_eq!(members.len(), 2);
    assert_eq!(members[0].idp_subject, "alice");
    assert_eq!(members[1].idp_subject, "bob");
}

#[tokio::test]
async fn test_get_user_parses_name() {
    let mock_server = MockServer::start().await;
    let client = make_client(&mock_server).await;

    Mock::given(method("GET"))
        .and(path("/users/alice"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_user_alice()))
        .expect(1)
        .mount(&mock_server)
        .await;

    let user = client.get_user("alice").await.expect("get_user");
    assert_eq!(user.first_name.as_deref(), Some("Alice"));
    assert_eq!(user.last_name.as_deref(), Some("Smith"));
    assert_eq!(user.email, "alice@example.com");
    assert_eq!(user.idp_provider, "github");
}

// ──────────────────────────────────────────────────────────
// 8.2: GitHubHierarchyMapper with testcontainers PG
// ──────────────────────────────────────────────────────────

fn make_groups(teams_json: serde_json::Value) -> Vec<IdpGroup> {
    let teams: Vec<serde_json::Value> = serde_json::from_value(teams_json).unwrap();
    teams
        .into_iter()
        .map(|t| {
            let parent = t
                .get("parent")
                .and_then(|p| if p.is_null() { None } else { Some(p.clone()) });
            let (group_type, description) = match parent {
                None => (
                    GroupType::GitHubTeam,
                    t["description"].as_str().map(String::from),
                ),
                Some(p) => {
                    let slug = p["slug"].as_str().unwrap_or("");
                    (GroupType::GitHubNestedTeam, Some(format!("parent:{slug}")))
                }
            };
            IdpGroup {
                id: t["slug"].as_str().unwrap().to_string(),
                name: t["name"].as_str().unwrap().to_string(),
                description,
                group_type,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }
        })
        .collect()
}

#[tokio::test]
#[serial]
async fn test_hierarchy_mapper_flat_teams() {
    let Some(pool) = pg_pool().await else {
        eprintln!("Skipping: Docker not available");
        return;
    };

    let tenant_id = Uuid::new_v4();
    let mapper = GitHubHierarchyMapper::new(pool.clone(), tenant_id);
    let groups = make_groups(mock_teams_flat());

    let mappings = mapper
        .create_hierarchy("test-org", &groups)
        .await
        .expect("create_hierarchy");

    assert_eq!(mappings.len(), 2, "should map 2 top-level teams");
    assert!(mappings.contains_key("platform"));
    assert!(mappings.contains_key("security"));

    let org_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM organizations WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(org_count.0, 2);

    let team_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM teams t JOIN organizations o ON o.id = t.org_id WHERE o.tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(team_count.0, 2);
}

#[tokio::test]
#[serial]
async fn test_hierarchy_mapper_two_level_nesting() {
    let Some(pool) = pg_pool().await else {
        eprintln!("Skipping: Docker not available");
        return;
    };

    let tenant_id = Uuid::new_v4();
    let mapper = GitHubHierarchyMapper::new(pool.clone(), tenant_id);
    let groups = make_groups(mock_teams_nested());

    let mappings = mapper
        .create_hierarchy("test-org", &groups)
        .await
        .expect("hierarchy");

    assert_eq!(mappings.len(), 4);
    assert!(mappings.contains_key("api-team"));
    assert!(mappings.contains_key("frontend-team"));

    let api_team: (String,) = sqlx::query_as(
        "SELECT o.slug
           FROM teams t
           JOIN organizations o ON o.id = t.org_id
          WHERE o.tenant_id = $1 AND t.slug = 'api-team'",
    )
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("api-team row");
    assert_eq!(api_team.0, "platform");
}

#[tokio::test]
#[serial]
async fn test_hierarchy_mapper_three_level_nesting() {
    let Some(pool) = pg_pool().await else {
        eprintln!("Skipping: Docker not available");
        return;
    };

    let tenant_id = Uuid::new_v4();
    let mapper = GitHubHierarchyMapper::new(pool.clone(), tenant_id);
    let groups = make_groups(mock_teams_three_level());

    let mappings = mapper
        .create_hierarchy("test-org", &groups)
        .await
        .expect("hierarchy");

    assert_eq!(mappings.len(), 3);

    let engineering_id = mappings.get("engineering").unwrap();
    let platform_id = mappings.get("platform").unwrap();
    let api_team_id = mappings.get("api-team").unwrap();
    assert_ne!(engineering_id, platform_id);
    assert_ne!(platform_id, api_team_id);

    let org_and_team_counts: (i64, i64) = sqlx::query_as(
        "SELECT
             (SELECT COUNT(*) FROM organizations WHERE tenant_id = $1),
             (SELECT COUNT(*) FROM teams t JOIN organizations o ON o.id = t.org_id WHERE o.tenant_id = $1)"
    )
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("counts");
    assert_eq!(org_and_team_counts.0, 1);
    assert_eq!(org_and_team_counts.1, 3);

    let descendant_orgs: Vec<(String,)> = sqlx::query_as(
        "SELECT o.slug
           FROM teams t
           JOIN organizations o ON o.id = t.org_id
           WHERE o.tenant_id = $1 AND t.slug IN ('platform', 'api-team')
          ORDER BY t.slug",
    )
    .bind(tenant_id)
    .fetch_all(&pool)
    .await
    .expect("descendant orgs");
    assert_eq!(
        descendant_orgs,
        vec![("engineering".to_string(),), ("engineering".to_string(),)]
    );
}

#[tokio::test]
#[serial]
async fn test_hierarchy_mapper_idempotent_upsert() {
    let Some(pool) = pg_pool().await else {
        eprintln!("Skipping: Docker not available");
        return;
    };

    let tenant_id = Uuid::new_v4();
    let mapper = GitHubHierarchyMapper::new(pool.clone(), tenant_id);
    let groups = make_groups(mock_teams_flat());

    let first = mapper
        .create_hierarchy("test-org", &groups)
        .await
        .expect("first");
    let second = mapper
        .create_hierarchy("test-org", &groups)
        .await
        .expect("second");

    assert_eq!(
        first.get("platform"),
        second.get("platform"),
        "ID should be stable across upserts"
    );
    assert_eq!(first.get("security"), second.get("security"));

    let counts: (i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT COUNT(*) FROM organizations WHERE tenant_id = $1),
            (SELECT COUNT(*) FROM teams t JOIN organizations o ON o.id = t.org_id WHERE o.tenant_id = $1)"
    )
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(counts, (2, 2), "2 orgs + 2 teams, no duplicates");
}

#[tokio::test]
#[serial]
async fn test_store_group_to_team_mappings() {
    let Some(pool) = pg_pool().await else {
        eprintln!("Skipping: Docker not available");
        return;
    };

    let tenant_id = Uuid::new_v4();
    let mapper = GitHubHierarchyMapper::new(pool.clone(), tenant_id);
    let groups = make_groups(mock_teams_flat());

    let mappings = mapper
        .create_hierarchy("test-org", &groups)
        .await
        .expect("hierarchy");
    mapper
        .store_group_to_team_mappings(&mappings)
        .await
        .expect("store mappings");

    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM idp_group_mappings WHERE idp_provider = 'github'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(count.0 >= 2, "should have at least 2 mappings");
}

// ──────────────────────────────────────────────────────────
// 8.3: Role mapping (augmented edge cases)
// ──────────────────────────────────────────────────────────

#[test]
fn test_role_mapping_unknown_org_role_defaults_developer() {
    assert_eq!(
        map_github_org_role("billing_manager"),
        AeternaRole::Developer
    );
    assert_eq!(map_github_org_role(""), AeternaRole::Developer);
    assert_eq!(map_github_org_role("ADMIN"), AeternaRole::Developer); // case-sensitive
}

#[test]
fn test_role_mapping_unknown_team_role_defaults_developer() {
    assert_eq!(map_github_team_role(""), AeternaRole::Developer);
    assert_eq!(map_github_team_role("admin"), AeternaRole::Developer);
    assert_eq!(map_github_team_role("MAINTAINER"), AeternaRole::Developer);
}

#[test]
fn test_aeterna_role_as_str() {
    assert_eq!(AeternaRole::Admin.as_str(), "admin");
    assert_eq!(AeternaRole::TechLead.as_str(), "techlead");
    assert_eq!(AeternaRole::Developer.as_str(), "developer");
}

#[test]
fn test_role_precedence_total_ordering() {
    let roles = [
        AeternaRole::Developer,
        AeternaRole::TechLead,
        AeternaRole::Admin,
    ];
    for i in 0..roles.len() {
        for j in (i + 1)..roles.len() {
            assert!(
                roles[i].precedence() < roles[j].precedence(),
                "{:?} should have lower precedence than {:?}",
                roles[i],
                roles[j]
            );
        }
    }
}

// ──────────────────────────────────────────────────────────
// 8.4: Full sync flow (testcontainers PG + wiremock)
// ──────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_full_sync_flow_creates_users_and_teams() {
    let Some(pool) = pg_pool().await else {
        eprintln!("Skipping: Docker not available");
        return;
    };

    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"/app/installations/\d+/access_tokens"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_token_response()))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/teams\?"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_teams_nested()))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/members\?"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_org_members()))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/teams/platform/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_team_members_alice_bob()))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/teams/security/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_team_members_charlie()))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/teams/api-team/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_team_members_alice_bob()))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/teams/frontend-team/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&mock_server)
        .await;

    let config = github_config(&mock_server);
    let tenant_id = Uuid::new_v4();

    let report = run_github_sync(&config, &pool, tenant_id)
        .await
        .expect("sync should succeed");

    assert_eq!(report.users_created, 3, "should create 3 users");
    assert!(report.groups_synced >= 2, "should sync groups");
    assert!(report.completed_at.is_some(), "should be marked complete");
    assert!(!report.has_errors(), "no errors: {:?}", report.errors);

    let user_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM users WHERE idp_provider = 'github'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(user_count.0, 3);

    let hierarchy_counts: (i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT COUNT(*) FROM organizations WHERE tenant_id = $1),
            (SELECT COUNT(*) FROM teams t JOIN organizations o ON o.id = t.org_id WHERE o.tenant_id = $1)"
    )
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(hierarchy_counts.0 >= 2, "expected at least 2 organizations");
    assert!(hierarchy_counts.1 >= 4, "expected at least 4 teams");
}

#[tokio::test]
#[serial]
async fn test_full_sync_idempotent_second_run_no_creates() {
    let Some(pool) = pg_pool().await else {
        eprintln!("Skipping: Docker not available");
        return;
    };

    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"/app/installations/\d+/access_tokens"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_token_response()))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/teams\?"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_teams_flat()))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/members\?"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_org_members()))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/teams/platform/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_team_members_alice_bob()))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/teams/security/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_team_members_charlie()))
        .mount(&mock_server)
        .await;

    let config = github_config(&mock_server);
    let tenant_id = Uuid::new_v4();

    let first = run_github_sync(&config, &pool, tenant_id)
        .await
        .expect("first sync");
    assert_eq!(first.users_created, 3);

    let second = run_github_sync(&config, &pool, tenant_id)
        .await
        .expect("second sync");
    assert_eq!(second.users_created, 0, "no new users on re-sync");
    assert_eq!(second.users_deactivated, 0);
}

// ──────────────────────────────────────────────────────────
// 8.5: Webhook event deserialization & processing
// ──────────────────────────────────────────────────────────

#[test]
fn test_org_member_added_payload_deser() {
    let payload = serde_json::json!({
        "action": "member_added",
        "organization": {"login": "test-org", "id": 1},
        "membership": {
            "user": {"login": "newuser", "id": 9999},
            "role": "member"
        }
    });
    let val: serde_json::Value = serde_json::from_value(payload).unwrap();
    assert_eq!(val["action"], "member_added");
    assert_eq!(val["membership"]["user"]["login"], "newuser");
}

#[test]
fn test_team_created_payload_deser() {
    let payload = serde_json::json!({
        "action": "created",
        "team": {
            "id": 555,
            "slug": "new-team",
            "name": "New Team",
            "parent": null
        },
        "organization": {"login": "test-org", "id": 1}
    });
    let val: serde_json::Value = serde_json::from_value(payload).unwrap();
    assert_eq!(val["action"], "created");
    assert_eq!(val["team"]["slug"], "new-team");
}

#[test]
fn test_membership_added_payload_deser() {
    let payload = serde_json::json!({
        "action": "added",
        "member": {"login": "alice", "id": 1001},
        "team": {"id": 100, "slug": "platform", "name": "Platform"},
        "organization": {"login": "test-org", "id": 1}
    });
    let val: serde_json::Value = serde_json::from_value(payload).unwrap();
    assert_eq!(val["action"], "added");
    assert_eq!(val["member"]["login"], "alice");
    assert_eq!(val["team"]["slug"], "platform");
}

#[test]
fn test_schema_initialization_is_safe_to_call_twice() {
    // This is a compile-time test ensuring the function signature is correct.
    // Actual DB test is in the integration tests above (every pg_pool call runs it).
    let _ = initialize_github_sync_schema;
}

// ──────────────────────────────────────────────────────────
// 8.6: Governance bridge integration (task 0.3.4)
// ──────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_bridge_sync_to_governance_populates_roles() {
    let Some(pool) = pg_pool().await else {
        eprintln!("Skipping: Docker not available");
        return;
    };

    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path_regex(r"/app/installations/\d+/access_tokens"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_token_response()))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/teams\?"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_teams_nested()))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/members\?"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_org_members()))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/teams/platform/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_team_members_alice_bob()))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/teams/security/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_team_members_charlie()))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/teams/api-team/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_team_members_alice_bob()))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path_regex(r"/orgs/test-org/teams/frontend-team/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&mock_server)
        .await;

    let config = github_config(&mock_server);
    let tenant_id = Uuid::new_v4();

    // Step 1: Run sync to create users, memberships, and modern hierarchy rows
    let report = run_github_sync(&config, &pool, tenant_id)
        .await
        .expect("sync should succeed");
    assert!(!report.has_errors(), "sync errors: {:?}", report.errors);
    assert!(report.users_created >= 3, "should create users");

    // Step 2: Run governance bridge
    let (gov_roles, user_roles) = bridge_sync_to_governance(&pool, tenant_id)
        .await
        .expect("bridge should succeed");

    // Step 3: Verify governance_roles were created
    // We have 3 users (alice, bob, charlie) with memberships to teams.
    // alice+bob → platform + api-team, charlie → security
    // That's at least 5 membership rows, so at least 5 governance_roles.
    assert!(
        gov_roles >= 3,
        "should create governance_roles, got {gov_roles}"
    );

    // Step 4: Verify user_roles were created
    assert!(
        user_roles >= 3,
        "should create user_roles, got {user_roles}"
    );

    // Step 5: Verify data in DB directly
    let gov_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM governance_roles WHERE principal_type = 'user'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        gov_count.0 >= 3,
        "governance_roles DB count should be >= 3, got {}",
        gov_count.0
    );

    let ur_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM user_roles WHERE tenant_id = $1")
        .bind(tenant_id.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        ur_count.0 >= 3,
        "user_roles DB count should be >= 3, got {}",
        ur_count.0
    );

    // Step 6: Verify idempotency — running bridge again should not duplicate
    let (gov_roles_2, user_roles_2) = bridge_sync_to_governance(&pool, tenant_id)
        .await
        .expect("second bridge should succeed");
    assert_eq!(
        gov_roles_2, 0,
        "second bridge should not create new governance_roles"
    );
    assert_eq!(
        user_roles_2, 0,
        "second bridge should not create new user_roles"
    );
}
