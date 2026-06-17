use std::collections::HashMap;
use std::sync::Arc;

use aeterna::server::plugin_auth::{
    PluginTokenClaims, RefreshTokenStore, RefreshTokenStoreBackend,
};
use aeterna::server::{AppState, PluginAuthState, health, metrics, router};
use agent_a2a::config::TrustedIdentityConfig;
use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode, header::AUTHORIZATION};
use jsonwebtoken::{EncodingKey, Header, encode};
use knowledge::api::GovernanceDashboardApi;
use knowledge::governance::GovernanceEngine;
use knowledge::manager::KnowledgeManager;
use knowledge::repository::{GitRepository, RepositoryError};
use knowledge::tenant_repo_resolver::TenantRepositoryResolver;
use memory::manager::MemoryManager;
use memory::reasoning::ReflectiveReasoner;
use mk_core::traits::{AuthorizationService, KnowledgeRepository, StorageBackend};
use mk_core::types::{
    KnowledgeEntry, KnowledgeLayer, KnowledgeStatus, KnowledgeType, ReasoningStrategy,
    ReasoningTrace, Role, RoleIdentifier, TenantContext, TenantId, UserId,
};
use serde_json::json;
use storage::governance::GovernanceStorage;
use storage::postgres::PostgresBackend;
use storage::secret_provider::LocalSecretProvider;
use storage::tenant_config_provider::KubernetesTenantConfigProvider;
use storage::tenant_store::{TenantRepositoryBindingStore, TenantStore};
use sync::bridge::SyncManager;
use sync::state_persister::FilePersister;
use sync::websocket::{AuthToken, TokenValidator, WsResult, WsServer};
use tempfile::TempDir;
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
}

impl MockRepo {
    fn new() -> Self {
        Self {
            items: tokio::sync::RwLock::new(HashMap::new()),
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
            .filter(|((entry_layer, path), _)| *entry_layer == layer && path.starts_with(prefix))
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

    async fn get_head_commit(&self, _ctx: TenantContext) -> Result<Option<String>, Self::Error> {
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

    fn root_path(&self) -> Option<std::path::PathBuf> {
        None
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

async fn test_app_state_with_plugin_auth(
    plugin_auth_config: config::PluginAuthConfig,
) -> Option<(Arc<AppState>, TempDir)> {
    let tempdir = tempfile::tempdir().unwrap();
    let repo = Arc::new(MockRepo::new());
    repo.store(
        TenantContext::new(
            mk_core::types::TenantId::new("default".to_string()).unwrap(),
            mk_core::types::UserId::new("system".to_string()).unwrap(),
        ),
        sample_entry("specs/example.md"),
        "seed",
    )
    .await
    .unwrap();

    let fixture = postgres().await?;
    let postgres = Arc::new(PostgresBackend::new(fixture.url()).await.ok()?);
    postgres.initialize_schema().await.ok()?;
    let governance_engine = Arc::new(GovernanceEngine::new());
    let git_repo = Arc::new(GitRepository::new(tempdir.path()).unwrap());
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
        .unwrap(),
    );
    let auth_service: Arc<dyn AuthorizationService<Error = anyhow::Error> + Send + Sync> =
        Arc::new(MockAuth);
    let dashboard = Arc::new(GovernanceDashboardApi::new(
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
    let git_provider_connection_registry =
        Arc::new(storage::git_provider_connection_store::InMemoryGitProviderConnectionStore::new());
    let tenant_repo_resolver = Arc::new(
        TenantRepositoryResolver::new(
            tenant_repository_binding_store.clone(),
            std::env::temp_dir(),
            Arc::new(LocalSecretProvider::new(HashMap::new())),
        )
        .with_connection_registry(git_provider_connection_registry.clone()),
    );

    Some((
        Arc::new(AppState {
            config: Arc::new(config::Config::default()),
            revocation_cache: Default::default(),
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
            governance_storage: Some(Arc::new(GovernanceStorage::new(postgres.pool().clone()))),
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
                config: plugin_auth_config,
                postgres: Some(postgres.clone()),
                refresh_store: RefreshTokenStoreBackend::InMemory(RefreshTokenStore::new()),
            }),
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
                aeterna::server::tenant_runtime_state::TenantRuntimeRegistry::new(),
            ),
            bootstrap_tracker: std::sync::Arc::new(
                aeterna::server::bootstrap_tracker::BootstrapTracker::new(),
            ),
        }),
        tempdir,
    ))
}

async fn test_app_state() -> Option<(Arc<AppState>, TempDir)> {
    test_app_state_with_plugin_auth(config::PluginAuthConfig::default()).await
}

async fn seed_company_unit(state: &Arc<AppState>, tenant_id: &TenantId) -> String {
    let unit_id = uuid::Uuid::new_v4().to_string();
    state
        .postgres
        .create_unit(&mk_core::types::OrganizationalUnit {
            id: unit_id.clone(),
            name: "Acme".to_string(),
            unit_type: mk_core::types::UnitType::Organization,
            parent_id: None,
            tenant_id: tenant_id.clone(),
            metadata: HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            source_owner: mk_core::types::RecordSource::Admin,
        })
        .await
        .unwrap();
    unit_id
}

/// #44.d §4.1 — seed an Organization unit in the given tenant.
/// Used by the cross-tenant envelope contract tests below.
async fn seed_unit_of_type(
    state: &Arc<AppState>,
    tenant_id: &TenantId,
    unit_type: mk_core::types::UnitType,
    name: &str,
) -> String {
    let unit_id = uuid::Uuid::new_v4().to_string();
    state
        .postgres
        .create_unit(&mk_core::types::OrganizationalUnit {
            id: unit_id.clone(),
            name: name.to_string(),
            unit_type,
            parent_id: None,
            tenant_id: tenant_id.clone(),
            metadata: HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            source_owner: mk_core::types::RecordSource::Admin,
        })
        .await
        .unwrap();
    unit_id
}

fn mint_test_plugin_bearer(secret: &str, tenant_id: &str, github_login: &str) -> String {
    let now = chrono::Utc::now().timestamp();
    encode(
        &Header::new(jsonwebtoken::Algorithm::HS256),
        &PluginTokenClaims {
            sub: github_login.to_string(),
            idp_provider: "github".to_string(),
            tenant_id: tenant_id.to_string(),
            iss: "aeterna-test".to_string(),
            aud: vec![PluginTokenClaims::AUDIENCE.to_string()],
            iat: now,
            exp: now + 3600,
            jti: "test-jti".to_string(),
            github_id: 42,
            email: Some(format!("{github_login}@example.com")),
            kind: PluginTokenClaims::KIND.to_string(),
            token_type: PluginTokenClaims::TOKEN_TYPE_USER.to_string(),
            scopes: Vec::new(),
        },
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap()
}

#[tokio::test]
async fn server_health_route_returns_ok() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn server_metrics_route_returns_prometheus_content_type() {
    let handle = metrics::create_recorder();
    let app = metrics::router(handle);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("text/plain")
    );
}

#[tokio::test]
async fn server_ready_route_returns_503_with_unavailable_backend() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = health::router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn server_mcp_sse_route_returns_ok() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/mcp/sse")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn server_mcp_message_route_returns_json_rpc_response() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let expected_tool_names: std::collections::HashSet<String> = state
        .mcp_server
        .list_tools()
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    let app = router::build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp/message")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "tools/list"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["jsonrpc"], "2.0");
    assert_eq!(json["id"], 1);
    let tool_names: std::collections::HashSet<String> = json["result"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(ToOwned::to_owned))
        .collect();
    assert_eq!(tool_names, expected_tool_names);
    assert!(tool_names.contains("knowledge_list"));
    assert!(tool_names.contains("sync_status"));
}

#[tokio::test]
async fn server_mcp_message_route_can_call_registered_tool() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp/message")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "tools/call",
                        "params": {
                            "name": "knowledge_list",
                            "tenantContext": {
                                "tenantId": "default",
                                "userId": "system"
                            },
                            "arguments": {
                                "layer": "project",
                                "prefix": "specs/"
                            }
                        }
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["jsonrpc"], "2.0");
    assert_eq!(json["id"], 2);
    assert_eq!(json["result"]["success"], true);
    let entries = json["result"]["entries"].as_array().unwrap();
    assert!(!entries.is_empty());
    assert_eq!(entries[0]["path"], "specs/example.md");
}

#[tokio::test]
async fn server_a2a_route_is_mounted() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/a2a/.well-known/agent.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_ne!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn shutdown_channel_propagates_true_signal() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let mut rx = state.shutdown_tx.subscribe();
    state.shutdown_tx.send(true).unwrap();
    rx.changed().await.unwrap();
    assert!(*rx.borrow());
}

