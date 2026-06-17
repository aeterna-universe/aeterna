use std::path::{Path, PathBuf};
use std::sync::Arc;

use adapters::auth::cedar::CedarAuthorizer;
use adapters::auth::permit::PermitAuthorizationService;
use agent_a2a::{AuthState as A2aAuthState, Config as A2aConfig};
use anyhow::Context;
use async_trait::async_trait;
use chrono::Utc;
use idp_sync::azure::AzureAdClient;
use idp_sync::config::{AzureAdConfig, IdpProvider, IdpSyncConfig, OktaConfig};
use idp_sync::okta::OktaClient;
use idp_sync::{IdpClient, IdpSyncService};
use knowledge::api::GovernanceDashboardApi;
use knowledge::git_provider::{GitHubProvider, GitProvider};
use knowledge::governance::GovernanceEngine;
use knowledge::manager::KnowledgeManager;
use knowledge::repository::{GitRepository, RemoteConfig};
use knowledge::tenant_repo_resolver::TenantRepositoryResolver;
use memory::embedding::create_embedding_service_from_env;
use memory::llm::create_llm_service_from_env;
use memory::manager::MemoryManager;
use memory::provider_registry::TenantProviderRegistry;
use memory::reasoning::{DefaultReflectiveReasoner, ReflectiveReasoner};
use mk_core::secret::SecretReference;
use mk_core::traits::{AuthorizationService, GitProviderConnectionRegistry};
use mk_core::types::{
    DEFAULT_TENANT_SLUG, GitProviderConnection, GitProviderKind, INSTANCE_SCOPE_TENANT_ID,
    PROVIDER_GITHUB, ReasoningStrategy, ReasoningTrace, Role, RoleIdentifier, TenantContext,
    TenantId, UserId,
};
use serde::Deserialize;
use storage::git_provider_connection_store::{
    InMemoryGitProviderConnectionStore, RedisGitProviderConnectionStore,
};
use storage::governance::GovernanceStorage;
use storage::graph_duckdb::{DuckDbGraphConfig, DuckDbGraphStore, SnapshotPolicy};
use storage::postgres::PostgresBackend;
use storage::secret_provider::build_secret_provider_from_env;
use storage::tenant_config_provider::KubernetesTenantConfigProvider;
use storage::tenant_store::{TenantRepositoryBindingStore, TenantStore};
use sync::bridge::SyncManager;
use sync::state_persister::DatabasePersister;
use sync::websocket::{AuthToken, TokenValidator, WsResult, WsServer};
use tools::server::McpServer;

use super::plugin_auth::{RefreshTokenStore, RefreshTokenStoreBackend};
use super::service_token_validator;
use super::{AppState, PluginAuthState, bootstrap_tracker};

const DEFAULT_K8S_NAMESPACE: &str = "default";
const SYSTEM_MARKER_TENANT_ID: &str = "00000000-0000-0000-0000-000000000000";

const ENV_AUTH_BACKEND: &str = "AETERNA_AUTH_BACKEND";
const ENV_GIT_PROVIDER_CONNECTIONS_PATH: &str = "AETERNA_GIT_PROVIDER_CONNECTIONS_PATH";
const AUTH_BACKEND_ALLOW_ALL: &str = "allow-all";

/// The version sentinel written into Redis by the
/// [`storage::redis_version_guard::RedisVersionGuard`] at boot.
///
/// Resolution order, most-precise first:
///
/// 1. `AETERNA_GIT_SHA` env var if set at build time (set by the CI release
///    pipeline). A git SHA changes on every commit, so this catches dev
///    rebuilds that share a Cargo version.
/// 2. `AETERNA_BUILD_VERSION` env var if set at build time (allows
///    overriding the version string in tests or hotfix builds without
///    bumping `Cargo.toml`).
/// 3. `CARGO_PKG_VERSION` (always available; coarsest — only changes on
///    explicit version bumps).
///
/// Operators can also override at runtime via `AETERNA_RUNTIME_VERSION`
/// when iterating in dev (e.g. forcing a purge on a known-good binary).
fn aeterna_runtime_version() -> String {
    if let Ok(v) = std::env::var("AETERNA_RUNTIME_VERSION") {
        if !v.is_empty() {
            return v;
        }
    }
    if let Some(sha) = option_env!("AETERNA_GIT_SHA") {
        if !sha.is_empty() {
            return sha.to_string();
        }
    }
    if let Some(v) = option_env!("AETERNA_BUILD_VERSION") {
        if !v.is_empty() {
            return v.to_string();
        }
    }
    env!("CARGO_PKG_VERSION").to_string()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitProviderConnectionSeed {
    id: String,
    name: String,
    provider_kind: GitProviderKind,
    app_id: u64,
    installation_id: u64,
    pem_secret_ref: String,
    webhook_secret_ref: Option<String>,
    #[serde(default)]
    allowed_tenant_ids: Vec<TenantId>,
}

fn git_provider_connections_seed_path() -> Option<PathBuf> {
    std::env::var(ENV_GIT_PROVIDER_CONNECTIONS_PATH)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn same_seed_metadata(existing: &GitProviderConnection, seed: &GitProviderConnectionSeed) -> bool {
    existing.id == seed.id
        && existing.name == seed.name
        && existing.provider_kind == seed.provider_kind
        && existing.app_id == seed.app_id
        && existing.installation_id == seed.installation_id
        && existing.pem_secret_ref == seed.pem_secret_ref
        && existing.webhook_secret_ref == seed.webhook_secret_ref
}

async fn seed_git_provider_connections_from_path(
    registry: &(
         dyn GitProviderConnectionRegistry<
        Error = storage::git_provider_connection_store::GitProviderConnectionError,
    > + Send
             + Sync
     ),
    path: &Path,
) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read git provider connection seed file {}", path.display()))?;
    let seeds: Vec<GitProviderConnectionSeed> = serde_json::from_str(&raw).with_context(|| {
        format!(
            "parse git provider connection seed file {} as JSON",
            path.display()
        )
    })?;

    for seed in seeds {
        match registry
            .get_connection(&seed.id)
            .await
            .map_err(|e| anyhow::anyhow!("lookup git provider connection '{}': {e}", seed.id))?
        {
            Some(existing) => {
                if !same_seed_metadata(&existing, &seed) {
                    anyhow::bail!(
                        "git provider connection seed '{}' conflicts with existing registry metadata; delete/recreate the connection or align the chart seed",
                        seed.id
                    );
                }

                tracing::info!(
                    connection_id = %seed.id,
                    seeded_allowed_tenants = seed.allowed_tenant_ids.len(),
                    runtime_allowed_tenants = existing.allowed_tenant_ids.len(),
                    "Seeded git provider connection already exists; preserving runtime tenant visibility"
                );
            }
            None => {
                let now = Utc::now();
                registry
                    .create_connection(GitProviderConnection {
                        id: seed.id.clone(),
                        name: seed.name.clone(),
                        provider_kind: seed.provider_kind.clone(),
                        app_id: seed.app_id,
                        installation_id: seed.installation_id,
                        pem_secret_ref: seed.pem_secret_ref.clone(),
                        webhook_secret_ref: seed.webhook_secret_ref.clone(),
                        allowed_tenant_ids: Vec::new(),
                        created_at: now,
                        updated_at: now,
                    })
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!("create seeded git provider connection '{}': {e}", seed.id)
                    })?;
                for tenant_id in &seed.allowed_tenant_ids {
                    registry
                        .grant_tenant_visibility(&seed.id, tenant_id)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "grant tenant '{}' visibility of seeded connection '{}': {e}",
                                tenant_id,
                                seed.id
                            )
                        })?;
                }
                tracing::info!(
                    connection_id = %seed.id,
                    allowed_tenants = seed.allowed_tenant_ids.len(),
                    "Seeded git provider connection from bootstrap file"
                );
            }
        }
    }

    Ok(())
}

