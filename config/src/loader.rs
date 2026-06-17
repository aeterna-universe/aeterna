//! # Environment Variable Loader
//!
//! Loads configuration from environment variables following 12-factor app
//! principles.
//!
//! # Naming Convention
//! - `MK_*`: Memory-related settings
//! - `KK_*`: Knowledge-related settings
//! - `SY_*`: Sync-related settings
//! - `TL_*`: Tool-related settings
//! - `PG_*`: PostgreSQL settings
//! - `QD_*`: Qdrant settings
//! - `RD_*`: Redis settings
//! - `OB_*`: Observability settings

use crate::config::{
    Config, ContentionAlertConfig, GraphConfig, KnowledgeRepoConfig, KubernetesAuthConfig,
    MemoryConfig, ObservabilityConfig, PluginAuthConfig, PostgresConfig, ProviderConfig,
    QdrantConfig, ReasoningConfig, RedisConfig, RlmConfig, SyncConfig, ToolConfig,
};
use mk_core::types::PROVIDER_GITHUB;
use std::env;

/// Load configuration from environment variables.
///
/// # M-CANONICAL-DOCS
///
/// ## Purpose
/// Loads configuration from environment variables following 12-factor app
/// principles. Environment variables override default values but can be
/// overridden by CLI arguments.
///
/// ## Usage
/// ```rust,no_run
/// use config::load_from_env;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let config = load_from_env()?;
///     println!("PostgreSQL host: {}", config.providers.postgres.host);
///     Ok(())
/// }
/// ```
///
/// ## Environment Variables
/// ### General Settings
/// - `MK_LOG_LEVEL`: Logging level (trace/debug/info/warn/error)
///
/// ### PostgreSQL Settings (`PG_*`)
/// - `PG_HOST`: Database host (default: "localhost")
/// - `PG_PORT`: Database port (default: 5432)
/// - `PG_DATABASE`: Database name (default: "memory_knowledge")
/// - `PG_USERNAME`: Database user (default: "postgres")
/// - `PG_PASSWORD`: Database password (default: "")
/// - `PG_POOL_SIZE`: Connection pool size (default: 10)
/// - `PG_TIMEOUT_SECONDS`: Connection timeout in seconds (default: 30)
///
/// ### Qdrant Settings (`QD_*`)
/// - `QD_HOST`: Qdrant host (default: "localhost")
/// - `QD_PORT`: Qdrant port (default: 6333)
/// - `QD_COLLECTION`: Collection name (default: "memory_embeddings")
/// - `QD_TIMEOUT_SECONDS`: Request timeout in seconds (default: 30)
///
/// ### Redis Settings (`RD_*`)
/// - `RD_HOST`: Redis host (default: "localhost")
/// - `RD_PORT`: Redis port (default: 6379)
/// - `RD_DB`: Redis database number (default: 0)
/// - `RD_POOL_SIZE`: Connection pool size (default: 10)
/// - `RD_TIMEOUT_SECONDS`: Connection timeout in seconds (default: 30)
///
/// ### Sync Settings (`SY_*`)
/// - `SY_ENABLED`: Enable sync (true/false, default: true)
/// - `SY_SYNC_INTERVAL_SECONDS`: Sync interval (default: 60)
/// - `SY_BATCH_SIZE`: Batch size (default: 100)
/// - `SY_CHECKPOINT_ENABLED`: Enable checkpointing (true/false, default: true)
/// - `SY_CONFLICT_RESOLUTION`: Conflict resolution
///   (prefer_knowledge/prefer_memory/manual, default: prefer_knowledge)
///
/// ### Tools Settings (`TL_*`)
/// - `TL_ENABLED`: Enable MCP server (true/false, default: true)
/// - `TL_HOST`: Server host (default: "localhost")
/// - `TL_PORT`: Server port (default: 8080)
/// - `TL_API_KEY`: API key for authentication (optional)
/// - `TL_RATE_LIMIT_REQUESTS_PER_MINUTE`: Rate limit (default: 60)
///
/// ### Observability Settings (`OB_*`)
/// - `OB_METRICS_ENABLED`: Enable metrics (true/false, default: true)
/// - `OB_TRACING_ENABLED`: Enable tracing (true/false, default: true)
/// - `OB_LOGGING_LEVEL`: Logging level (trace/debug/info/warn/error, default:
///   "info")
/// - `OB_METRICS_PORT`: Metrics server port (default: 9090)
pub fn load_from_env() -> anyhow::Result<Config> {
    let config = Config {
        providers: load_provider_from_env()?,
        sync: load_sync_from_env()?,
        memory: load_memory_from_env()?,
        tools: load_tools_from_env()?,
        observability: load_observability_from_env()?,
        deployment: load_deployment_from_env()?,
        job: load_job_from_env()?,
        knowledge_repo: load_knowledge_repo_from_env()?,
        plugin_auth: load_plugin_auth_from_env()?,
        k8s_auth: load_k8s_auth_from_env()?,
        admin_bootstrap: load_admin_bootstrap_from_env()?,
        cca: crate::cca::CcaConfig::default(),
        provisioning: crate::ProvisioningConfig::default(),
    };

    Ok(config)
}