#[tokio::test]
async fn sync_push_stores_entries() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    let push_body = json!({
        "entries": [
            {
                "id": "test-mem-1",
                "content": "Test memory content",
                "layer": "project",
                "tags": ["test"],
                "metadata": null,
                "importance": 0.5,
                "created_at": "2025-01-15T10:30:00Z",
                "updated_at": "2025-01-15T10:30:00Z",
                "deleted_at": null
            }
        ],
        "device_id": "test-device"
    });

    let push_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sync/push")
                .header("content-type", "application/json")
                .header("authorization", "Bearer test-token")
                .body(Body::from(serde_json::to_vec(&push_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(push_response.status(), StatusCode::OK);
    let push_bytes = axum::body::to_bytes(push_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let push_json: serde_json::Value = serde_json::from_slice(&push_bytes).unwrap();
    assert!(!push_json["cursor"].as_str().unwrap().is_empty());
    assert!(push_json["conflicts"].as_array().unwrap().is_empty());

    let pull_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/sync/pull?since_cursor=0&layers=project")
                .header("authorization", "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(pull_response.status(), StatusCode::OK);
    let pull_bytes = axum::body::to_bytes(pull_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let pull_json: serde_json::Value = serde_json::from_slice(&pull_bytes).unwrap();
    let entries = pull_json["entries"].as_array().unwrap();
    assert!(!entries.is_empty());
    assert!(entries.iter().any(|entry| {
        entry["id"] == "test-mem-1"
            && entry["content"] == "Test memory content"
            && entry["layer"] == "project"
    }));
}

#[tokio::test]
async fn sync_pull_returns_seeded_data() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    let push_body = json!({
        "entries": [
            {
                "id": "test-mem-project",
                "content": "Project memory",
                "layer": "project",
                "tags": ["project"],
                "metadata": null,
                "importance": 0.7,
                "created_at": "2025-01-15T10:30:00Z",
                "updated_at": "2025-01-15T10:30:00Z",
                "deleted_at": null
            },
            {
                "id": "test-mem-team",
                "content": "Team memory",
                "layer": "team",
                "tags": ["team"],
                "metadata": null,
                "importance": 0.6,
                "created_at": "2025-01-15T10:31:00Z",
                "updated_at": "2025-01-15T10:31:00Z",
                "deleted_at": null
            }
        ],
        "device_id": "test-device"
    });

    let push_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sync/push")
                .header("content-type", "application/json")
                .header("authorization", "Bearer test-token")
                .body(Body::from(serde_json::to_vec(&push_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(push_response.status(), StatusCode::OK);

    let pull_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/sync/pull?since_cursor=0&layers=project")
                .header("authorization", "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(pull_response.status(), StatusCode::OK);
    let pull_bytes = axum::body::to_bytes(pull_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let pull_json: serde_json::Value = serde_json::from_slice(&pull_bytes).unwrap();

    let entries = pull_json["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], "test-mem-project");
    assert_eq!(entries[0]["layer"], "project");
    assert!(entries.iter().all(|entry| entry["layer"] == "project"));

    let cursor = pull_json["cursor"].as_str().unwrap();
    assert!(!cursor.is_empty());
    assert_ne!(cursor, "0");
}

#[tokio::test]
async fn sync_endpoints_reject_unauthenticated() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    let push_body = json!({
        "entries": [
            {
                "id": "test-mem-unauth",
                "content": "Test memory content",
                "layer": "project",
                "tags": ["test"],
                "metadata": null,
                "importance": 0.5,
                "created_at": "2025-01-15T10:30:00Z",
                "updated_at": "2025-01-15T10:30:00Z",
                "deleted_at": null
            }
        ],
        "device_id": "test-device"
    });

    let push_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sync/push")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&push_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(push_response.status(), StatusCode::UNAUTHORIZED);
    let push_bytes = axum::body::to_bytes(push_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let push_json: serde_json::Value = serde_json::from_slice(&push_bytes).unwrap();
    assert_eq!(
        push_json,
        json!({
            "error": "auth_required",
            "message": "Bearer token required"
        })
    );

    let pull_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/sync/pull?since_cursor=0&layers=project")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(pull_response.status(), StatusCode::UNAUTHORIZED);
    let pull_bytes = axum::body::to_bytes(pull_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let pull_json: serde_json::Value = serde_json::from_slice(&pull_bytes).unwrap();
    assert_eq!(
        pull_json,
        json!({
            "error": "auth_required",
            "message": "Bearer token required"
        })
    );
}

#[tokio::test]
async fn session_start_rejects_invalid_plugin_bearer_when_plugin_auth_enabled() {
    let secret = "super-secret-test-key-at-least-32-chars";
    let Some((state, _tmp)) = test_app_state_with_plugin_auth(config::PluginAuthConfig {
        enabled: true,
        jwt_secret: Some(secret.to_string()),
        ..Default::default()
    })
    .await
    else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sessions")
                .header("content-type", "application/json")
                .header(AUTHORIZATION, "Bearer not-a-jwt")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "project": "auth-test",
                        "directory": "/tmp/auth-test"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "invalid_plugin_token");
}

#[tokio::test]
async fn session_start_accepts_valid_plugin_bearer_and_returns_github_login() {
    let secret = "super-secret-test-key-at-least-32-chars";
    let Some((state, _tmp)) = test_app_state_with_plugin_auth(config::PluginAuthConfig {
        enabled: true,
        jwt_secret: Some(secret.to_string()),
        ..Default::default()
    })
    .await
    else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);
    let token = mint_test_plugin_bearer(secret, "tenant-7", "octocat");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sessions")
                .header("content-type", "application/json")
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "project": "auth-test",
                        "directory": "/tmp/auth-test"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["userId"], "octocat");
    assert_eq!(json["project"], "auth-test");
    assert!(!json["sessionId"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn sync_push_rejects_invalid_plugin_bearer_when_plugin_auth_enabled() {
    let secret = "super-secret-test-key-at-least-32-chars";
    let Some((state, _tmp)) = test_app_state_with_plugin_auth(config::PluginAuthConfig {
        enabled: true,
        jwt_secret: Some(secret.to_string()),
        ..Default::default()
    })
    .await
    else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sync/push")
                .header("content-type", "application/json")
                .header(AUTHORIZATION, "Bearer not-a-jwt")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "entries": [],
                        "device_id": "test-device"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "invalid_plugin_token");
}

#[tokio::test]
async fn plugin_auth_bootstrap_returns_service_unavailable_when_disabled() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/plugin/bootstrap")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "provider": "github",
                        "github_access_token": "gho_test"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn plugin_auth_refresh_rejects_invalid_refresh_token() {
    let secret = "super-secret-test-key-at-least-32-chars";
    let Some((state, _tmp)) = test_app_state_with_plugin_auth(config::PluginAuthConfig {
        enabled: true,
        jwt_secret: Some(secret.to_string()),
        ..Default::default()
    })
    .await
    else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/plugin/refresh")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "refresh_token": "missing-refresh-token"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "invalid_refresh_token");
}

#[tokio::test]
async fn tenant_admin_route_group_is_mounted_for_org_team_user_and_govern() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    for path in [
        "/api/v1/org",
        "/api/v1/team",
        "/api/v1/user/test-user/roles",
        "/api/v1/govern/roles",
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_ne!(
            response.status(),
            StatusCode::NOT_FOUND,
            "route should be mounted: {path}"
        );
    }
}

#[tokio::test]
async fn user_role_revoke_fails_closed_when_assignment_scope_is_ambiguous() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let tenant_id = TenantId::new("default".to_string()).unwrap();
    let user_id = UserId::new("test-user".to_string()).unwrap();

    let company_unit_id = "11111111-1111-1111-1111-111111111111".to_string();
    let org_unit_id = "22222222-2222-2222-2222-222222222222".to_string();
    let team_unit_id = "33333333-3333-3333-3333-333333333333".to_string();

    state
        .postgres
        .create_unit(&mk_core::types::OrganizationalUnit {
            id: company_unit_id.clone(),
            name: "Acme".to_string(),
            unit_type: mk_core::types::UnitType::Organization,
            parent_id: None,
            tenant_id: tenant_id.clone(),
            metadata: HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            source_owner: mk_core::types::RecordSource::Admin,
        })
        .await
        .unwrap();
    state
        .postgres
        .create_unit(&mk_core::types::OrganizationalUnit {
            id: org_unit_id.clone(),
            name: "Platform".to_string(),
            unit_type: mk_core::types::UnitType::Organization,
            parent_id: Some(company_unit_id.clone()),
            tenant_id: tenant_id.clone(),
            metadata: HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            source_owner: mk_core::types::RecordSource::Admin,
        })
        .await
        .unwrap();
    state
        .postgres
        .create_unit(&mk_core::types::OrganizationalUnit {
            id: team_unit_id.clone(),
            name: "API".to_string(),
            unit_type: mk_core::types::UnitType::Team,
            parent_id: Some(org_unit_id.clone()),
            tenant_id: tenant_id.clone(),
            metadata: HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            source_owner: mk_core::types::RecordSource::Admin,
        })
        .await
        .unwrap();
    state
        .postgres
        .assign_role(&user_id, &tenant_id, &org_unit_id, Role::Developer.into())
        .await
        .unwrap();
    state
        .postgres
        .assign_role(&user_id, &tenant_id, &team_unit_id, Role::Developer.into())
        .await
        .unwrap();

    let secret = "super-secret-test-key-at-least-32-chars";
    let token = mint_test_plugin_bearer(secret, "default", "admin-user");
    let mut state = (*state).clone();
    state.plugin_auth_state = Arc::new(PluginAuthState {
        config: config::PluginAuthConfig {
            enabled: true,
            jwt_secret: Some(secret.to_string()),
            ..Default::default()
        },
        postgres: Some(state.postgres.clone()),
        refresh_store: RefreshTokenStoreBackend::InMemory(RefreshTokenStore::new()),
    });
    let app = router::build_router(Arc::new(state));

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/user/test-user/roles/developer")
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "ambiguous_role_assignment");
    assert_eq!(json["assignments"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn tenant_admin_hierarchy_role_crud_via_rest_api() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/admin/hierarchy")
                .header("content-type", "application/json")
                .header("x-user-id", "tenant-admin")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": "Platform",
                        "unit_type": "organization",
                        "parent_id": null,
                        "metadata": {},
                        "source_owner": "admin"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_response.status(), StatusCode::CREATED);
    let create_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let unit_id = create_json["unit"]["id"].as_str().unwrap().to_string();
    assert_eq!(create_json["unit"]["unitType"], "organization");

    let assign_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/admin/hierarchy/{unit_id}/members"))
                .header("content-type", "application/json")
                .header("x-user-id", "tenant-admin")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "user_id": "tenant-member",
                        "role": "Developer"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(assign_response.status(), StatusCode::CREATED);
    let assign_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(assign_response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(assign_json["membership"]["userId"], "tenant-member");
    assert_eq!(assign_json["membership"]["role"], "developer");

    let list_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/admin/hierarchy/{unit_id}/members"))
                .header("x-user-id", "tenant-admin")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(list_response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        list_json["members"]
            .as_array()
            .unwrap()
            .iter()
            .any(|member| {
                member["user_id"] == "tenant-member" && member["role"] == "developer"
            })
    );

    let revoke_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/api/v1/admin/hierarchy/{unit_id}/members/tenant-member/roles/developer"
                ))
                .header("x-user-id", "tenant-admin")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoke_response.status(), StatusCode::OK);
    let revoke_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(revoke_response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(revoke_json["success"], true);

    let list_after_revoke = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/admin/hierarchy/{unit_id}/members"))
                .header("x-user-id", "tenant-admin")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list_after_revoke.status(), StatusCode::OK);
    let list_after_revoke_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(list_after_revoke.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        !list_after_revoke_json["members"]
            .as_array()
            .unwrap()
            .iter()
            .any(|member| member["user_id"] == "tenant-member")
    );
}

#[tokio::test]
async fn tenant_admin_hierarchy_rejects_platform_admin_role_assignment() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/admin/hierarchy")
                .header("content-type", "application/json")
                .header("x-user-id", "tenant-admin")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": "Security",
                        "unit_type": "organization",
                        "parent_id": null,
                        "metadata": {},
                        "source_owner": "admin"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_response.status(), StatusCode::CREATED);
    let create_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let unit_id = create_json["unit"]["id"].as_str().unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/admin/hierarchy/{unit_id}/members"))
                .header("content-type", "application/json")
                .header("x-user-id", "tenant-admin")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "user_id": "tenant-member",
                        "role": "PlatformAdmin"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(json["error"], "invalid_role_assignment");
}

#[tokio::test]
async fn govern_roles_assign_and_revoke_via_rest_api() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let tenant_id = TenantId::new("default".to_string()).unwrap();
    let _tenant_id = seed_company_unit(&state, &tenant_id).await;
    let app = router::build_router(state);
    let principal = "11111111-2222-3333-4444-555555555555";

    let assign_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/govern/roles")
                .header("content-type", "application/json")
                .header("x-user-id", "tenant-admin")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "principal": principal,
                        "principalType": "user",
                        "role": "Architect",
                        "scope": "tenant"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(assign_response.status(), StatusCode::OK);
    let assign_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(assign_response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(assign_json["principal"], principal);
    assert_eq!(assign_json["principalType"], "user");
    assert_eq!(assign_json["role"], "architect");
    assert_eq!(assign_json["scope"], "tenant");

    let list_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/govern/roles")
                .header("x-user-id", "tenant-admin")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(list_response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(list_json.as_array().unwrap().iter().any(|entry| {
        entry["principal"] == principal
            && entry["role"] == "architect"
            && entry["scope"] == "tenant"
    }));

    let revoke_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/govern/roles/{principal}/architect"))
                .header("x-user-id", "tenant-admin")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoke_response.status(), StatusCode::OK);
    let revoke_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(revoke_response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(revoke_json["success"], true);
    assert_eq!(revoke_json["principal"], principal);
    assert_eq!(revoke_json["role"], "architect");

    let list_after_revoke = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/govern/roles")
                .header("x-user-id", "tenant-admin")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list_after_revoke.status(), StatusCode::OK);
    let list_after_revoke_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(list_after_revoke.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        !list_after_revoke_json
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["principal"] == principal && entry["role"] == "architect" })
    );
}

#[tokio::test]
async fn tenant_lifecycle_crud_via_rest_api() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/admin/tenants")
                .header("content-type", "application/json")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "slug": "acme-runtime",
                        "name": "Acme Runtime"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);
    let create_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(create.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(create_json["tenant"]["slug"], "acme-runtime");

    let show = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/admin/tenants/acme-runtime")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(show.status(), StatusCode::OK);

    let update = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/v1/admin/tenants/acme-runtime")
                .header("content-type", "application/json")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": "Acme Runtime Updated"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(update.status(), StatusCode::OK);
    let update_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(update.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(update_json["tenant"]["name"], "Acme Runtime Updated");

    let deactivate = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/admin/tenants/acme-runtime/deactivate")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deactivate.status(), StatusCode::OK);
    let deactivate_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(deactivate.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(deactivate_json["tenant"]["status"], "inactive");
}

#[tokio::test]
async fn tenant_repo_binding_set_and_show() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    state
        .tenant_store
        .create_tenant("binding-tenant", "Binding Tenant")
        .await
        .unwrap();
    let app = router::build_router(state);

    let set_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/admin/tenants/binding-tenant/repository-binding")
                .header("content-type", "application/json")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "kind": "local",
                        "local_path": "/tmp/binding-tenant",
                        "branch": "main",
                        "branch_policy": "directCommit",
                        "credential_kind": "none",
                        "credential_ref": null,
                        "github_owner": null,
                        "github_repo": null,
                        "source_owner": "admin"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(set_response.status(), StatusCode::OK);
    let set_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(set_response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(set_json["binding"]["kind"], "local");
    assert_eq!(set_json["binding"]["sourceOwner"], "admin");

    let show_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/admin/tenants/binding-tenant/repository-binding")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(show_response.status(), StatusCode::OK);
    let show_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(show_response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(show_json["binding"]["localPath"], "/tmp/binding-tenant");
    assert_eq!(show_json["binding"]["branchPolicy"], "directCommit");
    assert_eq!(show_json["binding"]["sourceOwner"], "admin");
}

#[tokio::test]
async fn tenant_repo_binding_validate_and_missing_binding_behave_honestly() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    state
        .tenant_store
        .create_tenant("validate-tenant", "Validate Tenant")
        .await
        .unwrap();
    let app = router::build_router(state);

    let missing_show = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/admin/tenants/validate-tenant/repository-binding")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_show.status(), StatusCode::NOT_FOUND);
    let missing_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(missing_show.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(missing_json["error"], "binding_not_found");

    let validate_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/admin/tenants/validate-tenant/repository-binding/validate")
                .header("content-type", "application/json")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "kind": "local",
                        "local_path": "/tmp/validate-tenant",
                        "branch": "main",
                        "branch_policy": "directCommit",
                        "credential_kind": "none",
                        "credential_ref": null,
                        "github_owner": null,
                        "github_repo": null,
                        "source_owner": "admin"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(validate_response.status(), StatusCode::OK);
    let validate_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(validate_response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(validate_json["valid"], true);
    assert_eq!(validate_json["binding"]["sourceOwner"], "admin");
}

#[tokio::test]
async fn tenant_repo_binding_rejects_invalid_credential_ref() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    state
        .tenant_store
        .create_tenant("invalid-cred", "Invalid Cred")
        .await
        .unwrap();
    let app = router::build_router(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/admin/tenants/invalid-cred/repository-binding")
                .header("content-type", "application/json")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "kind": "github",
                        "local_path": null,
                        "remote_url": "https://github.com/acme/knowledge.git",
                        "branch": "main",
                        "branch_policy": "requirePullRequest",
                        "credential_kind": "githubApp",
                        "credential_ref": "raw-secret-material",
                        "github_owner": "acme",
                        "github_repo": "knowledge",
                        "source_owner": "admin"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(json["error"], "invalid_credential_ref");
}

#[tokio::test]
async fn tenant_api_requires_platform_admin_role() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/admin/tenants")
                .header("content-type", "application/json")
                .header("x-user-id", "developer-user")
                .header("x-user-role", "developer")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "slug": "forbidden-tenant",
                        "name": "Forbidden Tenant"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(json["error"], "forbidden");
    assert_eq!(json["message"], "PlatformAdmin role required");
}

#[tokio::test]
async fn git_provider_connection_lifecycle_and_binding_visibility_work() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let tenant = state
        .tenant_store
        .create_tenant("conn-tenant", "Connection Tenant")
        .await
        .unwrap();
    let app = router::build_router(state.clone());

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/admin/git-provider-connections")
                .header("content-type", "application/json")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": "Shared GitHub App",
                        "providerKind": "githubApp",
                        "appId": 12345,
                        "installationId": 67890,
                        "pemSecretRef": "local/github-app-pem",
                        "webhookSecretRef": "local/github-webhook"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_response.status(), StatusCode::CREATED);
    let create_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let connection_id = create_json["connection"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(create_json["connection"]["pemSecretRef"], "[redacted]");

    let pregrant_list = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/admin/tenants/{}/git-provider-connections",
                    tenant.slug
                ))
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(pregrant_list.status(), StatusCode::OK);
    let pregrant_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(pregrant_list.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(pregrant_json["connections"].as_array().unwrap().len(), 0);

    let forbidden_binding = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!(
                    "/api/v1/admin/tenants/{}/repository-binding",
                    tenant.slug
                ))
                .header("content-type", "application/json")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "kind": "github",
                        "remoteUrl": "https://github.com/acme/knowledge.git",
                        "branch": "main",
                        "branchPolicy": "RequirePullRequest",
                        "credentialKind": "githubApp",
                        "credentialRef": null,
                        "githubOwner": "acme",
                        "githubRepo": "knowledge",
                        "sourceOwner": "admin",
                        "gitProviderConnectionId": connection_id
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(forbidden_binding.status(), StatusCode::FORBIDDEN);

    let grant_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/admin/git-provider-connections/{}/tenants/{}",
                    connection_id, tenant.slug
                ))
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(grant_response.status(), StatusCode::OK);

    let visible_list = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/admin/tenants/{}/git-provider-connections",
                    tenant.slug
                ))
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(visible_list.status(), StatusCode::OK);
    let visible_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(visible_list.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(visible_json["connections"].as_array().unwrap().len(), 1);
    assert_eq!(visible_json["connections"][0]["id"], connection_id);

    let allowed_binding = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!(
                    "/api/v1/admin/tenants/{}/repository-binding",
                    tenant.slug
                ))
                .header("content-type", "application/json")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "kind": "github",
                        "remoteUrl": "https://github.com/acme/knowledge.git",
                        "branch": "main",
                        "branchPolicy": "RequirePullRequest",
                        "credentialKind": "githubApp",
                        "credentialRef": null,
                        "githubOwner": "acme",
                        "githubRepo": "knowledge",
                        "sourceOwner": "admin",
                        "gitProviderConnectionId": connection_id
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(allowed_binding.status(), StatusCode::OK);
    let allowed_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(allowed_binding.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        allowed_json["binding"]["gitProviderConnectionId"],
        connection_id
    );

    let revoke_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/api/v1/admin/git-provider-connections/{}/tenants/{}",
                    connection_id, tenant.slug
                ))
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoke_response.status(), StatusCode::OK);

    let post_revoke_list = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/admin/tenants/{}/git-provider-connections",
                    tenant.slug
                ))
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(post_revoke_list.status(), StatusCode::OK);
    let post_revoke_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(post_revoke_list.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(post_revoke_json["connections"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn admin_permissions_matrix_and_effective_endpoints_return_expected_shapes() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    let matrix_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/admin/permissions/matrix")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(matrix_response.status(), StatusCode::OK);
    let matrix_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(matrix_response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(matrix_json["matrix"]["platformAdmin"].is_array());
    assert!(matrix_json["matrix"]["admin"].is_array());
    assert!(matrix_json["matrix"]["architect"].is_array());
    assert!(matrix_json["matrix"]["techLead"].is_array());
    assert!(matrix_json["matrix"]["developer"].is_array());
    assert!(matrix_json["matrix"]["viewer"].is_array());
    assert!(matrix_json["actions"].as_array().unwrap().len() > 5);

    let effective_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/admin/permissions/effective?user_id=alice&role=developer")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(effective_response.status(), StatusCode::OK);
    let effective_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(effective_response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(effective_json["userId"], "alice");
    assert!(!effective_json["roles"].as_array().unwrap().is_empty());
    assert!(effective_json["granted"].as_array().is_some());
    assert!(effective_json["denied"].as_array().is_some());
}

// ─── Test 1: tenant list and hierarchy read operations ────────────────────────

#[tokio::test]
async fn tenant_list_and_hierarchy_read_operations() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let tenant_id = TenantId::new("default".to_string()).unwrap();
    let tenant_id = seed_company_unit(&state, &tenant_id).await;
    let app = router::build_router(state);

    // list_tenants – platform_admin
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/admin/tenants")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["success"], true);
    assert!(body["tenants"].is_array());
    // #44.d: cross-tenant list endpoints advertise scope=all.
    assert_eq!(body["scope"], "all");

    // list_tenants – forbidden without platform_admin
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/admin/tenants")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // list_hierarchy_units (no filter) – returns tenant unit
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/admin/hierarchy")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["success"], true);
    let units = body["units"].as_array().unwrap();
    assert!(!units.is_empty());
    let ids: Vec<_> = units.iter().filter_map(|u| u["id"].as_str()).collect();
    assert!(ids.contains(&tenant_id.as_str()));

    // show_hierarchy_unit
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/admin/hierarchy/{tenant_id}"))
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["success"], true);
    assert_eq!(body["unit"]["id"], tenant_id.as_str());

    // show_hierarchy_unit – not found
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/admin/hierarchy/00000000-0000-0000-0000-000000000000")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ─── Test 2: hierarchy update, ancestors/descendants, user memberships ────────

#[tokio::test]
async fn hierarchy_update_ancestors_descendants_memberships() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let tenant_id = TenantId::new("default".to_string()).unwrap();
    let tenant_id = seed_company_unit(&state, &tenant_id).await;

    // Create a child org unit via HTTP so it gets seeded in postgres
    let app = router::build_router(state.clone());
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/admin/hierarchy")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": "Engineering Org",
                        "unitType": "Organization",
                        "parentId": tenant_id,
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let org_unit_id = body["unit"]["id"].as_str().unwrap().to_string();

    // update_hierarchy_unit – rename the org
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/v1/admin/hierarchy/{org_unit_id}"))
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "name": "Platform Engineering" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["success"], true);
    assert_eq!(body["unit"]["name"], "Platform Engineering");

    // update_hierarchy_unit – not found
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/v1/admin/hierarchy/00000000-0000-0000-0000-000000000001")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "name": "Ghost" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // list_hierarchy_ancestors of org (parent = tenant-root organization)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/admin/hierarchy/{org_unit_id}/ancestors"))
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["success"], true);
    let ancestor_ids: Vec<_> = body["units"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|u| u["id"].as_str())
        .collect();
    assert!(ancestor_ids.contains(&tenant_id.as_str()));

    // list_hierarchy_descendants of tenant (should include org)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/admin/hierarchy/{tenant_id}/descendants"))
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["success"], true);
    let desc_ids: Vec<_> = body["units"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|u| u["id"].as_str())
        .collect();
    assert!(desc_ids.contains(&org_unit_id.as_str()));

    // assign a member then list_user_memberships
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/admin/hierarchy/{org_unit_id}/members"))
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "userId": "alice", "role": "Developer" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/admin/memberships?user_id=alice")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["success"], true);
    assert_eq!(body["userId"], "alice");
    let memberships = body["memberships"].as_array().unwrap();
    assert!(!memberships.is_empty());
    assert!(
        memberships
            .iter()
            .any(|m| m["unit_id"].as_str() == Some(org_unit_id.as_str()))
    );

    // add_domain_mapping
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/admin/tenants/default/domain-mappings")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "domain": "example.com" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    // 201 on success or 400 if tenant row not found — either way the handler ran
    assert!(
        resp.status() == StatusCode::CREATED || resp.status() == StatusCode::BAD_REQUEST,
        "unexpected status: {}",
        resp.status()
    );
}