async fn seed_git_provider_connections_from_env(
    registry: &(
         dyn GitProviderConnectionRegistry<
        Error = storage::git_provider_connection_store::GitProviderConnectionError,
    > + Send
             + Sync
     ),
) -> anyhow::Result<()> {
    let Some(path) = git_provider_connections_seed_path() else {
        return Ok(());
    };
    seed_git_provider_connections_from_path(registry, &path).await
}

pub async fn bootstrap() -> anyhow::Result<Arc<AppState>> {
    // Bootstrap phase tracker (B2 task 6.1). Instrumented across the
    // major phases below; finalized with `mark_ready()` immediately
    // before we hand the state back to `serve::run`. On error we
    // deliberately do NOT call `mark_ready()` — the process is about to
    // exit anyway (kubelet restart path) and leaving the tracker with
    // a `running` overall state would only matter if someone attached
    // a debugger mid-failure.
    let bootstrap_tracker = Arc::new(bootstrap_tracker::BootstrapTracker::new());

    bootstrap_tracker.begin("env_and_config");
    validate_required_env()?;

    let config = Arc::new(config::load_from_env()?);
    bootstrap_tracker.complete("env_and_config");

    bootstrap_tracker.begin("database");
    let tenant_url = postgres_connection_url(&config);
    // Dual-pool config (issue #58, RLS enforcement).
    //
    // DATABASE_URL_ADMIN points the admin pool at the BYPASSRLS aeterna_admin
    // role. If the env var is unset we fall back to the tenant URL \u2014 in
    // pre-Wave-6 environments both URLs point at the same role (typically
    // `postgres`), so a single pool is behaviourally correct. The split
    // becomes meaningful once Wave 6 flips DATABASE_URL to aeterna_app.
    let admin_url = std::env::var("DATABASE_URL_ADMIN").ok();
    let postgres = Arc::new(match admin_url {
        Some(admin) => PostgresBackend::new_with_admin(&tenant_url, &admin).await?,
        None => PostgresBackend::new(&tenant_url).await?,
    });
    postgres.initialize_schema().await?;

    seed_platform_admin(postgres.pool(), &config.admin_bootstrap).await?;
    seed_k8s_service_account(postgres.pool(), &config.admin_bootstrap).await?;
    bootstrap_tracker.complete("database");

    bootstrap_tracker.begin("knowledge_git");
    let governance_storage = Some(Arc::new(GovernanceStorage::new(postgres.pool().clone())));

    let git_provider: Option<Arc<dyn GitProvider>> = if let (Some(owner), Some(repo)) = (
        &config.knowledge_repo.github_owner,
        &config.knowledge_repo.github_repo,
    ) {
        if let (Some(app_id), Some(installation_id), Some(pem)) = (
            config.knowledge_repo.github_app_id,
            config.knowledge_repo.github_installation_id,
            &config.knowledge_repo.github_app_pem,
        ) {
            tracing::info!(
                "Initializing GitHub App auth (app_id={app_id}, installation_id={installation_id})"
            );
            Some(Arc::new(
                GitHubProvider::new_with_app(
                    app_id,
                    installation_id,
                    pem,
                    owner.clone(),
                    repo.clone(),
                    config.knowledge_repo.webhook_secret.clone(),
                )
                .await
                .context("Failed to build GitHub App provider")?,
            ))
        } else if let Some(token) = &config.knowledge_repo.github_token {
            tracing::info!("Initializing GitHub PAT auth");
            Some(Arc::new(
                GitHubProvider::new(
                    token,
                    owner.clone(),
                    repo.clone(),
                    config.knowledge_repo.webhook_secret.clone(),
                )
                .context("Failed to build GitHub PAT provider")?,
            ))
        } else {
            None
        }
    } else {
        None
    };

    let remote_config = config
        .knowledge_repo
        .remote_url
        .as_ref()
        .map(|url| RemoteConfig {
            url: url.clone(),
            branch: config.knowledge_repo.branch.clone(),
            git_provider: git_provider.clone(),
        });

    let knowledge_repository = Arc::new(
        GitRepository::new_with_remote(knowledge_repo_path(), remote_config)
            .context("Failed to initialize knowledge repository")?,
    );
    bootstrap_tracker.complete("knowledge_git");

    bootstrap_tracker.begin("memory_and_providers");
    let auth_for_memory = build_boxed_auth_service()?;
    let auth_service = build_anyhow_auth_service()?;

    let features = RuntimeFeatures::from_env();

    let llm_service = if features.reflective {
        create_llm_service_from_env()?
    } else {
        None
    };
    let reasoner = llm_service
        .clone()
        .map(|llm| Arc::new(DefaultReflectiveReasoner::new(llm)) as Arc<dyn ReflectiveReasoner>);
    let mcp_reasoner = reasoner
        .clone()
        .unwrap_or_else(|| Arc::new(NoopReflectiveReasoner) as Arc<dyn ReflectiveReasoner>);

    let graph_store = create_graph_store(&config)?;

    let mut memory_config = config.memory.clone();
    if !features.rlm {
        memory_config.rlm.enabled = false;
    }
    if !features.reflective {
        memory_config.reasoning.enabled = false;
    }
    if !features.cca {
        tracing::info!("CCA feature disabled via AETERNA_FEATURE_CCA");
    }
    if !features.radkit {
        tracing::info!("Radkit/A2A feature disabled via AETERNA_FEATURE_RADKIT");
    }

    let platform_embedding = create_embedding_service_from_env()?;
    let platform_llm = llm_service.clone();

    // Build the shared SecretBackend (envelope-encrypted Postgres rows with
    // the DEK wrapped by a KMS provider selected via AETERNA_KMS_PROVIDER).
    // The TenantConfigProvider delegates secret ops to this instance; other
    // call sites (e.g. git token resolution in B2) will reuse the same Arc.
    let secret_backend =
        storage::secret_backend::build_secret_backend_from_env(postgres.pool().clone())
            .await
            .map_err(|e| anyhow::anyhow!("failed to build tenant secret backend: {e}"))?;

    let tenant_config_provider = Arc::new(KubernetesTenantConfigProvider::new(
        tenant_config_provider_namespace_from_config(&config.k8s_auth),
        secret_backend.clone(),
    ));

    let mut registry =
        TenantProviderRegistry::new(platform_llm.clone(), platform_embedding.clone());

    // Wire the type-erased config resolver + typed SecretResolverRegistry
    // so the registry can resolve tenant-specific providers without a
    // generic TenantConfigProvider param.
    //
    // B4 §3.5 Phase B — the former `SecretResolver` closure has been
    // replaced end-to-end by the typed registry. Secret lookup now
    // goes: TenantConfigDocument.secret_references[logical_name]
    // → SecretReference variant → registry resolver. This removes the
    // String-through-closure path that allocated plaintext copies.
    {
        use memory::provider_registry::ConfigResolver;
        use mk_core::traits::TenantConfigProvider;

        let cp_for_config: Arc<KubernetesTenantConfigProvider> = tenant_config_provider.clone();
        let config_resolver: ConfigResolver = Arc::new(move |tenant_id| {
            let provider = cp_for_config.clone();
            Box::pin(async move { provider.get_config(&tenant_id).await.ok().flatten() })
        });
        registry.set_config_resolver(config_resolver);
    }

    // B4 §3.5 — install the typed SecretResolverRegistry.
    //
    // Registered resolvers: inline (test / dev-only), postgres
    // (envelope-encrypted via the shared SecretBackend), env, file,
    // k8s (pod-downward API), vault (stub unless built with --features
    // vault). The k8s resolver uses a no-op fetcher when
    // --features k8s-secrets is not enabled — from_pod_environment()
    // returns a resolver whose fetch() emits BackendUnavailable with
    // a clear diagnostic.
    {
        use memory::secret_resolver::SecretResolverRegistry;
        use memory::secret_resolvers::{
            EnvRefResolver, FileRefResolver, InlineRefResolver, K8sRefResolver,
            PodDownwardApiFetcher, PostgresRefResolver, VaultRefResolver,
        };

        let mut secret_registry = SecretResolverRegistry::new();
        secret_registry.register(Arc::new(InlineRefResolver::new()));
        secret_registry.register(Arc::new(PostgresRefResolver::new(secret_backend.clone())));
        secret_registry.register(Arc::new(EnvRefResolver::new()));
        secret_registry.register(Arc::new(FileRefResolver::new()));
        secret_registry.register(Arc::new(VaultRefResolver::new()));

        // K8s resolver: build the pod-downward-API fetcher + default
        // namespace. from_pod_environment() only fails on feature-on
        // builds outside a pod — log and skip registration in that
        // case so non-pod dev environments still boot.
        match PodDownwardApiFetcher::from_pod_environment() {
            Ok(fetcher) => {
                let default_ns = PodDownwardApiFetcher::read_pod_namespace().await;
                let mut k8s = K8sRefResolver::new(fetcher);
                if let Some(ns) = default_ns {
                    k8s = k8s.with_default_namespace(ns);
                }
                secret_registry.register(Arc::new(k8s));
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "K8s secret resolver unavailable — SecretReference::K8s values will fail \
                     to resolve. This is expected outside Kubernetes; in-cluster deployments \
                     should investigate.",
                );
            }
        }

        registry.set_secret_resolver_registry(Arc::new(secret_registry));
    }

    vault_marker_self_test(&registry).await;

    let provider_registry = Arc::new(registry);

    let mut memory_manager = MemoryManager::new()
        .with_config(memory_config)
        .with_auth_service(auth_for_memory)
        .with_provider_registry(provider_registry.clone());

    if let Some(embedding_service) = platform_embedding {
        memory_manager = memory_manager.with_embedding_service(embedding_service.clone());
    }

    if let Some(llm_service) = llm_service.clone() {
        memory_manager = memory_manager.with_llm_service(llm_service);
    }

    if let Some(reasoner) = reasoner.clone() {
        memory_manager = memory_manager.with_reasoner(reasoner);
    }

    if let Some(graph_store) = graph_store.clone() {
        memory_manager = memory_manager.with_graph_store(graph_store.clone());
    }

    let governance_engine = Arc::new(build_governance_engine(
        postgres.clone(),
        knowledge_repository.clone(),
        llm_service,
    ));

    let knowledge_manager = Arc::new(KnowledgeManager::new(
        knowledge_repository.clone(),
        governance_engine.clone(),
    ));
    memory_manager = memory_manager.with_knowledge_manager(knowledge_manager.clone());

    let memory_manager = Arc::new(memory_manager);
    bootstrap_tracker.complete("memory_and_providers");

    bootstrap_tracker.begin("sync_and_protocols");
    let persister = Arc::new(DatabasePersister::new(postgres.clone(), "sync".to_string()));
    let sync_manager = Arc::new(
        SyncManager::new(
            memory_manager.clone(),
            knowledge_manager.clone(),
            config.deployment.clone(),
            None,
            persister,
            None,
        )
        .await?,
    );

    let governance_dashboard = Arc::new(GovernanceDashboardApi::new(
        governance_engine.clone(),
        postgres.clone(),
        config.deployment.clone(),
    ));

    let mcp_server = Arc::new(McpServer::new(
        memory_manager.clone(),
        sync_manager.clone(),
        knowledge_manager.clone(),
        knowledge_repository.clone(),
        postgres.clone(),
        governance_engine.clone(),
        mcp_reasoner,
        auth_service.clone(),
        None,
        graph_store.clone().map(|g| g as Arc<DuckDbGraphStore>),
        governance_storage.clone(),
    ));

    let a2a_config = Arc::new(if features.radkit {
        A2aConfig::from_env().unwrap_or_default()
    } else {
        A2aConfig::default()
    });
    let a2a_auth_state = Arc::new(A2aAuthState {
        api_key: a2a_config.auth.api_key.clone(),
        jwt_secret: a2a_config.auth.jwt_secret.clone(),
        enabled: a2a_config.auth.enabled,
        trusted_identity: a2a_config.auth.trusted_identity.clone(),
    });
    a2a_auth_state.validate()?;
    bootstrap_tracker.complete("sync_and_protocols");

    bootstrap_tracker.begin("redis_and_auth_stores");
    // Build a Redis connection manager for shared state stores.
    // If Redis is available, use Redis-backed stores for HA; otherwise fall back to in-memory.
    let (redis_conn, redis_url): (Option<Arc<redis::aio::ConnectionManager>>, Option<String>) = {
        let rc = &config.providers.redis;
        let url = format!("redis://{}:{}/{}", rc.host, rc.port, rc.db);
        match redis::Client::open(url.as_str()) {
            Ok(client) => match client.get_connection_manager().await {
                Ok(cm) => {
                    tracing::info!("Redis connection manager established for shared state stores");
                    (Some(Arc::new(cm)), Some(url))
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to connect to Redis ({url}): {e}. Falling back to in-memory stores."
                    );
                    (None, None)
                }
            },
            Err(e) => {
                tracing::warn!("Invalid Redis URL ({url}): {e}. Falling back to in-memory stores.");
                (None, None)
            }
        }
    };

    // Server-version guard: when this binary's version differs from the
    // sentinel previously stored in Redis, purge `aeterna:*` keys so we do
    // not read records whose schema the new binary cannot deserialise. The
    // canonical example is the rc.8 → rc.9 jump where stale
    // `git_provider_connections` records carrying camelCase enum tags
    // failed deserialise on the new code path. Uses a Redis-resident
    // leader-election lock so only one replica purges during a rolling
    // deploy. Fail-open: if Redis is unreachable here we let bootstrap
    // continue rather than refusing to start the pod (the warning is
    // surfaced in deployment logs and matched by alerting). See
    // `storage::redis_version_guard` for the full contract.
    if let Some(conn) = &redis_conn {
        let guard = storage::redis_version_guard::RedisVersionGuard::new(conn.clone());
        let expected_version = aeterna_runtime_version();
        match guard.ensure_version(&expected_version).await {
            Ok(action) => {
                tracing::info!(
                    expected_version = %expected_version,
                    ?action,
                    "redis version guard completed"
                );
            }
            Err(e) => {
                tracing::warn!(
                    expected_version = %expected_version,
                    error = %e,
                    "redis version guard failed; continuing with possibly-stale cache state"
                );
            }
        }
    }

    let refresh_store = match &redis_conn {
        Some(conn) => {
            let store = storage::RedisStore::new(conn.clone(), "aeterna:refresh_tokens");
            RefreshTokenStoreBackend::Redis(super::plugin_auth::RedisRefreshTokenStore::new(store))
        }
        None => RefreshTokenStoreBackend::InMemory(RefreshTokenStore::new()),
    };

    let plugin_auth_state = Arc::new(PluginAuthState {
        config: config.plugin_auth.clone(),
        postgres: Some(postgres.clone()),
        refresh_store,
    });
    let k8s_auth_config = config.k8s_auth.clone();

    // Initialize backup job stores (export/import) with Redis when available.
    super::backup_api::init_job_stores(redis_conn.as_ref());
    bootstrap_tracker.complete("redis_and_auth_stores");

    bootstrap_tracker.begin("assemble_state");
    let (idp_config, idp_client, idp_sync_service) = build_optional_idp_services(postgres.clone())?;
    let ws_server = Arc::new(WsServer::new(Arc::new(AllowAllTokenValidator {
        access_token_ttl_seconds: config.plugin_auth.access_token_ttl_seconds.unwrap_or(3600),
    })));
    let webhook_secret = config.knowledge_repo.webhook_secret.clone();

    let (shutdown_tx, _) = tokio::sync::watch::channel(false);

    // Tenant stores and repository resolver
    let tenant_store = Arc::new(TenantStore::new(postgres.pool().clone()));
    let tenant_repository_binding_store =
        Arc::new(TenantRepositoryBindingStore::new(postgres.pool().clone()));
    // Build the SecretProvider used by `TenantRepositoryResolver` to fetch
    // GitHub App PEM keys and other credential refs stored on per-tenant
    // bindings. Driven by `AETERNA_SECRET_PROVIDER` (default `local` in
    // dev/CI; refused in production unless `vault`). Includes a startup
    // self-test so a misconfigured Vault address fails the deployment
    // health check rather than the first repo clone. See
    // `storage::secret_provider::build_secret_provider_from_env` for the
    // full selector matrix and production safety contract.
    let secret_provider = build_secret_provider_from_env()
        .await
        .map_err(|e| anyhow::anyhow!("failed to build platform secret provider: {e}"))?;
    let git_provider_connection_registry: Arc<
        dyn mk_core::traits::GitProviderConnectionRegistry<
                Error = storage::git_provider_connection_store::GitProviderConnectionError,
            > + Send
            + Sync,
    > = match &redis_conn {
        Some(conn) => Arc::new(RedisGitProviderConnectionStore::new(conn.clone())),
        None => Arc::new(InMemoryGitProviderConnectionStore::new()),
    };
    bootstrap_tracker.begin("git_provider_connections");
    seed_git_provider_connections_from_env(git_provider_connection_registry.as_ref()).await?;
    bootstrap_tracker.complete("git_provider_connections");
    let tenant_repo_resolver = Arc::new(
        TenantRepositoryResolver::new(
            tenant_repository_binding_store.clone(),
            knowledge_repo_path(),
            secret_provider,
        )
        .with_connection_registry(git_provider_connection_registry.clone()),
    );
    Ok(Arc::new(AppState {
        config,
        postgres,
        memory_manager,
        knowledge_manager,
        knowledge_repository,
        governance_engine,
        governance_dashboard,
        auth_service,
        mcp_server,
        sync_manager,
        git_provider,
        webhook_secret,
        event_publisher: None,
        graph_store,
        governance_storage,
        reasoner,
        ws_server,
        a2a_config,
        a2a_auth_state,
        plugin_auth_state,
        revocation_cache: Arc::new(service_token_validator::RevocationCache::new(
            redis_conn.clone(),
        )),
        k8s_auth_config,
        idp_config,
        idp_sync_service,
        idp_client,
        shutdown_tx: Arc::new(shutdown_tx),
        tenant_store,
        tenant_repository_binding_store,
        tenant_repo_resolver,
        tenant_config_provider,
        provider_registry,
        git_provider_connection_registry,
        redis_conn,
        redis_url,
        tenant_runtime_state: Arc::new(
            crate::server::tenant_runtime_state::TenantRuntimeRegistry::new(),
        ),
        bootstrap_tracker: {
            // Close the final phase and finalize the tracker BEFORE
            // handing the Arc<AppState> back. Post this point the
            // `/admin/bootstrap/status` endpoint will report
            // `state: "completed"` with all phase durations populated.
            bootstrap_tracker.complete("assemble_state");
            bootstrap_tracker.mark_ready();
            bootstrap_tracker
        },
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuntimeFeatures {
    cca: bool,
    radkit: bool,
    rlm: bool,
    reflective: bool,
}

impl RuntimeFeatures {
    fn from_env() -> Self {
        Self {
            cca: feature_enabled("AETERNA_FEATURE_CCA", true),
            radkit: feature_enabled("AETERNA_FEATURE_RADKIT", true),
            rlm: feature_enabled("AETERNA_FEATURE_RLM", true),
            reflective: feature_enabled("AETERNA_FEATURE_REFLECTIVE", true),
        }
    }
}

fn feature_enabled(key: &str, default: bool) -> bool {
    std::env::var(key)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(default)
}

fn validate_required_env() -> anyhow::Result<()> {
    let mut missing = Vec::new();

    if std::env::var("AETERNA_POSTGRESQL_HOST").is_err() && std::env::var("PG_HOST").is_err() {
        missing.push("AETERNA_POSTGRESQL_HOST|PG_HOST");
    }
    if std::env::var("AETERNA_POSTGRESQL_DATABASE").is_err()
        && std::env::var("PG_DATABASE").is_err()
    {
        missing.push("AETERNA_POSTGRESQL_DATABASE|PG_DATABASE");
    }
    if std::env::var("AETERNA_REDIS_HOST").is_err() && std::env::var("RD_HOST").is_err() {
        missing.push("AETERNA_REDIS_HOST|RD_HOST");
    }

    if missing.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "Required environment variables are not set: {}",
            missing.join(", ")
        )
    }
}

fn postgres_connection_url(config: &config::Config) -> String {
    let pg = &config.providers.postgres;
    format!(
        "postgres://{}:{}@{}:{}/{}",
        pg.username, pg.password, pg.host, pg.port, pg.database
    )
}

fn knowledge_repo_path() -> PathBuf {
    std::env::var("AETERNA_KNOWLEDGE_REPO_PATH")
        .map_or_else(|_| PathBuf::from("./knowledge-repo"), PathBuf::from)
}

fn tenant_config_provider_namespace() -> String {
    std::env::var("AETERNA_K8S_NAMESPACE").unwrap_or_else(|_| DEFAULT_K8S_NAMESPACE.to_string())
}

fn tenant_config_provider_namespace_from_config(k8s_auth: &config::KubernetesAuthConfig) -> String {
    k8s_auth
        .namespace
        .as_deref()
        .unwrap_or(DEFAULT_K8S_NAMESPACE)
        .to_string()
}

async fn vault_marker_self_test(registry: &TenantProviderRegistry) {
    if std::env::var("VAULT_ADDR")
        .map(|v| v.trim().is_empty())
        .unwrap_or(true)
    {
        return;
    }

    let tenant = TenantId::new(SYSTEM_MARKER_TENANT_ID.to_string())
        .expect("hard-coded zero uuid is a valid TenantId");
    let chart_version_ref = SecretReference::Vault {
        mount: "secret".to_string(),
        path: "global/aeterna/.bootstrap-marker".to_string(),
        field: "chart_version".to_string(),
    };
    let timestamp_ref = SecretReference::Vault {
        mount: "secret".to_string(),
        path: "global/aeterna/.bootstrap-marker".to_string(),
        field: "timestamp".to_string(),
    };

    match registry
        .resolve_secret_ref(&tenant, &chart_version_ref)
        .await
    {
        Ok(chart_version) => {
            let chart_version = String::from_utf8_lossy(chart_version.expose()).to_string();
            let timestamp = registry
                .resolve_secret_ref(&tenant, &timestamp_ref)
                .await
                .ok()
                .map(|value| String::from_utf8_lossy(value.expose()).to_string());
            tracing::info!(
                chart_version = %chart_version,
                marker_timestamp = timestamp.as_deref().unwrap_or("<unknown>"),
                "Vault/OpenBao bootstrap marker read succeeded at startup"
            );
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "Vault/OpenBao bootstrap marker read failed at startup; continuing"
            );
        }
    }
}