fn load_deployment_from_env() -> anyhow::Result<crate::config::DeploymentConfig> {
    Ok(crate::config::DeploymentConfig {
        mode: env::var("AETERNA_DEPLOYMENT_MODE").unwrap_or_else(|_| "local".to_string()),
        remote_url: env::var("AETERNA_REMOTE_GOVERNANCE_URL").ok(),
        sync_enabled: parse_env("AETERNA_SYNC_ENABLED").unwrap_or(true),
    })
}

fn load_job_from_env() -> anyhow::Result<crate::config::JobConfig> {
    Ok(crate::config::JobConfig {
        lock_ttl_seconds: parse_env("AETERNA_JOB_LOCK_TTL_SECONDS").unwrap_or(2100),
        job_timeout_seconds: parse_env("AETERNA_JOB_TIMEOUT_SECONDS").unwrap_or(1800),
        deduplication_window_seconds: parse_env("AETERNA_JOB_DEDUP_WINDOW_SECONDS").unwrap_or(300),
        checkpoint_interval: parse_env("AETERNA_JOB_CHECKPOINT_INTERVAL").unwrap_or(100),
        graceful_shutdown_timeout_seconds: parse_env("AETERNA_JOB_SHUTDOWN_TIMEOUT_SECONDS")
            .unwrap_or(30),
    })
}

fn load_knowledge_repo_from_env() -> anyhow::Result<KnowledgeRepoConfig> {
    Ok(KnowledgeRepoConfig {
        remote_url: env::var("AETERNA_KNOWLEDGE_REPO_URL").ok(),
        branch: env::var("AETERNA_KNOWLEDGE_REPO_BRANCH").unwrap_or_else(|_| "main".to_string()),
        github_owner: env::var("AETERNA_GITHUB_OWNER").ok(),
        github_repo: env::var("AETERNA_GITHUB_REPO").ok(),
        github_token: env::var("AETERNA_GITHUB_TOKEN").ok(),
        github_app_id: env::var("AETERNA_GITHUB_APP_ID")
            .ok()
            .and_then(|v| v.parse::<u64>().ok()),
        github_installation_id: env::var("AETERNA_GITHUB_INSTALLATION_ID")
            .ok()
            .and_then(|v| v.parse::<u64>().ok()),
        github_app_pem: env::var("AETERNA_GITHUB_APP_PEM").ok(),
        webhook_secret: env::var("AETERNA_WEBHOOK_SECRET").ok(),
    })
}