// ─── Test 3: org CRUD and member lifecycle ────────────────────────────────────

#[tokio::test]
async fn org_crud_and_member_lifecycle() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let tenant_id = TenantId::new("default".to_string()).unwrap();
    let _tenant_id = seed_company_unit(&state, &tenant_id).await;
    let app = router::build_router(state);

    // list_orgs – initially empty
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/org")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(body.as_array().unwrap().is_empty());

    // create_org with invalid tenant_id
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/org")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": "Ghost Org",
                        "parentId": "00000000-0000-0000-0000-000000000099",
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // create_org – success
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/org")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": "Platform Engineering",
                        "description": "Core infra org",
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let org_id = body["id"].as_str().unwrap().to_string();

    // show_org
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/org/{org_id}"))
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["id"], org_id.as_str());
    assert_eq!(body["name"], "Platform Engineering");

    // show_org – not found
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/org/00000000-0000-0000-0000-000000000000")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // list_members – empty
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/org/{org_id}/members"))
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(body.as_array().unwrap().is_empty());

    // add_member
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/org/{org_id}/members"))
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "userId": "alice", "role": "Developer" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // list_members – alice present
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/org/{org_id}/members"))
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        body.as_array()
            .unwrap()
            .iter()
            .any(|m| m["userId"].as_str() == Some("alice"))
    );

    // set_member_role
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/org/{org_id}/members/alice/role"))
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "role": "tech_lead" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // remove_member
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/org/{org_id}/members/alice"))
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ─── Test 4: team CRUD and member lifecycle ───────────────────────────────────