fn create_graph_store(config: &config::Config) -> anyhow::Result<Option<Arc<DuckDbGraphStore>>> {
    if !config.providers.graph.enabled {
        return Ok(None);
    }

    let graph = DuckDbGraphStore::new(DuckDbGraphConfig {
        path: config.providers.graph.database_path.clone(),
        s3_bucket: config.providers.graph.s3_bucket.clone(),
        s3_prefix: config.providers.graph.s3_prefix.clone(),
        s3_endpoint: config.providers.graph.s3_endpoint.clone(),
        s3_region: Some(config.providers.graph.s3_region.clone()),
        reader_pool_size: config.providers.graph.reader_pool_size,
        snapshot_policy: SnapshotPolicy {
            full_interval_secs: config.providers.graph.snapshot_full_interval_secs,
            delta_interval_secs: config.providers.graph.snapshot_delta_interval_secs,
        },
        ..Default::default()
    })?;

    Ok(Some(Arc::new(graph)))
}

fn build_governance_engine(
    postgres: Arc<PostgresBackend>,
    repository: Arc<GitRepository>,
    llm_service: Option<
        Arc<
            dyn mk_core::traits::LlmService<Error = Box<dyn std::error::Error + Send + Sync>>
                + Send
                + Sync,
        >,
    >,
) -> GovernanceEngine {
    let mut engine = GovernanceEngine::new()
        .with_storage(postgres)
        .with_repository(repository);

    if let Some(llm_service) = llm_service {
        engine = engine.with_llm_service(llm_service);
    }

    engine
}