fn load_plugin_auth_from_env() -> anyhow::Result<PluginAuthConfig> {
    Ok(PluginAuthConfig {
        enabled: parse_env("AETERNA_PLUGIN_AUTH_ENABLED").unwrap_or(false),
        allowed_providers: env::var("AETERNA_PLUGIN_ALLOWED_PROVIDERS")
            .ok()
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|providers| !providers.is_empty())
            .unwrap_or_else(|| vec![PROVIDER_GITHUB.to_string()]),
        github_client_id: env::var("AETERNA_PLUGIN_AUTH_GITHUB_CLIENT_ID").ok(),
        github_client_secret: env::var("AETERNA_PLUGIN_AUTH_GITHUB_CLIENT_SECRET").ok(),
        github_api_base_url: env::var("AETERNA_GITHUB_API_BASE_URL").ok(),
        github_oauth_base_url: env::var("AETERNA_GITHUB_OAUTH_BASE_URL").ok(),
        github_app_id: env::var("AETERNA_PLUGIN_AUTH_GITHUB_APP_ID")
            .ok()
            .and_then(|v| v.parse::<u64>().ok()),
        github_app_name: env::var("AETERNA_PLUGIN_AUTH_GITHUB_APP_NAME").ok(),
        github_app_pem: env::var("AETERNA_PLUGIN_AUTH_GITHUB_APP_PEM").ok(),
        redirect_base_url: env::var("AETERNA_PLUGIN_AUTH_REDIRECT_BASE_URL").ok(),
        token_issuer: env::var("AETERNA_PLUGIN_AUTH_TOKEN_ISSUER").ok(),
        jwt_secret: env::var("AETERNA_PLUGIN_AUTH_JWT_SECRET").ok(),
        access_token_ttl_seconds: env::var("AETERNA_PLUGIN_AUTH_ACCESS_TOKEN_TTL_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok()),
        refresh_token_ttl_seconds: env::var("AETERNA_PLUGIN_AUTH_REFRESH_TOKEN_TTL_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok()),
        default_tenant_id: env::var("AETERNA_DEFAULT_TENANT_ID").ok(),
    })
}

fn load_k8s_auth_from_env() -> anyhow::Result<KubernetesAuthConfig> {
    Ok(KubernetesAuthConfig {
        enabled: parse_env("AETERNA_K8S_AUTH_ENABLED").unwrap_or(false),
        api_server_url: env::var("AETERNA_K8S_AUTH_API_SERVER_URL").ok(),
        token_review_audience: env::var("AETERNA_K8S_AUTH_TOKEN_REVIEW_AUDIENCE").ok(),
        sa_token_path: env::var("AETERNA_K8S_AUTH_SA_TOKEN_PATH").ok(),
        ca_bundle_path: env::var("AETERNA_K8S_AUTH_CA_BUNDLE_PATH").ok(),
        namespace: env::var("AETERNA_K8S_NAMESPACE").ok(),
    })
}

fn load_admin_bootstrap_from_env() -> anyhow::Result<crate::config::AdminBootstrapConfig> {
    Ok(crate::config::AdminBootstrapConfig {
        email: env::var("AETERNA_ADMIN_BOOTSTRAP_EMAIL").ok(),
        provider: env::var("AETERNA_ADMIN_BOOTSTRAP_PROVIDER")
            .unwrap_or_else(|_| PROVIDER_GITHUB.to_string()),
        provider_subject: env::var("AETERNA_ADMIN_BOOTSTRAP_PROVIDER_SUBJECT").ok(),
        tenant_slug: env::var("AETERNA_BOOTSTRAP_TENANT_SLUG")
            .unwrap_or_else(|_| "default".to_string()),
        org_slug: env::var("AETERNA_BOOTSTRAP_ORG_SLUG").unwrap_or_else(|_| "platform".to_string()),
        team_slug: env::var("AETERNA_BOOTSTRAP_TEAM_SLUG").unwrap_or_else(|_| "admins".to_string()),
        k8s_sa_subject: env::var("AETERNA_ADMIN_BOOTSTRAP_K8S_SA_SUBJECT").ok(),
    })
}

fn load_provider_from_env() -> anyhow::Result<ProviderConfig> {
    Ok(ProviderConfig {
        postgres: load_postgres_from_env()?,
        qdrant: load_qdrant_from_env()?,
        redis: load_redis_from_env()?,
        graph: load_graph_from_env()?,
    })
}

fn load_postgres_from_env() -> anyhow::Result<PostgresConfig> {
    Ok(PostgresConfig {
        host: env::var("PG_HOST").unwrap_or_else(|_| "localhost".to_string()),
        port: parse_env("PG_PORT").unwrap_or(5432),
        database: env::var("PG_DATABASE").unwrap_or_else(|_| "memory_knowledge".to_string()),
        username: env::var("PG_USERNAME").unwrap_or_else(|_| "postgres".to_string()),
        password: env::var("PG_PASSWORD").unwrap_or_default(),
        pool_size: parse_env("PG_POOL_SIZE").unwrap_or(10),
        timeout_seconds: parse_env("PG_TIMEOUT_SECONDS").unwrap_or(30),
    })
}

fn load_qdrant_from_env() -> anyhow::Result<QdrantConfig> {
    Ok(QdrantConfig {
        host: env::var("QD_HOST").unwrap_or_else(|_| "localhost".to_string()),
        port: parse_env("QD_PORT").unwrap_or(6333),
        collection: env::var("QD_COLLECTION").unwrap_or_else(|_| "memory_embeddings".to_string()),
        timeout_seconds: parse_env("QD_TIMEOUT_SECONDS").unwrap_or(30),
    })
}

fn load_redis_from_env() -> anyhow::Result<RedisConfig> {
    Ok(RedisConfig {
        host: env::var("RD_HOST").unwrap_or_else(|_| "localhost".to_string()),
        port: parse_env("RD_PORT").unwrap_or(6379),
        db: parse_env("RD_DB").unwrap_or(0),
        pool_size: parse_env("RD_POOL_SIZE").unwrap_or(10),
        timeout_seconds: parse_env("RD_TIMEOUT_SECONDS").unwrap_or(30),
    })
}

fn load_graph_from_env() -> anyhow::Result<GraphConfig> {
    Ok(GraphConfig {
        enabled: parse_env("GR_ENABLED").unwrap_or(true),
        database_path: env::var("GR_DATABASE_PATH").unwrap_or_else(|_| ":memory:".to_string()),
        s3_bucket: env::var("GR_S3_BUCKET").ok(),
        s3_prefix: env::var("GR_S3_PREFIX").ok(),
        s3_endpoint: env::var("GR_S3_ENDPOINT").ok(),
        s3_region: env::var("GR_S3_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
        reader_pool_size: parse_env("GR_READER_POOL_SIZE").ok(),
        snapshot_full_interval_secs: parse_env("GR_SNAPSHOT_FULL_INTERVAL_SECS").unwrap_or(604800),
        snapshot_delta_interval_secs: parse_env("GR_SNAPSHOT_DELTA_INTERVAL_SECS").unwrap_or(300),
        event_sourcing_enabled: parse_env("GR_EVENT_SOURCING_ENABLED").unwrap_or(false),
        contention_alerts: ContentionAlertConfig::default(),
    })
}

fn load_sync_from_env() -> anyhow::Result<SyncConfig> {
    Ok(SyncConfig {
        enabled: parse_env("SY_ENABLED").unwrap_or(true),
        sync_interval_seconds: parse_env("SY_SYNC_INTERVAL_SECONDS").unwrap_or(60),
        batch_size: parse_env("SY_BATCH_SIZE").unwrap_or(100),
        checkpoint_enabled: parse_env("SY_CHECKPOINT_ENABLED").unwrap_or(true),
        conflict_resolution: env::var("SY_CONFLICT_RESOLUTION")
            .unwrap_or_else(|_| "prefer_knowledge".to_string()),
    })
}

fn load_memory_from_env() -> anyhow::Result<MemoryConfig> {
    Ok(MemoryConfig {
        promotion_threshold: parse_env("MK_PROMOTION_THRESHOLD").unwrap_or(0.8),
        decay_interval_secs: parse_env("MK_DECAY_INTERVAL_SECS").unwrap_or(86400),
        decay_rate: parse_env("MK_DECAY_RATE").unwrap_or(0.05),
        optimization_trigger_count: parse_env("MK_OPTIMIZATION_TRIGGER_COUNT").unwrap_or(100),
        layer_summary_configs: std::collections::HashMap::new(),
        reasoning: ReasoningConfig::default(),
        rlm: load_rlm_from_env()?,
    })
}

fn load_rlm_from_env() -> anyhow::Result<RlmConfig> {
    Ok(RlmConfig {
        enabled: parse_env("MK_RLM_ENABLED").unwrap_or(true),
        max_steps: parse_env("MK_RLM_MAX_STEPS").unwrap_or(5),
        complexity_threshold: parse_env("MK_RLM_COMPLEXITY_THRESHOLD").unwrap_or(0.3),
    })
}

fn load_tools_from_env() -> anyhow::Result<ToolConfig> {
    Ok(ToolConfig {
        enabled: parse_env("TL_ENABLED").unwrap_or(true),
        host: env::var("TL_HOST").unwrap_or_else(|_| "localhost".to_string()),
        port: parse_env("TL_PORT").unwrap_or(8080),
        api_key: env::var("TL_API_KEY").ok(),
        rate_limit_requests_per_minute: parse_env("TL_RATE_LIMIT_REQUESTS_PER_MINUTE")
            .unwrap_or(60),
    })
}

fn load_observability_from_env() -> anyhow::Result<ObservabilityConfig> {
    Ok(ObservabilityConfig {
        metrics_enabled: parse_env("OB_METRICS_ENABLED").unwrap_or(true),
        tracing_enabled: parse_env("OB_TRACING_ENABLED").unwrap_or(true),
        logging_level: env::var("OB_LOGGING_LEVEL").unwrap_or_else(|_| "info".to_string()),
        metrics_port: parse_env("OB_METRICS_PORT").unwrap_or(9090),
    })
}

fn parse_env<T>(key: &str) -> anyhow::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match env::var(key) {
        Ok(s) => s.parse::<T>().map_err(anyhow::Error::from),
        Err(e) => Err(anyhow::Error::from(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn test_load_from_env_defaults() {
        unsafe {
            env::remove_var("PG_HOST");
            env::remove_var("QD_HOST");
            env::remove_var("RD_HOST");
            env::remove_var("SY_ENABLED");
            env::remove_var("TL_PORT");
            env::remove_var("OB_LOGGING_LEVEL");
        }
        let config = load_from_env().unwrap();
        assert_eq!(config.providers.postgres.host, "localhost");
        assert_eq!(config.providers.qdrant.host, "localhost");
        assert_eq!(config.providers.redis.host, "localhost");
        assert!(config.sync.enabled);
        assert_eq!(config.tools.port, 8080);
        assert_eq!(config.observability.logging_level, "info");
    }

    #[test]
    #[serial]
    fn test_load_from_env_overrides() {
        unsafe {
            env::set_var("PG_HOST", "testhost");
            env::set_var("PG_PORT", "9999");
            env::set_var("SY_ENABLED", "false");
        }

        let config = load_from_env().unwrap();
        assert_eq!(config.providers.postgres.host, "testhost");
        assert_eq!(config.providers.postgres.port, 9999);
        assert!(!config.sync.enabled);

        unsafe {
            env::remove_var("PG_HOST");
            env::remove_var("PG_PORT");
            env::remove_var("SY_ENABLED");
        }
    }

    #[test]
    fn test_parse_env_missing() {
        let result: Result<u32, _> = parse_env("NONEXISTENT_VAR");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_env_valid_string() {
        unsafe {
            env::set_var("TEST_VAR", "test_value");
        }
        let result: Result<String, _> = parse_env("TEST_VAR");
        assert_eq!(result.unwrap(), "test_value");
        unsafe {
            env::remove_var("TEST_VAR");
        }
    }

    #[test]
    fn test_parse_env_valid_number() {
        unsafe {
            env::set_var("TEST_VAR", "123");
        }
        let result: Result<u32, _> = parse_env("TEST_VAR");
        assert_eq!(result.unwrap(), 123);
        unsafe {
            env::remove_var("TEST_VAR");
        }
    }

    #[test]
    fn test_parse_env_valid_number_with_parse_env() {
        unsafe {
            env::set_var("TEST_VAR", "123");
        }
        let result: Result<u32, _> = parse_env("TEST_VAR");
        assert_eq!(result.unwrap(), 123);
        unsafe {
            env::remove_var("TEST_VAR");
        }
    }

    #[test]
    fn test_parse_env_invalid_number() {
        unsafe {
            env::set_var("TEST_VAR", "not_a_number");
        }
        let result: Result<u32, _> = parse_env("TEST_VAR");
        assert!(result.is_err());
        unsafe {
            env::remove_var("TEST_VAR");
        }
    }

    #[test]
    #[serial]
    fn test_load_postgres_from_env() {
        unsafe {
            env::set_var("PG_HOST", "customhost");
            env::set_var("PG_PORT", "5433");
            env::set_var("PG_DATABASE", "testdb");
            env::set_var("PG_USERNAME", "testuser");
            env::set_var("PG_PASSWORD", "testpass");
            env::set_var("PG_POOL_SIZE", "20");
            env::set_var("PG_TIMEOUT_SECONDS", "60");
        }

        let postgres = load_postgres_from_env().unwrap();

        unsafe {
            env::remove_var("PG_HOST");
            env::remove_var("PG_PORT");
            env::remove_var("PG_DATABASE");
            env::remove_var("PG_USERNAME");
            env::remove_var("PG_PASSWORD");
            env::remove_var("PG_POOL_SIZE");
            env::remove_var("PG_TIMEOUT_SECONDS");
        }

        assert_eq!(postgres.host, "customhost");
        assert_eq!(postgres.port, 5433);
        assert_eq!(postgres.database, "testdb");
        assert_eq!(postgres.username, "testuser");
        assert_eq!(postgres.password, "testpass");
        assert_eq!(postgres.pool_size, 20);
        assert_eq!(postgres.timeout_seconds, 60);
    }

    #[test]
    #[serial]
    fn test_load_qdrant_from_env() {
        unsafe {
            env::set_var("QD_HOST", "qdranthost");
            env::set_var("QD_PORT", "7333");
            env::set_var("QD_COLLECTION", "test_collection");
            env::set_var("QD_TIMEOUT_SECONDS", "45");
        }

        let qdrant = load_qdrant_from_env().unwrap();
        assert_eq!(qdrant.host, "qdranthost");
        assert_eq!(qdrant.port, 7333);
        assert_eq!(qdrant.collection, "test_collection");
        assert_eq!(qdrant.timeout_seconds, 45);

        unsafe {
            env::remove_var("QD_HOST");
            env::remove_var("QD_PORT");
            env::remove_var("QD_COLLECTION");
            env::remove_var("QD_TIMEOUT_SECONDS");
        }
    }

    #[test]
    #[serial]
    fn test_load_redis_from_env() {
        unsafe {
            env::set_var("RD_HOST", "redishost");
            env::set_var("RD_PORT", "6380");
            env::set_var("RD_DB", "1");
            env::set_var("RD_POOL_SIZE", "15");
            env::set_var("RD_TIMEOUT_SECONDS", "45");
        }

        let redis = load_redis_from_env().unwrap();
        assert_eq!(redis.host, "redishost");
        assert_eq!(redis.port, 6380);
        assert_eq!(redis.db, 1);
        assert_eq!(redis.pool_size, 15);
        assert_eq!(redis.timeout_seconds, 45);

        unsafe {
            env::remove_var("RD_HOST");
            env::remove_var("RD_PORT");
            env::remove_var("RD_DB");
            env::remove_var("RD_POOL_SIZE");
            env::remove_var("RD_TIMEOUT_SECONDS");
        }
    }

    #[test]
    #[serial]
    fn test_load_admin_bootstrap_defaults() {
        unsafe {
            env::remove_var("AETERNA_ADMIN_BOOTSTRAP_EMAIL");
            env::remove_var("AETERNA_ADMIN_BOOTSTRAP_PROVIDER");
            env::remove_var("AETERNA_ADMIN_BOOTSTRAP_PROVIDER_SUBJECT");
        }

        let cfg = load_admin_bootstrap_from_env().unwrap();
        assert_eq!(cfg.email, None);
        assert_eq!(cfg.provider, "github");
        assert_eq!(cfg.provider_subject, None);
    }

    #[test]
    #[serial]
    fn test_load_admin_bootstrap_from_env_all_set() {
        unsafe {
            env::set_var("AETERNA_ADMIN_BOOTSTRAP_EMAIL", "admin@example.com");
            env::set_var("AETERNA_ADMIN_BOOTSTRAP_PROVIDER", "okta");
            env::set_var("AETERNA_ADMIN_BOOTSTRAP_PROVIDER_SUBJECT", "admin-subject");
        }

        let cfg = load_admin_bootstrap_from_env().unwrap();

        unsafe {
            env::remove_var("AETERNA_ADMIN_BOOTSTRAP_EMAIL");
            env::remove_var("AETERNA_ADMIN_BOOTSTRAP_PROVIDER");
            env::remove_var("AETERNA_ADMIN_BOOTSTRAP_PROVIDER_SUBJECT");
        }

        assert_eq!(cfg.email, Some("admin@example.com".to_string()));
        assert_eq!(cfg.provider, "okta");
        assert_eq!(cfg.provider_subject, Some("admin-subject".to_string()));
    }

    #[test]
    #[serial]
    fn test_load_admin_bootstrap_email_only() {
        unsafe {
            env::set_var("AETERNA_ADMIN_BOOTSTRAP_EMAIL", "admin@example.com");
            env::remove_var("AETERNA_ADMIN_BOOTSTRAP_PROVIDER");
            env::remove_var("AETERNA_ADMIN_BOOTSTRAP_PROVIDER_SUBJECT");
        }

        let cfg = load_admin_bootstrap_from_env().unwrap();

        unsafe {
            env::remove_var("AETERNA_ADMIN_BOOTSTRAP_EMAIL");
        }

        assert_eq!(cfg.email, Some("admin@example.com".to_string()));
        assert_eq!(cfg.provider, "github");
        assert_eq!(cfg.provider_subject, None);
    }
}