#[tokio::test]
async fn team_crud_and_member_lifecycle() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let tenant_id = TenantId::new("default".to_string()).unwrap();
    let _tenant_id = seed_company_unit(&state, &tenant_id).await;
    let app = router::build_router(state.clone());

    // Create parent org first
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/org")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": "Platform Org",
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let org_body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let org_id = org_body["id"].as_str().unwrap().to_string();

    // list_teams – initially empty
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/team")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(body.as_array().unwrap().is_empty());

    // create_team – invalid org
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/team")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": "Ghost Team",
                        "orgId": "00000000-0000-0000-0000-000000000099",
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // create_team – success
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/team")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": "API Team",
                        "orgId": org_id,
                        "description": "Core API team",
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let team_id = body["id"].as_str().unwrap().to_string();

    // show_team
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/team/{team_id}"))
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["id"], team_id.as_str());
    assert_eq!(body["name"], "API Team");

    // show_team – not found
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/team/00000000-0000-0000-0000-000000000000")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // list_members – empty
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/team/{team_id}/members"))
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(body.as_array().unwrap().is_empty());

    // add_member
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/team/{team_id}/members"))
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "userId": "bob", "role": "Developer" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // list_members – bob present
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/team/{team_id}/members"))
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        body.as_array()
            .unwrap()
            .iter()
            .any(|m| m["userId"].as_str() == Some("bob"))
    );

    // set_member_role
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/team/{team_id}/members/bob/role"))
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "role": "tech_lead" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // list_teams with org filter
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/team?org={org_id}"))
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        body.as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"].as_str() == Some(team_id.as_str()))
    );

    // remove_member
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/team/{team_id}/members/bob"))
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ─── Test 5: user roles grant/revoke (no users table needed) ─────────────────

#[tokio::test]
async fn user_roles_grant_revoke_and_unsupported_schema() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let tenant_id = TenantId::new("default".to_string()).unwrap();
    let tenant_id = seed_company_unit(&state, &tenant_id).await;
    let app = router::build_router(state);

    // list_user_roles – no roles yet (just ensures handler runs; no users table needed)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/user/alice/roles")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(body.as_array().unwrap().is_empty());

    // grant_user_role – scope "tenant" (needs tenant unit)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/user/alice/roles")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "role": "Developer", "scope": "tenant" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["userId"], "alice");
    assert_eq!(body["role"], "developer");
    assert_eq!(body["unitId"], tenant_id.as_str());

    // list_user_roles – alice now has developer role
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/user/alice/roles")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(body.as_array().unwrap().iter().any(|r| {
        r["role"].as_str() == Some("developer") && r["unit_id"].as_str() == Some(tenant_id.as_str())
    }));

    // revoke_user_role – success
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/user/alice/roles/developer")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["success"], true);

    // revoke_user_role – 404 when no role to revoke
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/user/alice/roles/developer")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // list_users – returns 503 service_unavailable when no users table exists
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/user")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["error"], "user_schema_unsupported");

    // show_user – also returns 503 without users table
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/user/alice")
                .header("x-user-id", "alice")
                .header("x-user-role", "developer")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    // invite_user – also returns 503 without users table
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/user/invite")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "email": "new@example.com" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// ─── Test 6: govern status, config, audit and pending ────────────────────────

#[tokio::test]
async fn govern_status_config_audit_and_pending() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let tenant_id = TenantId::new("default".to_string()).unwrap();
    let _tenant_id = seed_company_unit(&state, &tenant_id).await;
    let app = router::build_router(state);

    // govern/status
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/govern/status")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(body["config"].is_object());
    assert!(body["metrics"]["pendingRequests"].is_number());

    // govern/config GET
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/govern/config")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let config_before: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(config_before["approvalMode"].is_string());

    // govern/config PUT – update min_approvers
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/govern/config")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "minApprovers": 2 })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["config"]["minApprovers"], 2);

    // govern/config PUT – invalid approval_mode
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/govern/config")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "approvalMode": "not_a_real_mode" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // govern/audit GET
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/govern/audit")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(body.as_array().is_some());

    // govern/audit GET – invalid since param
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/govern/audit?since=notadate")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // govern/pending GET
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/govern/pending")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(body.as_array().is_some());

    // govern/pending GET – invalid request type
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/govern/pending?type=not_a_type")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // govern/approve – request not found (404 path)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/govern/approve/00000000-0000-0000-0000-000000000000")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "comment": "lgtm" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    // 404 or 400 depending on whether uuid is recognised
    assert!(
        resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::BAD_REQUEST,
        "unexpected status: {}",
        resp.status()
    );

    // govern/reject – reason required validation
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/govern/reject/00000000-0000-0000-0000-000000000000")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&json!({})).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["error"], "reason_required");
}