fn build_anyhow_auth_service()
-> anyhow::Result<Arc<dyn AuthorizationService<Error = anyhow::Error> + Send + Sync>> {
    let backend =
        std::env::var(ENV_AUTH_BACKEND).unwrap_or_else(|_| AUTH_BACKEND_ALLOW_ALL.to_string());

    match backend.as_str() {
        "cedar" => {
            let policy = std::env::var("AETERNA_CEDAR_POLICY_TEXT")
                .context("AETERNA_CEDAR_POLICY_TEXT is required for cedar auth")?;
            let schema = std::env::var("AETERNA_CEDAR_SCHEMA_TEXT")
                .context("AETERNA_CEDAR_SCHEMA_TEXT is required for cedar auth")?;
            let mut service = CedarAuthorizer::new(&policy, &schema)?;
            if let Ok(url) = std::env::var("AETERNA_OPAL_FETCHER_URL") {
                tracing::info!(url = %url, "Cedar auth: OPAL fetcher configured");
                service = service.with_opal_fetcher(url);
            }
            Ok(Arc::new(AnyhowAuthWrapper { inner: service }))
        }
        "permit" => {
            let api_key = std::env::var("AETERNA_PERMIT_API_KEY")
                .context("AETERNA_PERMIT_API_KEY is required for permit auth")?;
            let pdp_url = std::env::var("AETERNA_PERMIT_PDP_URL")
                .context("AETERNA_PERMIT_PDP_URL is required for permit auth")?;
            Ok(Arc::new(AnyhowAuthWrapper {
                inner: PermitAuthorizationService::new(&api_key, &pdp_url),
            }))
        }
        _ => {
            // Allow-all is only safe in local development or test environments.
            // Emit a warning so operators know this is active.  To suppress this
            // warning in a legitimate dev environment, set
            // AETERNA_ALLOW_PERMISSIVE_AUTH=dev.
            let permissive_mode =
                std::env::var("AETERNA_ALLOW_PERMISSIVE_AUTH").unwrap_or_default();
            if permissive_mode == "dev" {
                tracing::debug!(
                    "{} auth active (AETERNA_ALLOW_PERMISSIVE_AUTH=dev)",
                    AUTH_BACKEND_ALLOW_ALL
                );
            } else {
                tracing::warn!(
                    backend = %backend,
                    "Using {} authorization service. This grants every caller full access and MUST NOT be used in production. Set {}=cedar or {}=permit, or set AETERNA_ALLOW_PERMISSIVE_AUTH=dev to silence this warning.",
                    AUTH_BACKEND_ALLOW_ALL,
                    ENV_AUTH_BACKEND,
                    ENV_AUTH_BACKEND,
                );
            }
            Ok(Arc::new(AllowAllAuthService))
        }
    }
}