// ─── Test: GET /api/v1/admin/stats ───────────────────────────────────────────

#[tokio::test]
async fn admin_stats_returns_counts() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let tenant_id = TenantId::new("default".to_string()).unwrap();
    let _tenant_id = seed_company_unit(&state, &tenant_id).await;
    let app = router::build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/admin/stats")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        body["tenantCount"].is_number(),
        "tenantCount must be a number"
    );
    assert!(body["userCount"].is_number(), "userCount must be a number");
    assert!(
        body["memoryCount"].is_number(),
        "memoryCount must be a number"
    );
    assert!(
        body["knowledgeCount"].is_number(),
        "knowledgeCount must be a number"
    );
}

// ─── Test: GET + POST /api/v1/govern/policies ────────────────────────────────

#[tokio::test]
async fn govern_policies_list_and_create() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let tenant_id = TenantId::new("default".to_string()).unwrap();
    let _tenant_id = seed_company_unit(&state, &tenant_id).await;
    let app = router::build_router(state);

    // GET /govern/policies – empty list initially
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/govern/policies")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(body["policies"].is_array(), "policies must be an array");

    // POST /govern/policies – create a new policy
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/govern/policies")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": "test-policy",
                        "description": "integration test policy",
                        "layer": "tenant",
                        "mode": "optional"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(body["id"].is_string(), "id must be a string");
    assert_eq!(body["name"], "test-policy");

    // POST /govern/policies – invalid layer returns 422
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/govern/policies")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "name": "bad", "layer": "unknown_layer" }))
                        .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["error"], "invalid_layer");
}

// ─── Test: POST /api/v1/memory/search is reachable ───────────────────────────

#[tokio::test]
async fn memory_search_route_is_mounted() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/memory/search")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "query": "test", "limit": 5 })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    // The route must exist – 404 is the only unacceptable status
    assert_ne!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "POST /api/v1/memory/search must be mounted"
    );
}

// ─── Test: govern/approve and govern/reject path order ───────────────────────

#[tokio::test]
async fn govern_approve_reject_path_order() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let tenant_id = TenantId::new("default".to_string()).unwrap();
    let _tenant_id = seed_company_unit(&state, &tenant_id).await;
    let app = router::build_router(state);

    // /govern/approve/{id} must be routable (not 404 / 405)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/govern/approve/00000000-0000-0000-0000-000000000001")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "comment": "lgtm" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_ne!(resp.status(), StatusCode::NOT_FOUND);

    // /govern/reject/{id} must be routable (not 404 / 405)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/govern/reject/00000000-0000-0000-0000-000000000001")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "reason": "not good" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_ne!(resp.status(), StatusCode::NOT_FOUND);
}

/// #44.d §2.5 — GET /govern/audit with the full `?tenant=` grammar.
///
/// Since Bundle D `governance_audit_log.acting_as_tenant_id` carries per-row
/// tenant attribution, so `/govern/audit` supports the same `?tenant=`
/// grammar as /user / /project / /org. This test asserts:
///   1. Gate semantics match the other endpoints (`?tenant=*` as non-admin
///      → 403 forbidden_scope).
///   2. `?tenant=<slug>` as PlatformAdmin returns the `scope:"tenant"`
///      envelope (with governance) or 503 governance_unavailable (fixture
///      variant) — crucially NOT 501 (was the pre-Bundle-D response).
///   3. `?tenant=*` + filters (?actor, ?since) compose correctly — the
///      presence of ?tenant=* must NOT cause the storage filter params
///      to be dropped.
///   4. Envelope shape is {success, scope:"all", items} when governance is
///      available; 503 governance_unavailable is accepted.
#[tokio::test]
async fn list_audit_cross_tenant_scope_gates_and_filter_compose() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    // Case 1: ?tenant=* as non-admin → 403 forbidden_scope.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/govern/audit?tenant=*")
                .header("x-user-id", "developer-user")
                .header("x-user-role", "developer")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["error"], "forbidden_scope");

    // Case 2 (Bundle D): ?tenant=<slug> as PlatformAdmin must NOT return 501.
    //
    // Acceptable outcomes:
    //   - 200 with envelope {success, scope:"tenant", tenant:{id,slug,name},
    //     items:[...]}: governance configured + slug resolved.
    //   - 404 tenant_not_found: slug "acme" not seeded in this fixture
    //     (common — tests use random slugs). Still proves §2.5 dispatches
    //     via the resolver rather than short-circuiting to 501.
    //   - 503 governance_unavailable: governance_storage not configured.
    //
    // What we MUST NOT see: 501 scope_not_implemented (pre-Bundle-D response).
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/govern/audit?tenant=acme")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_ne!(
        status,
        StatusCode::NOT_IMPLEMENTED,
        "Bundle D graduates §2.5: ?tenant=<slug> must no longer return 501; body={body}"
    );
    match status {
        StatusCode::OK => {
            assert_eq!(body["scope"], "tenant", "envelope scope must be \"tenant\"");
            assert!(
                body["tenant"].is_object(),
                "envelope.tenant must be present"
            );
            assert!(body["items"].is_array(), "envelope.items must be an array");
        }
        StatusCode::NOT_FOUND => {
            assert_eq!(body["error"], "tenant_not_found");
        }
        StatusCode::SERVICE_UNAVAILABLE => {
            assert_eq!(body["error"], "governance_unavailable");
        }
        other => panic!("/govern/audit?tenant=acme: unexpected status {other}; body={body}"),
    }

    // Case 3: ?tenant=* + ?since=1d + ?actor=<uuid> as PlatformAdmin.
    //
    // Two acceptable outcomes:
    //   - 200 with envelope {success, scope:"all", items[]}: governance is
    //     configured in the fixture and filters composed successfully.
    //   - 503 governance_unavailable: governance_storage not configured.
    //     This is the same skip pattern /user uses for variant fixtures.
    //
    // What we MUST NOT see:
    //   - 400: would mean filter composition silently broke under ?tenant=*.
    //   - 500: would mean the envelope branch has a bug.
    let actor_uuid = uuid::Uuid::new_v4();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/v1/govern/audit?tenant=*&since=1d&actor={actor_uuid}"
                ))
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    match resp.status() {
        StatusCode::OK => {
            let body: serde_json::Value = serde_json::from_slice(
                &axum::body::to_bytes(resp.into_body(), usize::MAX)
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(body["success"], true, "envelope success must be true");
            assert_eq!(body["scope"], "all", "envelope scope must be \"all\"");
            assert!(body["items"].is_array(), "envelope items must be an array");
        }
        StatusCode::SERVICE_UNAVAILABLE => {
            let body: serde_json::Value = serde_json::from_slice(
                &axum::body::to_bytes(resp.into_body(), usize::MAX)
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                body["error"], "governance_unavailable",
                "503 must carry governance_unavailable"
            );
            eprintln!(
                "/govern/audit?tenant=*: governance not configured in fixture, envelope\
                 shape couldn't be exercised — gates were still verified by cases 1+2"
            );
        }
        other => panic!(
            "/govern/audit?tenant=*&since=1d&actor=...: unexpected status {other}; filter\
             composition may be broken"
        ),
    }
}

/// #44.d §2.4 — GET /org with `?tenant=*` / `?tenant=all` / `?tenant=<slug>`:
/// same gate-layer + full-envelope coverage as /project.
#[tokio::test]
async fn list_orgs_cross_tenant_scope_gates_and_aliases() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    // Case 1: ?tenant=* as non-admin → 403 forbidden_scope.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/org?tenant=*")
                .header("x-user-id", "developer-user")
                .header("x-user-role", "developer")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["error"], "forbidden_scope");

    // Case 2: ?tenant=<slug> as PlatformAdmin → 501 NotImplemented,
    // and the error message mentions /org so clients know which endpoint.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/org?tenant=acme")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["error"], "scope_not_implemented");
    assert!(
        body["message"].as_str().unwrap_or("").contains("/org"),
        "501 body should mention the endpoint path, got: {}",
        body["message"]
    );

    // Case 3: ?tenant=* as PlatformAdmin → 200 with scope=all envelope.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/org?tenant=*")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["success"], true);
    assert_eq!(body["scope"], "all");
    assert!(body["items"].is_array());

    // Case 4: ?tenant= (empty) → treated as absent (tenant-scoped path),
    // defensive for clients sending unfilled form fields. Should NOT 501.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/org?tenant=")
                .header("x-user-id", "admin-user")
                .header("x-user-role", "admin")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(resp.status(), StatusCode::NOT_IMPLEMENTED);
    // Admin in a tenant-scoped /org call returns a bare array (legacy shape).
    if resp.status() == StatusCode::OK {
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(
            body.is_array(),
            "empty ?tenant= must yield legacy bare-array body"
        );
    }
}

/// #44.d §2.3 — GET /project with `?tenant=*` / `?tenant=all` / `?tenant=<slug>`:
/// same gate-layer coverage as the /user test, for the project endpoint.
#[tokio::test]
async fn list_projects_cross_tenant_scope_gates_and_aliases() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    // Case 1: ?tenant=* as non-admin → 403 forbidden_scope.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/project?tenant=*")
                .header("x-user-id", "developer-user")
                .header("x-user-role", "developer")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["error"], "forbidden_scope");

    // Case 2: ?tenant=<slug> as PlatformAdmin → 501 NotImplemented.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/project?tenant=acme")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["error"], "scope_not_implemented");

    // Case 3: ?tenant=* as PlatformAdmin → 200 with scope=all envelope.
    // Unlike /user, /project doesn't depend on a schema variant so this
    // should always return 200 (empty items in a fresh fixture).
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/project?tenant=*")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["success"], true);
    assert_eq!(body["scope"], "all");
    assert!(body["items"].is_array());

    // Case 4: ?tenant=all (deprecated alias) → same behavior as *.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/project?tenant=all")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["scope"], "all");
}

/// #44.d §2.2 — GET /user with `?tenant=*` / `?tenant=all` / `?tenant=<slug>`:
/// verifies the authorization and scope-resolution branches that fire
/// BEFORE any database work. This test is schema-independent — it covers
/// only the pre-DB gates and does not depend on which `users` table
/// variant (Main / IdpSync / none) is present in the runtime fixture.
#[tokio::test]
async fn list_users_cross_tenant_scope_gates_and_aliases() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    // Case 1: ?tenant=* as a non-admin → 403 forbidden_scope.
    // This MUST fire before any users-table probe, independent of schema.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/user?tenant=*")
                .header("x-user-id", "developer-user")
                .header("x-user-role", "developer")
                .header("x-tenant-id", "default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["error"], "forbidden_scope");

    // Case 2: ?tenant=<slug> as PlatformAdmin → 501 NotImplemented.
    // This is the "deferred to PR #65" path; we want it to return a
    // stable, discoverable error so clients know to use ?tenant=* today.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/user?tenant=acme")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["error"], "scope_not_implemented");

    // Case 3: ?tenant=all (deprecated alias) → treated as `*` by
    // PlatformAdmin: reaches the cross-tenant branch. With no users
    // table in the fixture, this yields 503 user_schema_unsupported
    // (same code path as the tenant-scoped baseline). The important
    // assertion is "not 403/501" — the alias was accepted.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/user?tenant=all")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(resp.status(), StatusCode::FORBIDDEN);
    assert_ne!(resp.status(), StatusCode::NOT_IMPLEMENTED);
    // Accepts either the cross-tenant success envelope OR the 503 that
    // the existing `detect_user_table_variant` helper emits when the
    // users table isn't provisioned — both prove the alias was
    // accepted and PlatformAdmin was permitted.
    let status = resp.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::SERVICE_UNAVAILABLE,
        "unexpected status for ?tenant=all PlatformAdmin: {status}"
    );
    if status == StatusCode::OK {
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["scope"], "all");
        assert_eq!(body["success"], true);
        assert!(body["items"].is_array());
    }
}

// =============================================================================
// #44.d §4.1 — cross-tenant response envelope contract
// =============================================================================
//
// Lock down the scope=all envelope shape so it cannot silently drift between
// endpoints. Covers /user, /project, /org. /govern/audit has its own dedicated
// dispatch test (`list_audit_cross_tenant_scope_gates_and_filter_compose`)
// because pre-Bundle-D rows have `acting_as_tenant_id = NULL` and therefore
// surface without tenant decoration, which breaks the "every item must carry
// tenantId+tenantSlug" invariant this helper enforces.
//
// Per tasks.md §4.1: "for every scope=all response across the 5 endpoints,
// every item MUST contain non-empty tenantId and tenantSlug. Failing this
// test gates the PR."
//
// Ideally this would live in its own `cross_tenant_contract_test.rs` (per the
// tasks doc), but Rust integration tests don't share private helpers without
// a `tests/common/mod.rs` scaffold. Rather than duplicate ~300 lines of
// fixture setup, the tests live here next to `test_app_state`. Can be
// migrated later when a shared common module is introduced.

/// Hit `{endpoint_path}?tenant=*` as PlatformAdmin and assert the full
/// scope=all envelope contract. This is the single source of truth for
/// "what a cross-tenant list response looks like".
///
/// Contract:
///   1. HTTP 200
///   2. body.success == true
///   3. body.scope   == "all"
///   4. body.items is an array
///   5. every item has non-empty string `tenantId` AND `tenantSlug`
///   6. items span >= `min_tenants` distinct tenants (proves the endpoint
///      actually reads across tenant boundaries; catches bugs where the
///      filter accidentally collapses to a single tenant)
async fn assert_cross_tenant_envelope_contract(
    app: axum::Router,
    endpoint_path: &str,
    min_tenants: usize,
) {
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(endpoint_path)
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "{endpoint_path}: expected 200 OK for cross-tenant list"
    );
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        body["success"], true,
        "{endpoint_path}: envelope success must be true, body={body}"
    );
    assert_eq!(
        body["scope"], "all",
        "{endpoint_path}: envelope scope must be \"all\", body={body}"
    );
    let items = body["items"]
        .as_array()
        .unwrap_or_else(|| panic!("{endpoint_path}: envelope.items must be an array, body={body}"));
    let mut tenant_ids = std::collections::HashSet::new();
    for (i, item) in items.iter().enumerate() {
        let tid = item["tenantId"].as_str().unwrap_or_else(|| {
            panic!("{endpoint_path}: items[{i}].tenantId must be a string, item={item}")
        });
        assert!(
            !tid.is_empty(),
            "{endpoint_path}: items[{i}].tenantId must be non-empty, item={item}"
        );
        let tslug = item["tenantSlug"].as_str().unwrap_or_else(|| {
            panic!("{endpoint_path}: items[{i}].tenantSlug must be a string, item={item}")
        });
        assert!(
            !tslug.is_empty(),
            "{endpoint_path}: items[{i}].tenantSlug must be non-empty, item={item}"
        );
        tenant_ids.insert(tid.to_string());
    }
    assert!(
        tenant_ids.len() >= min_tenants,
        "{endpoint_path}: items must span >= {min_tenants} tenants; got {} ({tenant_ids:?})",
        tenant_ids.len(),
    );
}

/// #44.d §4.1 — Envelope contract for /project and /org across 2 tenants.
///
/// Seeds two tenants, each with one Organization and one Project unit,
/// then asserts the full cross-tenant envelope contract on both endpoints.
#[tokio::test]
async fn cross_tenant_envelope_contract_project_and_org() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };

    // Seed two tenants with content. Using randomized slugs prevents
    // collisions with fixtures when the postgres container is reused.
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let slug_a = format!("contract-a-{}", &suffix[..8]);
    let slug_b = format!("contract-b-{}", &suffix[..8]);
    let tenant_a = state
        .tenant_store
        .create_tenant(&slug_a, "Contract Tenant A")
        .await
        .expect("create tenant A");
    let tenant_b = state
        .tenant_store
        .create_tenant(&slug_b, "Contract Tenant B")
        .await
        .expect("create tenant B");
    for t in [&tenant_a, &tenant_b] {
        let tid = TenantId::new(t.id.as_str().to_string()).unwrap();
        seed_unit_of_type(
            &state,
            &tid,
            mk_core::types::UnitType::Organization,
            &format!("org-{}", t.slug),
        )
        .await;
        seed_unit_of_type(
            &state,
            &tid,
            mk_core::types::UnitType::Project,
            &format!("proj-{}", t.slug),
        )
        .await;
    }

    let app = router::build_router(state);

    // Both endpoints must emit identical envelope shape. If a future PR
    // decorates one endpoint differently (e.g. drops tenantSlug), this
    // test fails and forces the divergence to be addressed at review time.
    assert_cross_tenant_envelope_contract(app.clone(), "/api/v1/project?tenant=*", 2).await;
    assert_cross_tenant_envelope_contract(app, "/api/v1/org?tenant=*", 2).await;
}

/// #44.d §4.1 — Envelope contract for /user, best-effort.
///
/// /user has a storage-variant dependency (the fixture's persistence
/// layer may not implement the cross-tenant query used in scope=all
/// mode — see PR #64 for the 503 skip pattern). We still assert the
/// envelope contract WHEN the endpoint returns 200; a 503 is logged
/// and treated as a skip, because asserting "there must be users"
/// would be a fixture-coupling regression.
#[tokio::test]
async fn cross_tenant_envelope_contract_user_best_effort() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping server runtime test: Docker not available");
        return;
    };
    let app = router::build_router(state);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/user?tenant=*")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    match resp.status() {
        StatusCode::OK => {
            // Full contract applies. Use min_tenants=0 because the fixture
            // may not seed users and an empty items[] still satisfies the
            // envelope shape — what we care about here is that NO row violates
            // the contract when rows DO exist.
            let body: serde_json::Value = serde_json::from_slice(
                &axum::body::to_bytes(resp.into_body(), usize::MAX)
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(body["success"], true);
            assert_eq!(body["scope"], "all");
            let items = body["items"].as_array().expect("items array");
            for (i, item) in items.iter().enumerate() {
                let tid = item["tenantId"].as_str().unwrap_or_else(|| {
                    panic!("/user: items[{i}].tenantId must be a string, item={item}")
                });
                assert!(!tid.is_empty(), "/user: items[{i}].tenantId empty");
                let tslug = item["tenantSlug"].as_str().unwrap_or_else(|| {
                    panic!("/user: items[{i}].tenantSlug must be a string, item={item}")
                });
                assert!(!tslug.is_empty(), "/user: items[{i}].tenantSlug empty");
            }
        }
        StatusCode::SERVICE_UNAVAILABLE => {
            eprintln!(
                "cross_tenant_envelope_contract_user_best_effort: storage returned 503 \
                 (expected for fixture variants without cross-tenant user query) — skipping"
            );
        }
        other => panic!("/user?tenant=*: unexpected status {other}"),
    }
}

// ---------------------------------------------------------------------------
// #44.d §5 — Integration test matrix for ?tenant= across /user /project /org.
//
// Each test below maps to a specific task in the cross-tenant listing change
// (openspec/changes/add-cross-tenant-admin-listing/tasks.md). Keeping them
// grouped here (rather than spread across per-endpoint modules) ensures the
// matrix is easy to audit as a single coherent contract.
//
// Convention: tests gracefully skip when Docker is unavailable, following
// the same pattern as the surrounding suite. Postgres-only behaviors (e.g.
// seeded tenants, 404 tenant_not_found) therefore fall through in
// headless-CI variants instead of panicking.
// ---------------------------------------------------------------------------