fn build_boxed_auth_service() -> anyhow::Result<
    Arc<dyn AuthorizationService<Error = Box<dyn std::error::Error + Send + Sync>> + Send + Sync>,
> {
    Ok(Arc::new(AllowAllBoxedAuthService))
}

fn build_optional_idp_services(
    postgres: Arc<PostgresBackend>,
) -> anyhow::Result<(
    Option<Arc<IdpSyncConfig>>,
    Option<Arc<dyn IdpClient>>,
    Option<Arc<IdpSyncService>>,
)> {
    let provider = match std::env::var("AETERNA_IDP_PROVIDER") {
        Ok(provider) => provider,
        Err(_) => return Ok((None, None, None)),
    };

    let database_url = std::env::var("AETERNA_IDP_DATABASE_URL").unwrap_or_default();
    let webhook_port = std::env::var("AETERNA_IDP_WEBHOOK_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(8090);
    let webhook_secret = std::env::var("AETERNA_IDP_WEBHOOK_SECRET").ok();

    let provider = match provider.as_str() {
        "okta" => IdpProvider::Okta(OktaConfig {
            domain: std::env::var("AETERNA_OKTA_DOMAIN")?,
            api_token: std::env::var("AETERNA_OKTA_API_TOKEN")?,
            scim_enabled: std::env::var("AETERNA_OKTA_SCIM_ENABLED")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),
            group_filter: std::env::var("AETERNA_OKTA_GROUP_FILTER").ok(),
            user_filter: std::env::var("AETERNA_OKTA_USER_FILTER").ok(),
        }),
        "azure" => IdpProvider::AzureAd(AzureAdConfig {
            tenant_id: std::env::var("AETERNA_AZURE_TENANT_ID")?,
            client_id: std::env::var("AETERNA_AZURE_CLIENT_ID")?,
            client_secret: std::env::var("AETERNA_AZURE_CLIENT_SECRET")?,
            group_filter: std::env::var("AETERNA_AZURE_GROUP_FILTER").ok(),
            include_nested_groups: std::env::var("AETERNA_AZURE_INCLUDE_NESTED_GROUPS")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),
        }),
        PROVIDER_GITHUB => {
            tracing::info!("GitHub org sync uses the dedicated /api/v1/admin/sync/github endpoint");
            return Ok((None, None, None));
        }
        _ => return Ok((None, None, None)),
    };

    let config = Arc::new(IdpSyncConfig {
        provider: provider.clone(),
        sync_interval_seconds: std::env::var("AETERNA_IDP_SYNC_INTERVAL_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(300),
        batch_size: std::env::var("AETERNA_IDP_BATCH_SIZE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(100),
        database_url,
        webhook_port,
        webhook_secret,
        dry_run: std::env::var("AETERNA_IDP_DRY_RUN")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false),
        retry: idp_sync::config::RetryConfig {
            max_retries: 3,
            initial_backoff_ms: 1000,
            max_backoff_ms: 30000,
        },
    });

    let client: Arc<dyn IdpClient> = match &config.provider {
        IdpProvider::Okta(okta) => Arc::new(OktaClient::new(okta.clone())?),
        IdpProvider::AzureAd(azure) => Arc::new(AzureAdClient::new(azure.clone())?),
        IdpProvider::GitHub(_) => {
            unreachable!("GitHub provider is handled by the dedicated admin sync endpoint")
        }
    };

    let sync_service = Arc::new(IdpSyncService::new(
        (*config).clone(),
        client.clone(),
        postgres.pool().clone(),
    ));

    Ok((Some(config), Some(client), Some(sync_service)))
}

struct AnyhowAuthWrapper<S> {
    inner: S,
}

#[async_trait]
impl<S> AuthorizationService for AnyhowAuthWrapper<S>
where
    S: AuthorizationService + Send + Sync,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    type Error = anyhow::Error;

    async fn check_permission(
        &self,
        ctx: &TenantContext,
        action: &str,
        resource: &str,
    ) -> Result<bool, Self::Error> {
        self.inner
            .check_permission(ctx, action, resource)
            .await
            .map_err(anyhow::Error::from)
    }

    async fn get_user_roles(
        &self,
        ctx: &TenantContext,
    ) -> Result<Vec<RoleIdentifier>, Self::Error> {
        self.inner
            .get_user_roles(ctx)
            .await
            .map_err(anyhow::Error::from)
    }

    async fn assign_role(
        &self,
        ctx: &TenantContext,
        user_id: &UserId,
        role: RoleIdentifier,
    ) -> Result<(), Self::Error> {
        self.inner
            .assign_role(ctx, user_id, role)
            .await
            .map_err(anyhow::Error::from)
    }

    async fn remove_role(
        &self,
        ctx: &TenantContext,
        user_id: &UserId,
        role: RoleIdentifier,
    ) -> Result<(), Self::Error> {
        self.inner
            .remove_role(ctx, user_id, role)
            .await
            .map_err(anyhow::Error::from)
    }
}