/// #44.d §5.3 — Without ?tenant=, list endpoints preserve legacy behavior.
///
/// This locks the most important backward-compat guarantee of the feature:
/// clients that never adopt ?tenant= see BYTE-IDENTICAL responses to the
/// pre-RFC world. We verify by asserting the response is a bare JSON array
/// (not the new envelope), which is how all three endpoints shaped their
/// legacy body. If a future PR accidentally wraps the legacy path in the
/// new envelope, this test fails.
#[tokio::test]
async fn cross_tenant_s5_3_no_tenant_param_preserves_legacy_shape() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping §5.3: Docker not available");
        return;
    };
    let app = router::build_router(state);

    for endpoint in ["/api/v1/user", "/api/v1/project", "/api/v1/org"] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(endpoint)
                    .header("x-user-id", "platform-admin")
                    .header("x-user-role", "platform_admin")
                    .header("x-tenant-id", "default")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // 200 when fixture supplies the endpoint's backing data; 503 is
        // acceptable when the variant's cross-query path is unimplemented
        // (mirrors the best-effort pattern used elsewhere in this suite).
        if resp.status() == StatusCode::SERVICE_UNAVAILABLE {
            eprintln!("§5.3: {endpoint} returned 503 (fixture variant skip)");
            continue;
        }
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "{endpoint}: legacy path (no ?tenant) must return 200"
        );
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(
            body.is_array(),
            "{endpoint}: legacy path must return a bare array, not the envelope. \
             The envelope would BREAK every pre-RFC client. Got: {body}"
        );
    }
}

/// #44.d §5.4 — PlatformAdmin + ?tenant=<slug> returns ONLY that tenant.
///
/// Seeds two tenants with distinguishable units, issues `?tenant=<slug-a>`,
/// and asserts (a) the new `scope: "tenant"` envelope, (b) the echoed
/// `tenant` object, and (c) every returned item belongs to tenant A only.
#[tokio::test]
async fn cross_tenant_s5_4_slug_returns_only_that_tenant() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping §5.4: Docker not available");
        return;
    };
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let slug_a = format!("s5-4-a-{}", &suffix[..8]);
    let slug_b = format!("s5-4-b-{}", &suffix[..8]);
    let tenant_a = state
        .tenant_store
        .create_tenant(&slug_a, "S5.4 Tenant A")
        .await
        .expect("create tenant A");
    let tenant_b = state
        .tenant_store
        .create_tenant(&slug_b, "S5.4 Tenant B")
        .await
        .expect("create tenant B");
    for t in [&tenant_a, &tenant_b] {
        let tid = TenantId::new(t.id.as_str().to_string()).unwrap();
        seed_unit_of_type(
            &state,
            &tid,
            mk_core::types::UnitType::Project,
            &format!("proj-{}", t.slug),
        )
        .await;
        seed_unit_of_type(
            &state,
            &tid,
            mk_core::types::UnitType::Organization,
            &format!("org-{}", t.slug),
        )
        .await;
    }
    let app = router::build_router(state);

    for (endpoint_base, expected_unit_name) in [
        ("/api/v1/project", format!("proj-{slug_a}")),
        ("/api/v1/org", format!("org-{slug_a}")),
    ] {
        let uri = format!("{endpoint_base}?tenant={slug_a}");
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&uri)
                    .header("x-user-id", "platform-admin")
                    .header("x-user-role", "platform_admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{uri}: expected 200");
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["success"], true, "{uri}: success must be true");
        assert_eq!(
            body["scope"], "tenant",
            "{uri}: scope must be 'tenant' for slug path, got {}",
            body["scope"]
        );
        assert_eq!(
            body["tenant"]["slug"], slug_a,
            "{uri}: echoed tenant.slug must match requested slug"
        );
        assert_eq!(
            body["tenant"]["id"],
            tenant_a.id.as_str(),
            "{uri}: echoed tenant.id must match resolved tenant"
        );
        let items = body["items"].as_array().expect("items array");
        // Every item must belong to tenant A. This is the CORE guarantee
        // of the slug path — if any item leaks from tenant B we have a
        // cross-tenant isolation bug (silent data leak to PlatformAdmin).
        assert!(
            !items.is_empty(),
            "{uri}: expected at least the seeded {expected_unit_name} unit, got empty. body={body}"
        );
        for (i, item) in items.iter().enumerate() {
            assert_eq!(
                item["tenantId"],
                tenant_a.id.as_str(),
                "{uri}: items[{i}].tenantId must be tenant A only, item={item}"
            );
            assert_eq!(
                item["tenantSlug"], slug_a,
                "{uri}: items[{i}].tenantSlug must be slug A only, item={item}"
            );
        }
    }
}

/// #44.d §5.5 — PlatformAdmin + ?tenant=<nonexistent> returns 404 tenant_not_found.
///
/// Must happen AFTER authorization (PlatformAdmin required) so unauthenticated
/// callers can't probe for tenant existence — they see forbidden_scope first.
/// Here we assert the admin case hits the 404 cleanly.
#[tokio::test]
async fn cross_tenant_s5_5_nonexistent_slug_returns_404() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping §5.5: Docker not available");
        return;
    };
    let app = router::build_router(state);
    // Slug guaranteed not to exist — UUID-suffixed + explicit "nope" prefix.
    let ghost = format!("nope-{}", uuid::Uuid::new_v4().simple());

    for endpoint in ["/api/v1/user", "/api/v1/project", "/api/v1/org"] {
        let uri = format!("{endpoint}?tenant={ghost}");
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&uri)
                    .header("x-user-id", "platform-admin")
                    .header("x-user-role", "platform_admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "{uri}: expected 404 for nonexistent tenant slug"
        );
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            body["error"], "tenant_not_found",
            "{uri}: error code must be tenant_not_found, body={body}"
        );
        assert!(
            body["message"].as_str().unwrap_or("").contains(&ghost),
            "{uri}: 404 message should echo the requested slug for debuggability, got {}",
            body["message"]
        );
    }
}

/// #44.d §5.6 — ?tenant=all emits a deprecation warning + resolves like *.
///
/// Uses tracing-test to capture logs emitted during the request. The test
/// fails if EITHER the log line is missing (client-facing behavior drift)
/// OR the request is rejected (semantic drift from the ?tenant=* alias).
#[tokio::test]
#[tracing_test::traced_test]
async fn cross_tenant_s5_6_all_alias_emits_deprecation_log() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping §5.6: Docker not available");
        return;
    };
    let app = router::build_router(state);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/project?tenant=all")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "?tenant=all must resolve identically to ?tenant=* (same 200)"
    );
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        body["scope"], "all",
        "?tenant=all must emit scope=all envelope, body={body}"
    );
    // The compat log target carries structured fields; tracing-test lets
    // us assert the message fragment is present. We look for the endpoint
    // label + the parameter + the replacement hint — three independent
    // signals so the test doesn't brittle-bind to exact wording.
    assert!(
        logs_contain("deprecated tenant scope alias"),
        "expected compat warning for ?tenant=all; check tracing::warn target='compat'"
    );
    assert!(
        logs_contain("?tenant=*"),
        "compat warning must hint the canonical replacement"
    );
    assert!(
        logs_contain("/project"),
        "compat warning must tag the endpoint label for log filtering"
    );
}

/// #44.d §8 — Legacy `X-Target-Tenant-Id` header still works, but emits a
/// deprecation warning every time.
///
/// Behavioral contract (what we can NOT break without a major version):
///
/// 1. Requests carrying `X-Target-Tenant-Id` continue to be honored.
///    Existing clients (CI scripts, support tools, etc.) must not break
///    the day we ship this PR — removal is planned for a separate later
///    minor.
/// 2. Every such request logs a `tracing::warn!(target = "compat", ...)`
///    line naming (a) the header, (b) the replacement grammar, and (c)
///    enough of the raw value that log aggregation can correlate with
///    client traffic and identify stragglers.
///
/// This test exercises the happy path — auth succeeds with the header
/// set, a downstream endpoint returns 200 — and asserts the warn line is
/// present. If a future refactor stops honoring the header OR stops
/// emitting the warning, this test fails loudly with a precise message.
#[tokio::test]
#[tracing_test::traced_test]
async fn cross_tenant_s8_legacy_target_tenant_header_emits_deprecation_log() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping §8: Docker not available");
        return;
    };
    let app = router::build_router(state);

    // Use a GET that only needs auth (not cross-tenant scoping) so the
    // compat::warn signal we assert on is UNAMBIGUOUSLY from the
    // `X-Target-Tenant-Id` path, not from a `?tenant=*` alias code path.
    // `/api/v1/project` with plain tenant headers is sufficient.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/project")
                .header("x-user-id", "platform-admin")
                .header("x-user-role", "platform_admin")
                .header("x-tenant-id", "default")
                // The legacy header we want to verify. A realistic client
                // would pass a tenant slug or id here. We use a visually
                // distinctive value so we can assert the log line echoed
                // its prefix (operators need this to grep log streams and
                // find which clients are still sending it).
                .header("x-target-tenant-id", "legacy-client-abc123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "X-Target-Tenant-Id must remain honored — removal requires a minor bump"
    );

    // Three independent fragments — if ANY of them is missing, the log
    // line has drifted and operators' compat dashboards will break.
    assert!(
        logs_contain("deprecated cross-tenant header"),
        "expected compat warning naming the deprecation"
    );
    assert!(
        logs_contain("X-Target-Tenant-Id"),
        "compat warning must name the exact header for grep-ability"
    );
    assert!(
        logs_contain("?tenant="),
        "compat warning must point clients at the replacement grammar"
    );
    assert!(
        logs_contain("legacy-client-abc123"),
        "compat warning must echo the raw value prefix so operators can correlate with client traffic"
    );
}

/// #44.d §5.7 — Pagination ordering is stable across ?tenant=* responses.
///
/// Not true pagination (the endpoints don't implement offset/limit today),
/// but the STABLE-ORDERING contract the docs promise:
///   (tenant_id ASC, name ASC, id ASC)
/// Any repeated call must return items in the exact same order, and any
/// two consecutive items must respect the total ordering. If a future PR
/// adds real pagination, this test is the first line of defense against
/// ordering drift between pages.
#[tokio::test]
async fn cross_tenant_s5_7_stable_ordering_across_calls() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping §5.7: Docker not available");
        return;
    };
    // Seed 3 tenants × 2 projects each so we have enough data for the
    // ordering predicate to be non-trivial. 6 items, 3 tenant buckets.
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    for i in 0..3 {
        let slug = format!("s5-7-{i}-{}", &suffix[..8]);
        let tenant = state
            .tenant_store
            .create_tenant(&slug, &format!("S5.7 Tenant {i}"))
            .await
            .expect("create tenant");
        let tid = TenantId::new(tenant.id.as_str().to_string()).unwrap();
        for name in ["alpha", "beta"] {
            seed_unit_of_type(
                &state,
                &tid,
                mk_core::types::UnitType::Project,
                &format!("{name}-{slug}"),
            )
            .await;
        }
    }
    let app = router::build_router(state);

    let fetch_items = || async {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/project?tenant=*")
                    .header("x-user-id", "platform-admin")
                    .header("x-user-role", "platform_admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        body["items"].as_array().cloned().unwrap_or_default()
    };

    let first = fetch_items().await;
    let second = fetch_items().await;
    assert_eq!(
        first, second,
        "/project?tenant=* must return items in the same order across calls"
    );

    // Verify the ordering predicate holds: (tenant_id, name, id) ASC.
    for pair in first.windows(2) {
        let a = &pair[0];
        let b = &pair[1];
        let key = |v: &serde_json::Value| {
            (
                v["tenantId"].as_str().unwrap_or("").to_string(),
                v["name"].as_str().unwrap_or("").to_string(),
                v["id"].as_str().unwrap_or("").to_string(),
            )
        };
        let ka = key(a);
        let kb = key(b);
        assert!(
            ka <= kb,
            "ordering violated: {ka:?} > {kb:?}. The (tenant_id, name, id) \
             ordering is part of the documented envelope contract \
             (docs/api/admin.md)."
        );
    }
}

/// #44.d §5.8 — POST ?tenant=* returns 400 scope_not_allowed_for_write.
///
/// ?tenant=* is a READ-side broadening. Allowing writes under it would
/// either (a) silently collapse to the caller's tenant — bad, it looks
/// like you got what you asked for when you didn't — or (b) fan out to
/// every tenant, which is a bulk-write superpower that deserves its own
/// explicit API. Both are wrong; 400 at the entry point is correct.
#[tokio::test]
async fn cross_tenant_s5_8_post_with_tenant_star_rejected() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping §5.8: Docker not available");
        return;
    };
    let app = router::build_router(state);

    // Body shape is irrelevant — the guard fires BEFORE request-body
    // parsing. Using a plausible-looking payload avoids accidental early
    // returns that would mask the guard regression.
    let cases: &[(&str, serde_json::Value)] = &[
        (
            "/api/v1/user?tenant=*",
            json!({ "email": "x@example.com", "name": "x" }),
        ),
        (
            "/api/v1/project?tenant=*",
            json!({ "name": "x", "parent_id": null }),
        ),
        (
            "/api/v1/org?tenant=*",
            json!({ "name": "x", "parent_id": null }),
        ),
        // ?tenant=all (alias) must be rejected symmetrically; otherwise
        // clients using the deprecated alias would find a write hole.
        (
            "/api/v1/project?tenant=all",
            json!({ "name": "x", "parent_id": null }),
        ),
    ];
    for (uri, body_json) in cases {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(*uri)
                    .header("x-user-id", "platform-admin")
                    .header("x-user-role", "platform_admin")
                    .header("x-tenant-id", "default")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(body_json).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "{uri}: POST with cross-tenant scope must be rejected with 400"
        );
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            body["error"], "scope_not_allowed_for_write",
            "{uri}: error code must be scope_not_allowed_for_write, body={body}"
        );
    }
}

// =============================================================================
// B2 5.5 followup — HTTP integration tests for /admin/tenants/.../wiring
//
// These complement the unit tests in cli/src/server/tenant_wiring_api.rs which
// lock the JSON wire contract in isolation. The tests below drive the full
// axum router + auth layer, exercising the production code paths end-to-end:
//
//   * PA role enforcement against real `authenticated_platform_context`
//   * router mounting under /api/v1/
//   * 404 path when slug is absent from the pod-local registry
//   * list endpoint `total` field matches registry size
//   * reason field IS exposed to PA (by design — this is the authorisation
//     boundary for sensitive error text; see the unit test rationale)
//   * cross-state coverage: Loading, Available, LoadingFailed all round-trip
//
// Fixture: reuses `test_app_state()` (real Postgres via test container).
// Skipped when Docker is unavailable, matching the rest of this file.
// =============================================================================

use aeterna::server::tenant_runtime_state::TenantRuntimeState;

/// Helper: build a PA request against a wiring endpoint.
fn pa_wiring_request(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("x-user-id", "pa-user")
        .header("x-user-role", "platform_admin")
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn wiring_list_requires_platform_admin() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping wiring integration test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    // A non-PA user hits 403 with the standard error envelope — locks the
    // contract documented in tenant_wiring_api::require_platform_admin.
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/admin/tenants/wiring")
                .header("x-user-id", "dev-user")
                .header("x-user-role", "developer")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["error"], "forbidden");
    assert_eq!(body["message"], "PlatformAdmin role required");
}

#[tokio::test]
async fn wiring_get_requires_platform_admin() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping wiring integration test: Docker not available");
        return;
    };
    // Seed state so we can be sure the 403 isn't actually a 404 masquerading.
    state
        .tenant_runtime_state
        .set("acme", TenantRuntimeState::available_now(1))
        .await;
    let app = router::build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/admin/tenants/acme/wiring")
                .header("x-user-id", "viewer")
                .header("x-user-role", "viewer")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn wiring_get_returns_404_when_pod_has_no_entry() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping wiring integration test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    let resp = app
        .oneshot(pa_wiring_request(
            "/api/v1/admin/tenants/never-wired/wiring",
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    // Error code is stable for UI consumers — guard it.
    assert_eq!(body["error"], "tenant_not_wired");
    // The message documents the distinction between "pod hasn't touched it"
    // vs "pod failed to wire" — an assertion on substring locks that intent.
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("No wiring state on this pod"),
        "expected operator-friendly 404 message, got {body}"
    );
}

#[tokio::test]
async fn wiring_get_available_round_trips_rev_and_wired_at() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping wiring integration test: Docker not available");
        return;
    };
    state.tenant_runtime_state.mark_loading("acme").await;
    let rev = state.tenant_runtime_state.mark_available("acme").await;
    let app = router::build_router(state);

    let resp = app
        .oneshot(pa_wiring_request("/api/v1/admin/tenants/acme/wiring"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["slug"], "acme");
    assert_eq!(body["state"], "available");
    assert_eq!(body["rev"], rev);
    assert!(
        body["wiredAt"].as_u64().is_some(),
        "wiredAt must be a unix timestamp (number), body={body}"
    );
    // Failure-only fields must be absent on the happy path.
    assert!(body.get("reason").is_none());
    assert!(body.get("retryCount").is_none());
}

#[tokio::test]
async fn wiring_get_failed_exposes_reason_to_pa() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping wiring integration test: Docker not available");
        return;
    };
    state.tenant_runtime_state.mark_loading("acme").await;
    // The reason carries sensitive substance (here: a secret ref) — this
    // test locks that PA *does* see it. If PA auth is ever bypassed by a
    // bug, the unit test `envelope_flattens_failed_state_including_reason`
    // in tenant_wiring_api::tests triggers as the info-leak canary.
    let retry_count = state
        .tenant_runtime_state
        .mark_failed("acme", "secret openai.api_key missing")
        .await;
    let app = router::build_router(state);

    let resp = app
        .oneshot(pa_wiring_request("/api/v1/admin/tenants/acme/wiring"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["state"], "loadingFailed");
    assert_eq!(body["reason"], "secret openai.api_key missing");
    assert_eq!(body["retryCount"], retry_count);
    assert!(body["lastAttemptAt"].as_u64().is_some());
}

#[tokio::test]
async fn wiring_get_loading_returns_since_field() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping wiring integration test: Docker not available");
        return;
    };
    state.tenant_runtime_state.mark_loading("acme").await;
    let app = router::build_router(state);

    let resp = app
        .oneshot(pa_wiring_request("/api/v1/admin/tenants/acme/wiring"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["state"], "loading");
    assert!(
        body["since"].as_u64().is_some(),
        "loading state must carry `since` (unix timestamp), body={body}"
    );
}

#[tokio::test]
async fn wiring_list_returns_total_and_every_seeded_tenant() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping wiring integration test: Docker not available");
        return;
    };
    // Seed three tenants across all three states. Intentionally using
    // slugs with distinct alpha ordering so a broken iteration order
    // would show up in the snapshot.
    state.tenant_runtime_state.mark_loading("alpha").await;
    state.tenant_runtime_state.mark_available("alpha").await;
    state.tenant_runtime_state.mark_loading("beta").await;
    state.tenant_runtime_state.mark_loading("gamma").await;
    state
        .tenant_runtime_state
        .mark_failed("gamma", "resolver timeout")
        .await;
    let app = router::build_router(state);

    let resp = app
        .oneshot(pa_wiring_request("/api/v1/admin/tenants/wiring"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["total"], 3);
    let tenants = body["tenants"].as_array().unwrap();
    assert_eq!(tenants.len(), 3);

    // Build a slug→state map for order-agnostic assertions.
    let mut by_slug: std::collections::HashMap<String, &serde_json::Value> =
        std::collections::HashMap::new();
    for t in tenants {
        by_slug.insert(t["slug"].as_str().unwrap().to_string(), t);
    }
    assert_eq!(by_slug.get("alpha").unwrap()["state"], "available");
    assert_eq!(by_slug.get("beta").unwrap()["state"], "loading");
    assert_eq!(by_slug.get("gamma").unwrap()["state"], "loadingFailed");
    assert_eq!(by_slug.get("gamma").unwrap()["reason"], "resolver timeout");
}

#[tokio::test]
async fn wiring_list_empty_when_no_tenants_have_been_touched() {
    let Some((state, _tmp)) = test_app_state().await else {
        eprintln!("Skipping wiring integration test: Docker not available");
        return;
    };
    let app = router::build_router(state);

    let resp = app
        .oneshot(pa_wiring_request("/api/v1/admin/tenants/wiring"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    // Empty pod — total is 0 and `tenants` is an empty array (not null).
    assert_eq!(body["total"], 0);
    assert_eq!(body["tenants"].as_array().unwrap().len(), 0);
}