struct AllowAllAuthService;

#[async_trait]
impl AuthorizationService for AllowAllAuthService {
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

struct AllowAllBoxedAuthService;

#[async_trait]
impl AuthorizationService for AllowAllBoxedAuthService {
    type Error = Box<dyn std::error::Error + Send + Sync>;

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

struct NoopReflectiveReasoner;

#[async_trait]
impl ReflectiveReasoner for NoopReflectiveReasoner {
    async fn reason(
        &self,
        query: &str,
        _context_summary: Option<&str>,
    ) -> anyhow::Result<ReasoningTrace> {
        let now = Utc::now();
        Ok(ReasoningTrace {
            strategy: ReasoningStrategy::SemanticOnly,
            thought_process: "Reflective reasoning disabled".to_string(),
            refined_query: Some(query.to_string()),
            start_time: now,
            end_time: now,
            timed_out: false,
            duration_ms: 0,
            metadata: Default::default(),
        })
    }
}

struct AllowAllTokenValidator {
    access_token_ttl_seconds: u64,
}

#[async_trait]
impl TokenValidator for AllowAllTokenValidator {
    async fn validate(&self, token: &str) -> WsResult<AuthToken> {
        Ok(AuthToken {
            user_id: token.to_string(),
            tenant_id: mk_core::types::DEFAULT_TENANT_SLUG.to_string(),
            permissions: vec!["read".to_string(), "write".to_string()],
            expires_at: Utc::now().timestamp() + self.access_token_ttl_seconds as i64,
        })
    }
}

async fn seed_platform_admin(
    pool: &sqlx::Pool<sqlx::Postgres>,
    cfg: &config::AdminBootstrapConfig,
) -> anyhow::Result<()> {
    let email = if let Some(e) = &cfg.email {
        e
    } else {
        tracing::warn!("admin bootstrap skipped: AETERNA_ADMIN_BOOTSTRAP_EMAIL not set");
        return Ok(());
    };

    let provider = &cfg.provider;
    let subject = cfg.provider_subject.as_deref().unwrap_or(email.as_str());
    let now_ts = chrono::Utc::now();

    let mut tx = pool.begin().await.context("begin seed transaction")?;

    sqlx::query(
        "INSERT INTO organizational_units (id, name, type, parent_id, tenant_id, metadata, created_at, updated_at)
         VALUES ($1, $2, 'organization', NULL, $1, '{}', $3, $3)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(INSTANCE_SCOPE_TENANT_ID)
    .bind("Instance")
    .bind(now_ts)
    .execute(&mut *tx)
    .await
    .context("upsert instance-scope organizational unit")?;

    sqlx::query(
        "UPDATE user_roles
         SET tenant_id = $1, unit_id = $1
         WHERE role = $2 AND tenant_id = $3",
    )
    .bind(INSTANCE_SCOPE_TENANT_ID)
    .bind(Role::PlatformAdmin.to_string())
    .bind(DEFAULT_TENANT_SLUG)
    .execute(&mut *tx)
    .await
    .context("migrate legacy PlatformAdmin rows to instance scope")?;

    let tenant_slug = cfg.tenant_slug.as_str();

    // Ensure a `tenants` row (migration 017 schema) exists before writing
    // legacy wrapper rows. Bootstrap has historically written
    // `organizational_units` (legacy TEXT-id table) and the old UUID-root
    // hierarchy table but never the canonical `tenants` table. That was
    // tolerable while no foreign key connected the old root table -> tenants,
    // but §2.2-B (see
    // openspec/changes/harden-tenant-provisioning/
    // NOTES-hierarchy-migration-blast-radius.md) will add exactly that FK,
    // so the invariant "bootstrap leaves a tenants row matching the
    // tenant slug" needs to hold starting now. Idempotent via
    // ON CONFLICT (slug); no-op on repeat bootstraps.
    sqlx::query(
        "INSERT INTO tenants (slug, name, status, source_owner)
         VALUES ($1, $2, 'active', 'admin')
         ON CONFLICT (slug) DO NOTHING",
    )
    .bind(tenant_slug)
    .bind("Default")
    .execute(&mut *tx)
    .await
    .context("upsert bootstrap tenants row")?;

    let bootstrap_tenant_uuid: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM tenants WHERE slug = $1")
            .bind(tenant_slug)
            .fetch_one(&mut *tx)
            .await
            .context("fetch bootstrap tenant uuid")?;

    sqlx::query(
        "INSERT INTO organizational_units (id, name, type, parent_id, tenant_id, metadata, created_at, updated_at)
         VALUES ($1, $2, 'organization', NULL, $3, '{}', $4, $4)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(cfg.org_slug.as_str())
    .bind("Platform")
    .bind(tenant_slug)
    .bind(now_ts)
    .execute(&mut *tx)
    .await
    .context("upsert organizational_units organization")?;

    sqlx::query(
        "INSERT INTO organizations (tenant_id, slug, name, created_at, updated_at)
         VALUES ($1, $2, 'Platform', NOW(), NOW())
         ON CONFLICT (tenant_id, slug)
         DO UPDATE SET updated_at = NOW()",
    )
    .bind(bootstrap_tenant_uuid)
    .bind(cfg.org_slug.as_str())
    .execute(&mut *tx)
    .await
    .context("upsert bootstrap organization")?;

    let org_uuid: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM organizations WHERE tenant_id = $1 AND slug = $2")
            .bind(bootstrap_tenant_uuid)
            .bind(cfg.org_slug.as_str())
            .fetch_one(&mut *tx)
            .await
            .context("fetch org uuid")?;

    sqlx::query(
        "INSERT INTO teams (org_id, slug, name, created_at, updated_at)
         VALUES ($1, $2, 'Admins', NOW(), NOW())
         ON CONFLICT (org_id, slug) DO NOTHING",
    )
    .bind(org_uuid)
    .bind(cfg.team_slug.as_str())
    .execute(&mut *tx)
    .await
    .context("upsert bootstrap team")?;

    let team_uuid: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM teams WHERE org_id = $1 AND slug = $2")
            .bind(org_uuid)
            .bind(cfg.team_slug.as_str())
            .fetch_one(&mut *tx)
            .await
            .context("fetch team uuid")?;

    let user_uuid: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, name, idp_provider, idp_subject, status, created_at, updated_at)
         VALUES ($1, $1, $2, $3, 'active', NOW(), NOW())
         ON CONFLICT (idp_provider, idp_subject) DO UPDATE SET email = EXCLUDED.email, name = EXCLUDED.name, updated_at = NOW()
         RETURNING id",
    )
    .bind(email)
    .bind(provider)
    .bind(subject)
    .fetch_one(&mut *tx)
    .await
    .context("upsert admin user")?;

    sqlx::query(
        "INSERT INTO memberships (user_id, team_id, role, status, created_at, updated_at)
         VALUES ($1, $2, 'admin', 'active', NOW(), NOW())
         ON CONFLICT (user_id, team_id) DO NOTHING",
    )
    .bind(user_uuid)
    .bind(team_uuid)
    .execute(&mut *tx)
    .await
    .context("upsert membership")?;

    let user_id_str = user_uuid.to_string();
    sqlx::query(
        "INSERT INTO user_roles (user_id, tenant_id, unit_id, role, created_at)
         VALUES ($1, $2, $3, 'PlatformAdmin', $4)
         ON CONFLICT (user_id, tenant_id, unit_id, role) DO NOTHING",
    )
    .bind(&user_id_str)
    .bind(INSTANCE_SCOPE_TENANT_ID)
    .bind(INSTANCE_SCOPE_TENANT_ID)
    .bind(now_ts)
    .execute(&mut *tx)
    .await
    .context("upsert PlatformAdmin role")?;

    tx.commit().await.context("commit seed transaction")?;

    tracing::info!(
        email = %email,
        provider = %provider,
        subject = %subject,
        "platform admin seeded successfully"
    );

    Ok(())
}

/// Seeds a Kubernetes service account identity as a PlatformAdmin.
///
/// Idempotent — safe to call on every server restart.  Only runs when
/// `AdminBootstrapConfig::k8s_sa_subject` is configured.
async fn seed_k8s_service_account(
    pool: &sqlx::Pool<sqlx::Postgres>,
    cfg: &config::AdminBootstrapConfig,
) -> anyhow::Result<()> {
    let sa_subject = match &cfg.k8s_sa_subject {
        Some(s) => s,
        None => return Ok(()),
    };

    let now_ts_k8s = chrono::Utc::now();
    let synthetic_email = format!("k8s+{}@local", sa_subject.replace([':', '/'], "."));

    let mut tx = pool
        .begin()
        .await
        .context("begin k8s SA seed transaction")?;

    sqlx::query(
        "INSERT INTO organizational_units (id, name, type, parent_id, tenant_id, metadata, created_at, updated_at)
         VALUES ($1, $2, 'organization', NULL, $1, '{}', $3, $3)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(INSTANCE_SCOPE_TENANT_ID)
    .bind("Instance")
    .bind(now_ts_k8s)
    .execute(&mut *tx)
    .await
    .context("upsert instance-scope organizational unit for k8s SA")?;

    let user_uuid: uuid::Uuid = sqlx::query_scalar(
        "WITH inserted AS (
             INSERT INTO users (email, name, idp_provider, idp_subject, status, created_at, updated_at)
             VALUES ($1, $2, 'kubernetes', $3, 'active', NOW(), NOW())
             ON CONFLICT (idp_provider, idp_subject) DO NOTHING
             RETURNING id
         )
         SELECT id FROM inserted
         UNION ALL
         SELECT id FROM users WHERE idp_provider = 'kubernetes' AND idp_subject = $3
         LIMIT 1",
    )
    .bind(&synthetic_email)
    .bind(sa_subject)
    .bind(sa_subject)
    .fetch_one(&mut *tx)
    .await
    .context("upsert kubernetes service account user")?;

    sqlx::query(
        "INSERT INTO user_roles (user_id, tenant_id, unit_id, role, created_at)
         VALUES ($1, $2, $3, 'PlatformAdmin', $4)
         ON CONFLICT (user_id, tenant_id, unit_id, role) DO NOTHING",
    )
    .bind(user_uuid.to_string())
    .bind(INSTANCE_SCOPE_TENANT_ID)
    .bind(INSTANCE_SCOPE_TENANT_ID)
    .bind(now_ts_k8s)
    .execute(&mut *tx)
    .await
    .context("upsert PlatformAdmin role for kubernetes service account")?;

    tx.commit()
        .await
        .context("commit k8s service account seed transaction")?;

    tracing::info!(
        subject = %sa_subject,
        "kubernetes service account PlatformAdmin seeded successfully"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mk_core::traits::GitProviderConnectionRegistry;
    use serial_test::serial;

    fn set_env<K: AsRef<str>, V: AsRef<str>>(key: K, value: V) {
        unsafe { std::env::set_var(key.as_ref(), value.as_ref()) }
    }

    fn remove_env<K: AsRef<str>>(key: K) {
        unsafe { std::env::remove_var(key.as_ref()) }
    }

    #[test]
    #[serial]
    fn validate_required_env_accepts_pg_prefixes() {
        unsafe {
            std::env::set_var("PG_HOST", "localhost");
            std::env::set_var("PG_DATABASE", "aeterna");
            std::env::set_var("RD_HOST", "localhost");
        }

        assert!(validate_required_env().is_ok());

        unsafe {
            std::env::remove_var("PG_HOST");
            std::env::remove_var("PG_DATABASE");
            std::env::remove_var("RD_HOST");
        }
    }

    #[tokio::test]
    async fn noop_reasoner_returns_semantic_only_trace() {
        let trace = NoopReflectiveReasoner.reason("hello", None).await.unwrap();
        assert_eq!(trace.strategy, ReasoningStrategy::SemanticOnly);
        assert_eq!(trace.refined_query.as_deref(), Some("hello"));
    }

    #[test]
    fn knowledge_repo_path_uses_default_when_env_missing() {
        unsafe {
            std::env::remove_var("AETERNA_KNOWLEDGE_REPO_PATH");
        }
        assert_eq!(knowledge_repo_path(), PathBuf::from("./knowledge-repo"));
    }

    #[test]
    #[serial]
    fn tenant_config_provider_namespace_uses_env_override() {
        set_env("AETERNA_K8S_NAMESPACE", "aeterna");
        assert_eq!(tenant_config_provider_namespace(), "aeterna");
        remove_env("AETERNA_K8S_NAMESPACE");
    }

    #[test]
    #[serial]
    fn tenant_config_provider_namespace_defaults_to_default() {
        remove_env("AETERNA_K8S_NAMESPACE");
        assert_eq!(tenant_config_provider_namespace(), "default");
    }

    #[test]
    fn feature_enabled_respects_default_and_env() {
        remove_env("AETERNA_FEATURE_REFLECTIVE");
        assert!(feature_enabled("AETERNA_FEATURE_REFLECTIVE", true));
        assert!(!feature_enabled("AETERNA_FEATURE_REFLECTIVE", false));

        set_env("AETERNA_FEATURE_REFLECTIVE", "false");
        assert!(!feature_enabled("AETERNA_FEATURE_REFLECTIVE", true));

        set_env("AETERNA_FEATURE_REFLECTIVE", "true");
        assert!(feature_enabled("AETERNA_FEATURE_REFLECTIVE", false));

        remove_env("AETERNA_FEATURE_REFLECTIVE");
    }

    #[test]
    #[serial]
    fn feature_enabled_accepts_additional_truthy_variants() {
        for value in ["1", "TRUE", "yes", "on"] {
            set_env("AETERNA_FEATURE_REFLECTIVE", value);
            assert!(feature_enabled("AETERNA_FEATURE_REFLECTIVE", false));
        }
        remove_env("AETERNA_FEATURE_REFLECTIVE");
    }

    #[test]
    #[serial]
    fn runtime_features_reads_all_feature_flags() {
        set_env("AETERNA_FEATURE_CCA", "false");
        set_env("AETERNA_FEATURE_RADKIT", "true");
        set_env("AETERNA_FEATURE_RLM", "0");
        set_env("AETERNA_FEATURE_REFLECTIVE", "1");

        let features = RuntimeFeatures::from_env();
        assert_eq!(
            features,
            RuntimeFeatures {
                cca: false,
                radkit: true,
                rlm: false,
                reflective: true,
            }
        );

        remove_env("AETERNA_FEATURE_CCA");
        remove_env("AETERNA_FEATURE_RADKIT");
        remove_env("AETERNA_FEATURE_RLM");
        remove_env("AETERNA_FEATURE_REFLECTIVE");
    }

    #[test]
    fn postgres_connection_url_uses_provider_config_values() {
        let mut config = config::Config::default();
        config.providers.postgres.host = "db.internal".to_string();
        config.providers.postgres.port = 5433;
        config.providers.postgres.database = "aeterna".to_string();
        config.providers.postgres.username = "svc".to_string();
        config.providers.postgres.password = "example-password".to_string();

        assert_eq!(
            postgres_connection_url(&config),
            "postgres://svc:example-password@db.internal:5433/aeterna"
        );
    }

    #[test]
    fn create_graph_store_returns_none_when_graph_disabled() {
        let mut config = config::Config::default();
        config.providers.graph.enabled = false;

        let graph_store = create_graph_store(&config).unwrap();
        assert!(graph_store.is_none());
    }

    #[test]
    fn create_graph_store_passes_snapshot_policy() {
        let mut config = config::Config::default();
        config.providers.graph.database_path = ":memory:".to_string();
        config.providers.graph.snapshot_full_interval_secs = 1234;
        config.providers.graph.snapshot_delta_interval_secs = 56;

        let graph_store = create_graph_store(&config)
            .unwrap()
            .expect("graph store should be enabled");

        assert_eq!(graph_store.snapshot_policy().full_interval_secs, 1234);
        assert_eq!(graph_store.snapshot_policy().delta_interval_secs, 56);
    }

    #[tokio::test]
    #[serial]
    async fn build_anyhow_auth_service_defaults_to_allow_all() {
        remove_env("AETERNA_AUTH_BACKEND");

        let auth = build_anyhow_auth_service().unwrap();
        let allowed = auth
            .check_permission(&TenantContext::default(), "read", "resource")
            .await
            .unwrap();
        assert!(allowed);
    }

    #[tokio::test]
    #[serial]
    async fn build_anyhow_auth_service_ignores_plugin_auth_env() {
        remove_env("AETERNA_AUTH_BACKEND");
        set_env("AETERNA_PLUGIN_AUTH_ENABLED", "true");
        set_env("AETERNA_PLUGIN_AUTH_JWT_SECRET", "plugin-secret");

        let auth = build_anyhow_auth_service().unwrap();
        let allowed = auth
            .check_permission(&TenantContext::default(), "read", "resource")
            .await
            .unwrap();
        assert!(allowed);

        remove_env("AETERNA_PLUGIN_AUTH_ENABLED");
        remove_env("AETERNA_PLUGIN_AUTH_JWT_SECRET");
    }

    #[test]
    #[serial]
    fn build_anyhow_auth_service_errors_for_missing_cedar_inputs() {
        set_env("AETERNA_AUTH_BACKEND", "cedar");
        remove_env("AETERNA_CEDAR_POLICY_TEXT");
        remove_env("AETERNA_CEDAR_SCHEMA_TEXT");

        let error = build_anyhow_auth_service().err().unwrap();
        assert!(
            error
                .to_string()
                .contains("AETERNA_CEDAR_POLICY_TEXT is required for cedar auth")
        );

        remove_env("AETERNA_AUTH_BACKEND");
    }

    #[tokio::test]
    #[serial]
    async fn build_optional_idp_services_returns_none_when_provider_missing() {
        remove_env("AETERNA_IDP_PROVIDER");

        let lazy_pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://postgres:postgres@localhost:5432/aeterna")
            .unwrap();
        let postgres = Arc::new(PostgresBackend::from_pool(lazy_pool));

        let (config, client, service) = build_optional_idp_services(postgres).unwrap();
        assert!(config.is_none());
        assert!(client.is_none());
        assert!(service.is_none());
    }

    #[test]
    fn github_app_bootstrap_uses_knowledge_repo_fields_not_plugin_auth_fields() {
        let mut config = config::Config::default();
        config.knowledge_repo.github_owner = Some("acme".to_string());
        config.knowledge_repo.github_repo = Some("knowledge".to_string());
        config.knowledge_repo.github_app_id = Some(101);
        config.knowledge_repo.github_installation_id = Some(202);
        config.knowledge_repo.github_app_pem = Some("knowledge-pem".to_string());

        config.plugin_auth.enabled = true;
        config.plugin_auth.github_app_id = Some(999);
        config.plugin_auth.github_app_pem = Some("plugin-pem".to_string());

        assert_eq!(config.knowledge_repo.github_app_id, Some(101));
        assert_eq!(config.knowledge_repo.github_installation_id, Some(202));
        assert_eq!(
            config.knowledge_repo.github_app_pem.as_deref(),
            Some("knowledge-pem")
        );
        assert_eq!(config.plugin_auth.github_app_id, Some(999));
        assert_eq!(
            config.plugin_auth.github_app_pem.as_deref(),
            Some("plugin-pem")
        );
        assert_ne!(
            config.knowledge_repo.github_app_pem,
            config.plugin_auth.github_app_pem
        );
    }

    fn write_seed_file(body: serde_json::Value) -> tempfile::TempPath {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&body).unwrap()).unwrap();
        file.into_temp_path()
    }

    #[tokio::test]
    async fn seed_git_provider_connections_creates_missing_connection_and_allowlist() {
        let store = InMemoryGitProviderConnectionStore::new();
        let seed_path = write_seed_file(serde_json::json!([
            {
                "id": "shared-github-app",
                "name": "Shared GitHub App",
                "providerKind": "GitHubApp",
                "appId": 1234567,
                "installationId": 7654321,
                "pemSecretRef": "secret/aeterna-github-app-pem/pem-key",
                "webhookSecretRef": "secret/aeterna-github-app-webhook/webhook-secret",
                "allowedTenantIds": ["tenant-alpha", "tenant-beta"]
            }
        ]));

        seed_git_provider_connections_from_path(&store, &seed_path)
            .await
            .unwrap();

        let conn = store
            .get_connection("shared-github-app")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(conn.name, "Shared GitHub App");
        assert_eq!(conn.allowed_tenant_ids.len(), 2);
        assert!(
            conn.allowed_tenant_ids
                .contains(&TenantId::new("tenant-alpha".to_string()).unwrap())
        );
        assert!(
            conn.allowed_tenant_ids
                .contains(&TenantId::new("tenant-beta".to_string()).unwrap())
        );
    }

    #[tokio::test]
    async fn seed_git_provider_connections_preserves_existing_runtime_allowlist() {
        let store = InMemoryGitProviderConnectionStore::new();
        let now = Utc::now();
        store
            .create_connection(GitProviderConnection {
                id: "shared-github-app".to_string(),
                name: "Shared GitHub App".to_string(),
                provider_kind: GitProviderKind::GitHubApp,
                app_id: 1234567,
                installation_id: 7654321,
                pem_secret_ref: "secret/aeterna-github-app-pem/pem-key".to_string(),
                webhook_secret_ref: Some(
                    "secret/aeterna-github-app-webhook/webhook-secret".to_string(),
                ),
                allowed_tenant_ids: vec![TenantId::new("legacy-tenant".to_string()).unwrap()],
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let seed_path = write_seed_file(serde_json::json!([
            {
                "id": "shared-github-app",
                "name": "Shared GitHub App",
                "providerKind": "GitHubApp",
                "appId": 1234567,
                "installationId": 7654321,
                "pemSecretRef": "secret/aeterna-github-app-pem/pem-key",
                "webhookSecretRef": "secret/aeterna-github-app-webhook/webhook-secret",
                "allowedTenantIds": ["tenant-alpha"]
            }
        ]));

        seed_git_provider_connections_from_path(&store, &seed_path)
            .await
            .unwrap();

        let conn = store
            .get_connection("shared-github-app")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            conn.allowed_tenant_ids,
            vec![TenantId::new("legacy-tenant".to_string()).unwrap()]
        );
    }

    #[tokio::test]
    async fn seed_git_provider_connections_fails_on_metadata_drift() {
        let store = InMemoryGitProviderConnectionStore::new();
        let now = Utc::now();
        store
            .create_connection(GitProviderConnection {
                id: "shared-github-app".to_string(),
                name: "Shared GitHub App".to_string(),
                provider_kind: GitProviderKind::GitHubApp,
                app_id: 1,
                installation_id: 7654321,
                pem_secret_ref: "secret/aeterna-github-app-pem/pem-key".to_string(),
                webhook_secret_ref: None,
                allowed_tenant_ids: vec![],
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let seed_path = write_seed_file(serde_json::json!([
            {
                "id": "shared-github-app",
                "name": "Shared GitHub App",
                "providerKind": "GitHubApp",
                "appId": 1234567,
                "installationId": 7654321,
                "pemSecretRef": "secret/aeterna-github-app-pem/pem-key",
                "webhookSecretRef": null,
                "allowedTenantIds": []
            }
        ]));

        let err = seed_git_provider_connections_from_path(&store, &seed_path)
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("conflicts with existing registry metadata")
        );
    }
}
