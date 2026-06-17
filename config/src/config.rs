//! # Configuration Structures
//!
//! This module defines all configuration structures for the Memory-Knowledge
//! system.
//!
//! All configuration structures:
//! - Use `serde` for serialization/deserialization
//! - Use `validator` for input validation
//! - Follow Microsoft Pragmatic Rust Guidelines
//! - Include comprehensive M-CANONICAL-DOCS

use crate::cca::CcaConfig;
use mk_core::types::PROVIDER_GITHUB;
use serde::{Deserialize, Serialize};
use validator::Validate;

fn default_allowed_providers() -> Vec<String> {
    vec![PROVIDER_GITHUB.to_string()]
}

/// Main configuration structure for the Memory-Knowledge system.
///
/// This is the top-level configuration that aggregates all subsystem
/// configurations.
///
/// # M-CANONICAL-DOCS
///
/// ## Purpose
/// Provides centralized configuration for the entire Memory-Knowledge system,
/// including storage providers, sync behavior, MCP tools, observability, and
/// CCA.
///
/// ## Usage
/// ```rust,no_run
/// use config::Config;
///
/// let config = Config::default();
/// println!("PostgreSQL host: {}", config.providers.postgres.host);
/// ```
///
/// ## Fields
/// - `providers`: Configuration for storage backends (PostgreSQL, Qdrant,
///   Redis)
/// - `sync`: Configuration for memory-knowledge synchronization
/// - `tools`: Configuration for MCP server tools
/// - `observability`: Configuration for metrics and tracing
/// - `cca`: Configuration for CCA (Confucius Code Agent) capabilities
///
/// ## Validation
/// All nested configurations must pass their own validation rules.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, Default, PartialEq)]
pub struct Config {
    /// Storage provider configurations (PostgreSQL, Qdrant, Redis)
    #[serde(default)]
    pub providers: ProviderConfig,

    /// Memory-knowledge synchronization configuration
    #[serde(default)]
    pub sync: SyncConfig,

    /// Memory system configuration
    #[serde(default)]
    pub memory: MemoryConfig,

    /// MCP tool interface configuration
    #[serde(default)]
    pub tools: ToolConfig,

    /// Observability configuration (metrics, tracing, logging)
    #[serde(default)]
    pub observability: ObservabilityConfig,

    /// Deployment mode configuration (Local, Hybrid, Remote)
    #[serde(default)]
    pub deployment: DeploymentConfig,

    /// Job coordination configuration (locks, timeouts, checkpoints)
    #[serde(default)]
    pub job: JobConfig,

    #[serde(default)]
    pub knowledge_repo: KnowledgeRepoConfig,

    #[serde(default)]
    pub plugin_auth: PluginAuthConfig,

    #[serde(default)]
    pub k8s_auth: KubernetesAuthConfig,

    #[serde(default)]
    pub admin_bootstrap: AdminBootstrapConfig,

    /// CCA (Confucius Code Agent) capabilities configuration
    #[serde(default)]
    pub cca: CcaConfig,

    /// Tenant provisioning runtime gates.
    ///
    /// Controls which **inputs** the `POST /api/v1/admin/tenants/provision`
    /// endpoint will accept. The knobs here govern *policy*, not
    /// *behavior* — they let an operator explicitly opt into looser
    /// acceptance criteria (today: inline secret plaintext in the manifest
    /// body) that are otherwise off by default because they are hostile
    /// to audit trails and secret hygiene.
    ///
    /// See `openspec/changes/harden-tenant-provisioning/tasks.md` §4.
    #[serde(default)]
    pub provisioning: ProvisioningConfig,
}

impl Config {
    /// Detects environment settings for deployment mode.
    ///
    /// # M-CANONICAL-DOCS
    ///
    /// ## Purpose
    /// Initializes configuration based on AETERNA_ environment variables.
    pub fn detect_env() -> Self {
        let mut config = Self::default();

        if let Ok(url) = std::env::var("AETERNA_REMOTE_GOVERNANCE_URL") {
            config.deployment.remote_url = Some(url);
            config.deployment.mode =
                std::env::var("AETERNA_DEPLOYMENT_MODE").unwrap_or_else(|_| "hybrid".to_string());
        }

        if std::env::var("AETERNA_THIN_CLIENT").is_ok() {
            config.deployment.mode = "remote".to_string();
            config.sync.enabled = false;
        }

        config
    }
}

/// Deployment mode configuration.
///
/// # M-CANONICAL-DOCS
///
/// ## Purpose
/// Manages the deployment mode of the system (Local, Hybrid, or Remote).
///
/// ## Fields
/// - `mode`: Deployment mode (default: "local")
/// - `remote_url`: URL of the remote governance server (required for
///   Hybrid/Remote)
/// - `sync_enabled`: Enable synchronization in Hybrid mode (default: true)
#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct DeploymentConfig {
    /// Deployment mode
    #[serde(default = "default_deployment_mode")]
    #[validate(custom(function = "validate_deployment_mode"))]
    pub mode: String,

    /// URL of the remote governance server
    #[serde(default)]
    pub remote_url: Option<String>,

    /// Enable synchronization in Hybrid mode
    #[serde(default = "default_deployment_sync_enabled")]
    pub sync_enabled: bool,
}

fn default_deployment_mode() -> String {
    "local".to_string()
}

fn default_deployment_sync_enabled() -> bool {
    true
}

fn validate_deployment_mode(value: &str) -> Result<(), validator::ValidationError> {
    match value {
        "local" | "hybrid" | "remote" => Ok(()),
        _ => Err(validator::ValidationError::new("Invalid deployment mode")),
    }
}

impl Default for DeploymentConfig {
    fn default() -> Self {
        Self {
            mode: default_deployment_mode(),
            remote_url: None,
            sync_enabled: default_deployment_sync_enabled(),
        }
    }
}

impl DeploymentConfig {
    pub fn auto_detect() -> Self {
        let mut config = Self::default();

        if let Ok(mode) = std::env::var("AETERNA_DEPLOYMENT_MODE") {
            config.mode = mode;
        }

        if let Ok(url) = std::env::var("AETERNA_REMOTE_GOVERNANCE_URL") {
            config.remote_url = Some(url);
            if config.mode == "local" {
                config.mode = "hybrid".to_string();
            }
        }

        if std::env::var("AETERNA_THIN_CLIENT").is_ok() {
            config.mode = "remote".to_string();
            config.sync_enabled = false;
        }

        if let Ok(sync) = std::env::var("AETERNA_SYNC_ENABLED") {
            config.sync_enabled = sync.to_lowercase() == "true" || sync == "1";
        }

        config
    }

    pub fn is_local(&self) -> bool {
        self.mode == "local"
    }

    pub fn is_hybrid(&self) -> bool {
        self.mode == "hybrid"
    }

    pub fn is_remote(&self) -> bool {
        self.mode == "remote"
    }

    pub fn requires_remote_url(&self) -> bool {
        self.is_hybrid() || self.is_remote()
    }

    pub fn requires_local_engine(&self) -> bool {
        self.is_local() || self.is_hybrid()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, Default, PartialEq)]
pub struct KnowledgeRepoConfig {
    #[serde(default)]
    pub remote_url: Option<String>,
    #[serde(default = "default_branch")]
    pub branch: String,
    #[serde(default)]
    pub github_owner: Option<String>,
    #[serde(default)]
    pub github_repo: Option<String>,
    #[serde(default)]
    pub github_token: Option<String>,
    #[serde(default)]
    pub github_app_id: Option<u64>,
    #[serde(default)]
    pub github_installation_id: Option<u64>,
    #[serde(default)]
    pub github_app_pem: Option<String>,
    #[serde(default)]
    pub webhook_secret: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct PluginAuthConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_allowed_providers")]
    pub allowed_providers: Vec<String>,
    #[serde(default)]
    pub github_client_id: Option<String>,
    #[serde(default)]
    pub github_client_secret: Option<String>,
    #[serde(default)]
    pub github_api_base_url: Option<String>,
    #[serde(default)]
    pub github_oauth_base_url: Option<String>,
    #[serde(default)]
    pub github_app_id: Option<u64>,
    #[serde(default)]
    pub github_app_name: Option<String>,
    #[serde(default)]
    pub github_app_pem: Option<String>,
    #[serde(default)]
    pub redirect_base_url: Option<String>,
    #[serde(default)]
    pub token_issuer: Option<String>,
    #[serde(default)]
    pub jwt_secret: Option<String>,
    #[serde(default)]
    pub access_token_ttl_seconds: Option<u64>,
    #[serde(default)]
    pub refresh_token_ttl_seconds: Option<u64>,
    /// The default tenant ID for user resolution across all flows: plugin auth
    /// token issuance, webhook event routing, and any other surface that needs
    /// to place an authenticated user into a tenant when one is not explicitly
    /// provided.  Required for the plugin auth bootstrap to succeed.  When
    /// absent the server fails closed rather than defaulting to the synthetic
    /// "default" tenant.  Set via `AETERNA_DEFAULT_TENANT_ID` or via
    /// `defaultTenantId` in Helm values.
    #[serde(default)]
    pub default_tenant_id: Option<String>,
}

impl Default for PluginAuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_providers: default_allowed_providers(),
            github_client_id: None,
            github_client_secret: None,
            github_api_base_url: None,
            github_oauth_base_url: None,
            github_app_id: None,
            github_app_name: None,
            github_app_pem: None,
            redirect_base_url: None,
            token_issuer: None,
            jwt_secret: None,
            access_token_ttl_seconds: None,
            refresh_token_ttl_seconds: None,
            default_tenant_id: None,
        }
    }
}

/// Configuration for Kubernetes Service Account token authentication.
///
/// When `enabled` is true the server validates `Authorization: Bearer <token>`
/// requests via the Kubernetes TokenReview API before looking up the identity
/// in the `users` table.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, Default, PartialEq)]
pub struct KubernetesAuthConfig {
    /// Enable K8s SA token validation.
    #[serde(default)]
    pub enabled: bool,
    /// URL of the Kubernetes API server (e.g. "https://kubernetes.default.svc").
    /// Defaults to in-cluster API server when absent.
    #[serde(default)]
    pub api_server_url: Option<String>,
    /// Audience expected in the projected token (e.g. "aeterna").
    /// Passed to the TokenReview `spec.audiences` field.
    #[serde(default)]
    pub token_review_audience: Option<String>,
    /// Path to the service account token file used to authenticate the
    /// TokenReview request itself (in-cluster: `/var/run/secrets/kubernetes.io/serviceaccount/token`).
    #[serde(default)]
    pub sa_token_path: Option<String>,
    /// Path to the CA bundle for TLS verification of the K8s API server.
    /// Defaults to in-cluster CA bundle when absent.
    #[serde(default)]
    pub ca_bundle_path: Option<String>,
    #[serde(default = "default_k8s_namespace")]
    pub namespace: Option<String>,
}

fn default_k8s_namespace() -> Option<String> {
    None
}

/// Tenant provisioning policy gates.
///
/// B2 task 4.1: inline secret plaintext in `secrets[].secretValue` on the
/// provision manifest is a power-tool — it bypasses the normal
/// reference-indirected secret flow (`SecretReference` → config provider
/// → backend store) and puts raw material on the wire. We want it
/// available for local dev fixtures and emergency breakglass, but
/// **off by default** and **mechanically unreachable in release builds**.
///
/// The two-layer gate is:
///
/// 1. [`allow_inline_secret`] must be `true` in the server config.
/// 2. The caller must additionally pass `?allowInline=true` on the
///    provision request (task 4.2).
///
/// Both conditions are required; either one missing and the endpoint
/// rejects non-empty inline `secretValue` with a 422 pointing at
/// `config.secretReferences` (task 4.3).
///
/// ### Release-build hard-off
///
/// [`Self::effective_allow_inline_secret`] returns `false` unconditionally
/// when `debug_assertions` is off (i.e. `--release`), regardless of what
/// the YAML says. That is the "off in release builds" clause of task 4.1
/// and cannot be overridden by environment variable, config file, query
/// parameter, or header. Inline secrets are a debug/dev convenience
/// only; a prod pod will refuse them even if someone misconfigures the
/// container.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, Default, PartialEq)]
pub struct ProvisioningConfig {
    /// When `true` *and* running under `debug_assertions`, the provision
    /// endpoint will accept `secrets[].secretValue` plaintext **if** the
    /// caller also opts in with `?allowInline=true`.
    ///
    /// Default: `false`. Release builds force this to `false`
    /// (see [`Self::effective_allow_inline_secret`]).
    #[serde(default)]
    pub allow_inline_secret: bool,
}

impl ProvisioningConfig {
    /// The *effective* value of [`Self::allow_inline_secret`] after the
    /// release-build hard-off has been applied.
    ///
    /// This is the only method the request handler should consult. It
    /// returns `false` in release builds regardless of config, so inline
    /// secrets are mechanically unreachable in production even under
    /// misconfiguration. See the type-level docs for the full gate chain.
    #[must_use]
    pub fn effective_allow_inline_secret(&self) -> bool {
        if cfg!(debug_assertions) {
            self.allow_inline_secret
        } else {
            // Release-build hard-off. Task 4.1: "off in release builds".
            false
        }
    }
}

/// Configuration for the one-time PlatformAdmin bootstrap seeding.
///
/// When `email` is set, the bootstrap routine creates (or updates) a
/// PlatformAdmin user on startup.  This runs idempotently inside a
/// transaction so repeated restarts are safe.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, Default, PartialEq)]
pub struct AdminBootstrapConfig {
    /// Email of the initial PlatformAdmin.  When `None` the seeding is
    /// skipped entirely.
    #[serde(default)]
    pub email: Option<String>,

    /// Identity provider name (e.g. "github", "okta").  Defaults to "github".
    #[serde(default = "default_admin_bootstrap_provider")]
    pub provider: String,

    /// Provider-specific subject identifier (e.g. GitHub login).
    /// When `None` the email is used as subject.
    #[serde(default)]
    pub provider_subject: Option<String>,

    #[serde(default = "default_bootstrap_tenant_slug")]
    pub tenant_slug: String,

    #[serde(default = "default_bootstrap_org_slug")]
    pub org_slug: String,

    #[serde(default = "default_bootstrap_team_slug")]
    pub team_slug: String,

    /// Kubernetes service account subject to bootstrap as PlatformAdmin.
    /// Format: "system:serviceaccount:<namespace>:<name>"
    /// When set, a user row with idp_provider="kubernetes" is seeded.
    #[serde(default)]
    pub k8s_sa_subject: Option<String>,
}

fn default_admin_bootstrap_provider() -> String {
    PROVIDER_GITHUB.to_string()
}

fn default_bootstrap_tenant_slug() -> String {
    "default".to_string()
}

fn default_bootstrap_org_slug() -> String {
    "platform".to_string()
}

fn default_bootstrap_team_slug() -> String {
    "admins".to_string()
}

fn default_branch() -> String {
    "main".to_string()
}

/// Configuration for storage providers.
///
/// Manages connection settings for all storage backends.
///
/// # M-CANONICAL-DOCS
///
/// ## Purpose
/// Centralizes connection configuration for all storage backends:
/// - PostgreSQL: Primary data storage (metadata, governance, RLS)
/// - Qdrant: Vector similarity search
/// - Redis: Caching layer
/// - Graph: DuckDB-based knowledge graph
///
/// ## Usage
/// ```rust,no_run
/// use config::ProviderConfig;
///
/// let providers = ProviderConfig::default();
/// assert_eq!(providers.postgres.host, "localhost");
/// ```
///
/// ## Fields
/// - `postgres`: PostgreSQL connection configuration
/// - `qdrant`: Qdrant vector database configuration
/// - `redis`: Redis caching configuration
/// - `graph`: DuckDB graph store configuration
#[derive(Debug, Clone, Serialize, Deserialize, Validate, Default, PartialEq)]
pub struct ProviderConfig {
    /// PostgreSQL connection configuration
    #[serde(default)]
    pub postgres: PostgresConfig,

    /// Qdrant vector database configuration
    #[serde(default)]
    pub qdrant: QdrantConfig,

    /// Redis caching configuration
    #[serde(default)]
    pub redis: RedisConfig,

    /// DuckDB graph store configuration
    #[serde(default)]
    pub graph: GraphConfig,
}

/// PostgreSQL configuration.
///
/// # M-CANONICAL-DOCS
///
/// ## Purpose
/// Manages connection settings for PostgreSQL, the primary data storage
/// backend.
///
/// ## Fields
/// - `host`: Database server hostname (default: "localhost")
/// - `port`: Database server port (default: 5432)
/// - `database`: Database name (required)
/// - `username`: Database user (required)
/// - `password`: Database password (required, should use environment variable)
/// - `pool_size`: Maximum connections in pool (default: 10, range: 1-100)
/// - `timeout_seconds`: Connection timeout (default: 30, range: 1-300)
#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct PostgresConfig {
    /// Database server hostname
    #[serde(default = "default_postgres_host")]
    #[validate(length(min = 1, max = 255))]
    pub host: String,

    /// Database server port
    #[serde(default = "default_postgres_port")]
    #[validate(range(min = 1, max = 65535))]
    pub port: u16,

    /// Database name
    #[serde(default = "default_postgres_database")]
    #[validate(length(min = 1, max = 63))]
    pub database: String,

    /// Database username
    #[serde(default = "default_postgres_username")]
    #[validate(length(min = 1, max = 63))]
    pub username: String,

    /// Database password
    #[serde(default = "default_postgres_password")]
    #[validate(length(min = 1))]
    pub password: String,

    /// Maximum connections in pool
    #[serde(default = "default_postgres_pool_size")]
    #[validate(range(min = 1, max = 100))]
    pub pool_size: u32,

    /// Connection timeout in seconds
    #[serde(default = "default_postgres_timeout")]
    #[validate(range(min = 1, max = 300))]
    pub timeout_seconds: u64,
}

pub(crate) fn default_postgres_host() -> String {
    "localhost".to_string()
}

pub(crate) fn default_postgres_port() -> u16 {
    5432
}

pub(crate) fn default_postgres_database() -> String {
    "memory_knowledge".to_string()
}

pub(crate) fn default_postgres_username() -> String {
    "postgres".to_string()
}

fn default_postgres_password() -> String {
    "".to_string()
}

pub(crate) fn default_postgres_pool_size() -> u32 {
    10
}

pub(crate) fn default_postgres_timeout() -> u64 {
    30
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            host: default_postgres_host(),
            port: default_postgres_port(),
            database: default_postgres_database(),
            username: default_postgres_username(),
            password: default_postgres_password(),
            pool_size: default_postgres_pool_size(),
            timeout_seconds: default_postgres_timeout(),
        }
    }
}

/// Qdrant vector database configuration.
///
/// # M-CANONICAL-DOCS
///
/// ## Purpose
/// Manages connection settings for Qdrant, used for vector similarity search.
///
/// ## Fields
/// - `host`: Qdrant server hostname (default: "localhost")
/// - `port`: Qdrant server port (default: 6333)
/// - `collection`: Default collection name (required)
/// - `timeout_seconds`: Request timeout (default: 30, range: 1-300)
#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct QdrantConfig {
    /// Qdrant server hostname
    #[serde(default = "default_qdrant_host")]
    #[validate(length(min = 1, max = 255))]
    pub host: String,

    /// Qdrant server port
    #[serde(default = "default_qdrant_port")]
    #[validate(range(min = 1, max = 65535))]
    pub port: u16,

    /// Default collection name
    #[serde(default = "default_qdrant_collection")]
    #[validate(length(min = 1, max = 255))]
    pub collection: String,

    /// Request timeout in seconds
    #[serde(default = "default_qdrant_timeout")]
    #[validate(range(min = 1, max = 300))]
    pub timeout_seconds: u64,
}

pub(crate) fn default_qdrant_host() -> String {
    "localhost".to_string()
}

pub(crate) fn default_qdrant_port() -> u16 {
    6333
}

pub(crate) fn default_qdrant_collection() -> String {
    "memory_embeddings".to_string()
}

pub(crate) fn default_qdrant_timeout() -> u64 {
    30
}

impl Default for QdrantConfig {
    fn default() -> Self {
        Self {
            host: default_qdrant_host(),
            port: default_qdrant_port(),
            collection: default_qdrant_collection(),
            timeout_seconds: default_qdrant_timeout(),
        }
    }
}

/// Redis configuration.
///
/// # M-CANONICAL-DOCS
///
/// ## Purpose
/// Manages connection settings for Redis, used as a caching layer.
///
/// ## Fields
/// - `host`: Redis server hostname (default: "localhost")
/// - `port`: Redis server port (default: 6379)
/// - `db`: Redis database number (default: 0, range: 0-15)
/// - `pool_size`: Maximum connections in pool (default: 10, range: 1-100)
/// - `timeout_seconds`: Connection timeout (default: 30, range: 1-300)
#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct RedisConfig {
    /// Redis server hostname
    #[serde(default = "default_redis_host")]
    #[validate(length(min = 1, max = 255))]
    pub host: String,

    /// Redis server port
    #[serde(default = "default_redis_port")]
    #[validate(range(min = 1, max = 65535))]
    pub port: u16,

    /// Redis database number
    #[serde(default = "default_redis_db")]
    #[validate(range(min = 0, max = 15))]
    pub db: u8,

    /// Maximum connections in pool
    #[serde(default = "default_redis_pool_size")]
    #[validate(range(min = 1, max = 100))]
    pub pool_size: u32,

    /// Connection timeout in seconds
    #[serde(default = "default_redis_timeout")]
    #[validate(range(min = 1, max = 300))]
    pub timeout_seconds: u64,
}

pub(crate) fn default_redis_host() -> String {
    "localhost".to_string()
}

pub(crate) fn default_redis_port() -> u16 {
    6379
}

pub(crate) fn default_redis_db() -> u8 {
    0
}

pub(crate) fn default_redis_pool_size() -> u32 {
    10
}

pub(crate) fn default_redis_timeout() -> u64 {
    30
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            host: default_redis_host(),
            port: default_redis_port(),
            db: default_redis_db(),
            pool_size: default_redis_pool_size(),
            timeout_seconds: default_redis_timeout(),
        }
    }
}

/// DuckDB Graph Store configuration.
///
/// # M-CANONICAL-DOCS
///
/// ## Purpose
/// Manages configuration for the DuckDB-based knowledge graph storage layer.
///
/// ## Fields
/// - `enabled`: Enable/disable graph store (default: true)
/// - `database_path`: Path to DuckDB database file (default: ":memory:")
/// - `s3_bucket`: Optional S3 bucket for persistence
/// - `s3_prefix`: Optional S3 key prefix
/// - `s3_endpoint`: Optional S3 endpoint for MinIO/localstack
/// - `s3_region`: S3 region (default: "us-east-1")
#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct GraphConfig {
    /// Enable/disable graph store
    #[serde(default = "default_graph_enabled")]
    pub enabled: bool,

    /// Path to DuckDB database file (use ":memory:" for in-memory)
    #[serde(default = "default_graph_path")]
    #[validate(length(min = 1, max = 255))]
    pub database_path: String,

    /// Optional S3 bucket for graph persistence
    #[serde(default)]
    pub s3_bucket: Option<String>,

    /// Optional S3 key prefix
    #[serde(default)]
    pub s3_prefix: Option<String>,

    /// Optional S3 endpoint (for MinIO or localstack)
    #[serde(default)]
    pub s3_endpoint: Option<String>,

    /// S3 region
    #[serde(default = "default_graph_s3_region")]
    pub s3_region: String,

    /// Optional explicit reader-pool size override for DuckDB read connections.
    #[serde(default)]
    #[validate(range(min = 1, max = 64))]
    pub reader_pool_size: Option<usize>,

    /// Full snapshot cadence in seconds.
    #[serde(default = "default_graph_snapshot_full_interval_secs")]
    #[validate(range(min = 60, max = 31536000))]
    pub snapshot_full_interval_secs: u64,

    /// Delta snapshot cadence in seconds.
    #[serde(default = "default_graph_snapshot_delta_interval_secs")]
    #[validate(range(min = 1, max = 31536000))]
    pub snapshot_delta_interval_secs: u64,

    /// Enable event-sourced WAL coordination across pods (Phase 2).
    /// When OFF (default): writes go directly to local DuckDB only.
    /// When ON: writes are dual-written to Postgres event log + local DuckDB.
    #[serde(default)]
    pub event_sourcing_enabled: bool,

    /// Alerting thresholds for write contention
    #[serde(default)]
    pub contention_alerts: ContentionAlertConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct ContentionAlertConfig {
    #[serde(default = "default_queue_depth_warn")]
    #[validate(range(min = 1, max = 100))]
    pub queue_depth_warn: u32,

    #[serde(default = "default_queue_depth_critical")]
    #[validate(range(min = 1, max = 100))]
    pub queue_depth_critical: u32,

    #[serde(default = "default_wait_time_warn_ms")]
    #[validate(range(min = 100, max = 60000))]
    pub wait_time_warn_ms: u64,

    #[serde(default = "default_wait_time_critical_ms")]
    #[validate(range(min = 100, max = 60000))]
    pub wait_time_critical_ms: u64,

    #[serde(default = "default_timeout_rate_warn")]
    #[validate(range(min = 0.0, max = 100.0))]
    pub timeout_rate_warn_percent: f64,

    #[serde(default = "default_timeout_rate_critical")]
    #[validate(range(min = 0.0, max = 100.0))]
    pub timeout_rate_critical_percent: f64,
}

fn default_queue_depth_warn() -> u32 {
    5
}
fn default_queue_depth_critical() -> u32 {
    10
}
fn default_wait_time_warn_ms() -> u64 {
    1000
}
fn default_wait_time_critical_ms() -> u64 {
    3000
}
fn default_timeout_rate_warn() -> f64 {
    5.0
}
fn default_timeout_rate_critical() -> f64 {
    15.0
}

impl Default for ContentionAlertConfig {
    fn default() -> Self {
        Self {
            queue_depth_warn: default_queue_depth_warn(),
            queue_depth_critical: default_queue_depth_critical(),
            wait_time_warn_ms: default_wait_time_warn_ms(),
            wait_time_critical_ms: default_wait_time_critical_ms(),
            timeout_rate_warn_percent: default_timeout_rate_warn(),
            timeout_rate_critical_percent: default_timeout_rate_critical(),
        }
    }
}

fn default_graph_enabled() -> bool {
    true
}

fn default_graph_path() -> String {
    ":memory:".to_string()
}

fn default_graph_s3_region() -> String {
    "us-east-1".to_string()
}

fn default_graph_snapshot_full_interval_secs() -> u64 {
    604800
}

fn default_graph_snapshot_delta_interval_secs() -> u64 {
    300
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            enabled: default_graph_enabled(),
            database_path: default_graph_path(),
            s3_bucket: None,
            s3_prefix: None,
            s3_endpoint: None,
            s3_region: default_graph_s3_region(),
            reader_pool_size: None,
            snapshot_full_interval_secs: default_graph_snapshot_full_interval_secs(),
            snapshot_delta_interval_secs: default_graph_snapshot_delta_interval_secs(),
            event_sourcing_enabled: false,
            contention_alerts: ContentionAlertConfig::default(),
        }
    }
}

/// Synchronization configuration.
///
/// # M-CANONICAL-DOCS
///
/// ## Purpose
/// Manages synchronization behavior between memory and knowledge systems.
///
/// ## Fields
/// - `enabled`: Enable/disable automatic sync (default: true)
/// - `sync_interval_seconds`: Sync interval (default: 60, range: 10-3600)
/// - `batch_size`: Number of items per sync batch (default: 100, range: 1-1000)
/// - `checkpoint_enabled`: Enable checkpointing for rollback (default: true)
/// - `conflict_resolution`: Strategy for conflict resolution (default:
///   "prefer_knowledge")
#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct SyncConfig {
    /// Enable/disable automatic synchronization
    #[serde(default = "default_sync_enabled")]
    pub enabled: bool,

    /// Sync interval in seconds
    #[serde(default = "default_sync_interval")]
    #[validate(range(min = 10, max = 3600))]
    pub sync_interval_seconds: u64,

    /// Number of items per sync batch
    #[serde(default = "default_sync_batch_size")]
    #[validate(range(min = 1, max = 1000))]
    pub batch_size: u32,

    /// Enable checkpointing for rollback
    #[serde(default = "default_sync_checkpoint")]
    pub checkpoint_enabled: bool,

    /// Conflict resolution strategy
    #[serde(default = "default_sync_conflict_resolution")]
    #[validate(custom(function = "validate_conflict_resolution"))]
    pub conflict_resolution: String,
}

fn default_sync_enabled() -> bool {
    true
}

pub(crate) fn default_sync_interval() -> u64 {
    60
}

pub(crate) fn default_sync_batch_size() -> u32 {
    100
}

fn default_sync_checkpoint() -> bool {
    true
}

pub(crate) fn default_sync_conflict_resolution() -> String {
    "prefer_knowledge".to_string()
}

fn validate_conflict_resolution(value: &str) -> Result<(), validator::ValidationError> {
    match value {
        "prefer_knowledge" | "prefer_memory" | "manual" => Ok(()),
        _ => Err(validator::ValidationError::new(
            "Invalid conflict resolution strategy",
        )),
    }
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            enabled: default_sync_enabled(),
            sync_interval_seconds: default_sync_interval(),
            batch_size: default_sync_batch_size(),
            checkpoint_enabled: default_sync_checkpoint(),
            conflict_resolution: default_sync_conflict_resolution(),
        }
    }
}

/// MCP tool interface configuration.
///
/// # M-CANONICAL-DOCS
///
/// ## Purpose
/// Manages configuration for the MCP (Model Context Protocol) server interface.
///
/// ## Fields
/// - `enabled`: Enable/disable MCP server (default: true)
/// - `host`: Server hostname (default: "localhost")
/// - `port`: Server port (default: 8080)
/// - `api_key`: API key for authentication (optional)
/// - `rate_limit_requests_per_minute`: Rate limit (default: 60, range: 1-1000)
#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct ToolConfig {
    /// Enable/disable MCP server
    #[serde(default = "default_tools_enabled")]
    pub enabled: bool,

    /// Server hostname
    #[serde(default = "default_tools_host")]
    #[validate(length(min = 1, max = 255))]
    pub host: String,

    /// Server port
    #[serde(default = "default_tools_port")]
    #[validate(range(min = 1, max = 65535))]
    pub port: u16,

    /// API key for authentication
    #[serde(default)]
    pub api_key: Option<String>,

    /// Rate limit: requests per minute
    #[serde(default = "default_tools_rate_limit")]
    #[validate(range(min = 1, max = 1000))]
    pub rate_limit_requests_per_minute: u32,
}

fn default_tools_enabled() -> bool {
    true
}

pub(crate) fn default_tools_host() -> String {
    "localhost".to_string()
}

pub(crate) fn default_tools_port() -> u16 {
    8080
}

pub(crate) fn default_tools_rate_limit() -> u32 {
    60
}

impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            enabled: default_tools_enabled(),
            host: default_tools_host(),
            port: default_tools_port(),
            api_key: None,
            rate_limit_requests_per_minute: default_tools_rate_limit(),
        }
    }
}

/// Observability configuration.
///
/// # M-CANONICAL-DOCS
///
/// ## Purpose
/// Manages configuration for metrics, tracing, and logging.
///
/// ## Fields
/// - `metrics_enabled`: Enable metrics collection (default: true)
/// - `tracing_enabled`: Enable distributed tracing (default: true)
/// - `logging_level`: Log level (default: "info")
/// - `metrics_port`: Metrics server port (default: 9090)
#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct ObservabilityConfig {
    /// Enable metrics collection
    #[serde(default = "default_observability_metrics_enabled")]
    pub metrics_enabled: bool,

    /// Enable distributed tracing
    #[serde(default = "default_observability_tracing_enabled")]
    pub tracing_enabled: bool,

    /// Logging level
    #[serde(default = "default_observability_logging_level")]
    #[validate(custom(function = "validate_logging_level"))]
    pub logging_level: String,

    /// Metrics server port
    #[serde(default = "default_observability_metrics_port")]
    #[validate(range(min = 1, max = 65535))]
    pub metrics_port: u16,
}

fn default_observability_metrics_enabled() -> bool {
    true
}

fn default_observability_tracing_enabled() -> bool {
    true
}

pub(crate) fn default_observability_logging_level() -> String {
    "info".to_string()
}

pub(crate) fn default_observability_metrics_port() -> u16 {
    9090
}

fn validate_logging_level(value: &str) -> Result<(), validator::ValidationError> {
    match value {
        "trace" | "debug" | "info" | "warn" | "error" => Ok(()),
        _ => Err(validator::ValidationError::new("Invalid logging level")),
    }
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            metrics_enabled: default_observability_metrics_enabled(),
            tracing_enabled: default_observability_tracing_enabled(),
            logging_level: default_observability_logging_level(),
            metrics_port: default_observability_metrics_port(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct MemoryConfig {
    #[serde(default = "default_promotion_threshold")]
    #[validate(range(min = 0.0, max = 1.0))]
    pub promotion_threshold: f32,

    #[serde(default = "default_decay_interval")]
    #[validate(range(min = 3600, max = 86400))]
    pub decay_interval_secs: u64,

    #[serde(default = "default_decay_rate")]
    #[validate(range(min = 0.0, max = 0.5))]
    pub decay_rate: f32,

    #[serde(default = "default_optimization_trigger_count")]
    #[validate(range(min = 10, max = 1000))]
    pub optimization_trigger_count: usize,

    #[serde(default)]
    pub layer_summary_configs:
        std::collections::HashMap<mk_core::types::MemoryLayer, mk_core::types::SummaryConfig>,

    /// Configuration for reflective reasoning during memory search
    #[serde(default)]
    pub reasoning: ReasoningConfig,

    /// Configuration for Recursive Language Model (RLM) search
    #[serde(default)]
    pub rlm: RlmConfig,
}

/// Configuration for Recursive Language Model (RLM) search.
///
/// # M-CANONICAL-DOCS
///
/// ## Purpose
/// Controls the behavior of the recursive search strategy used for complex
/// queries that require multi-hop reasoning or synthesis across multiple
/// layers.
///
/// ## Fields
/// - `enabled`: Enable/disable RLM search (default: true)
/// - `max_steps`: Maximum number of decomposition steps (default: 5)
/// - `complexity_threshold`: Threshold for routing queries to RLM (default:
///   0.3)
#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct RlmConfig {
    /// Enable/disable RLM search
    #[serde(default = "default_rlm_enabled")]
    pub enabled: bool,

    /// Maximum number of recursive decomposition steps
    #[serde(default = "default_rlm_max_steps")]
    #[validate(range(min = 1, max = 20))]
    pub max_steps: usize,

    /// Threshold [0.0, 1.0] for routing to RLM based on query complexity
    #[serde(default = "default_rlm_complexity_threshold")]
    #[validate(range(min = 0.0, max = 1.0))]
    pub complexity_threshold: f32,
}

fn default_rlm_enabled() -> bool {
    true
}

fn default_rlm_max_steps() -> usize {
    5
}

fn default_rlm_complexity_threshold() -> f32 {
    0.3
}

impl Default for RlmConfig {
    fn default() -> Self {
        Self {
            enabled: default_rlm_enabled(),
            max_steps: default_rlm_max_steps(),
            complexity_threshold: default_rlm_complexity_threshold(),
        }
    }
}

/// Configuration for reflective reasoning during memory retrieval.
///
/// # M-CANONICAL-DOCS
///
/// ## Purpose
/// Controls the behavior of the reflective reasoning step that can be applied
/// before memory search to improve retrieval quality through query refinement
/// and strategy selection.
///
/// ## Fields
/// - `enabled`: Enable/disable reasoning step (default: false)
/// - `timeout_ms`: Maximum time for reasoning LLM call (default: 3000)
/// - `bypass_simple_queries`: Skip reasoning for simple queries (default: true)
/// - `simple_query_max_words`: Threshold for simple query detection (default:
///   5)
#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct ReasoningConfig {
    /// Enable/disable reflective reasoning during search
    #[serde(default = "default_reasoning_enabled")]
    pub enabled: bool,

    /// Maximum time in milliseconds for reasoning LLM call
    #[serde(default = "default_reasoning_timeout_ms")]
    #[validate(range(min = 500, max = 30000))]
    pub timeout_ms: u64,

    /// Skip reasoning for simple queries (reduces latency and cost)
    #[serde(default = "default_reasoning_bypass_simple")]
    pub bypass_simple_queries: bool,

    /// Maximum word count to consider a query "simple"
    #[serde(default = "default_reasoning_simple_max_words")]
    #[validate(range(min = 1, max = 20))]
    pub simple_query_max_words: usize,

    /// Multiplier for search limit when using Exhaustive strategy
    #[serde(default = "default_exhaustive_limit_multiplier")]
    #[validate(range(min = 1.0, max = 5.0))]
    pub exhaustive_limit_multiplier: f32,

    /// Multiplier for search limit when using Targeted strategy
    #[serde(default = "default_targeted_limit_multiplier")]
    #[validate(range(min = 0.5, max = 2.0))]
    pub targeted_limit_multiplier: f32,

    /// P95 latency threshold in ms for alerting (default: 2500, which is below
    /// 3000ms timeout)
    #[serde(default = "default_reasoning_p95_threshold_ms")]
    #[validate(range(min = 100, max = 30000))]
    pub p95_latency_threshold_ms: u64,

    /// Cache TTL in seconds for reasoning results (default: 3600)
    #[serde(default = "default_reasoning_cache_ttl_seconds")]
    #[validate(range(min = 60, max = 86400))]
    pub cache_ttl_seconds: u64,

    /// Enable caching of reasoning results (default: true)
    #[serde(default = "default_reasoning_cache_enabled")]
    pub cache_enabled: bool,

    /// Maximum number of entries in reasoning cache (default: 10,000)
    #[serde(default = "default_reasoning_cache_max_entries")]
    #[validate(range(min = 100, max = 1000000))]
    pub cache_max_entries: usize,

    /// Enable circuit breaker for reasoning failures (default: true)
    #[serde(default = "default_circuit_breaker_enabled")]
    pub circuit_breaker_enabled: bool,

    /// Circuit breaker failure threshold percentage (default: 5.0)
    #[serde(default = "default_circuit_breaker_failure_threshold_percent")]
    #[validate(range(min = 1.0, max = 100.0))]
    pub circuit_breaker_failure_threshold_percent: f64,

    /// Circuit breaker window duration in seconds (default: 300)
    #[serde(default = "default_circuit_breaker_window_secs")]
    #[validate(range(min = 30, max = 3600))]
    pub circuit_breaker_window_secs: u64,

    /// Minimum requests in window before circuit breaker can trip (default: 10)
    #[serde(default = "default_circuit_breaker_min_requests")]
    #[validate(range(min = 1, max = 1000))]
    pub circuit_breaker_min_requests: u64,

    /// Circuit breaker recovery timeout in seconds (default: 60)
    #[serde(default = "default_circuit_breaker_recovery_secs")]
    #[validate(range(min = 5, max = 600))]
    pub circuit_breaker_recovery_secs: u64,

    /// Max requests allowed in half-open state (default: 3)
    #[serde(default = "default_circuit_breaker_half_open_requests")]
    #[validate(range(min = 1, max = 10))]
    pub circuit_breaker_half_open_requests: u64,

    /// Maximum hop depth for multi-hop retrieval (default: 3)
    #[serde(default = "default_max_hop_depth")]
    #[validate(range(min = 1, max = 10))]
    pub max_hop_depth: u32,

    /// Relevance threshold for path continuation in multi-hop (default: 0.3)
    #[serde(default = "default_hop_relevance_threshold")]
    #[validate(range(min = 0.0, max = 1.0))]
    pub hop_relevance_threshold: f32,

    /// Maximum query budget for multi-hop retrieval (default: 50)
    #[serde(default = "default_max_query_budget")]
    #[validate(range(min = 1, max = 500))]
    pub max_query_budget: u32,
}

fn default_reasoning_enabled() -> bool {
    false
}

fn default_reasoning_timeout_ms() -> u64 {
    3000
}

fn default_reasoning_bypass_simple() -> bool {
    true
}

fn default_reasoning_simple_max_words() -> usize {
    5
}

fn default_exhaustive_limit_multiplier() -> f32 {
    2.0
}

fn default_targeted_limit_multiplier() -> f32 {
    1.0
}

fn default_reasoning_p95_threshold_ms() -> u64 {
    2500
}

fn default_reasoning_cache_ttl_seconds() -> u64 {
    3600
}

fn default_reasoning_cache_enabled() -> bool {
    true
}

fn default_reasoning_cache_max_entries() -> usize {
    10000
}

fn default_circuit_breaker_enabled() -> bool {
    true
}

fn default_circuit_breaker_failure_threshold_percent() -> f64 {
    5.0
}

fn default_circuit_breaker_window_secs() -> u64 {
    300
}

fn default_circuit_breaker_min_requests() -> u64 {
    10
}

fn default_circuit_breaker_recovery_secs() -> u64 {
    60
}

fn default_circuit_breaker_half_open_requests() -> u64 {
    3
}

fn default_max_hop_depth() -> u32 {
    3
}

fn default_hop_relevance_threshold() -> f32 {
    0.3
}

fn default_max_query_budget() -> u32 {
    50
}

impl Default for ReasoningConfig {
    fn default() -> Self {
        Self {
            enabled: default_reasoning_enabled(),
            timeout_ms: default_reasoning_timeout_ms(),
            bypass_simple_queries: default_reasoning_bypass_simple(),
            simple_query_max_words: default_reasoning_simple_max_words(),
            exhaustive_limit_multiplier: default_exhaustive_limit_multiplier(),
            targeted_limit_multiplier: default_targeted_limit_multiplier(),
            p95_latency_threshold_ms: default_reasoning_p95_threshold_ms(),
            cache_ttl_seconds: default_reasoning_cache_ttl_seconds(),
            cache_enabled: default_reasoning_cache_enabled(),
            cache_max_entries: default_reasoning_cache_max_entries(),
            circuit_breaker_enabled: default_circuit_breaker_enabled(),
            circuit_breaker_failure_threshold_percent:
                default_circuit_breaker_failure_threshold_percent(),
            circuit_breaker_window_secs: default_circuit_breaker_window_secs(),
            circuit_breaker_min_requests: default_circuit_breaker_min_requests(),
            circuit_breaker_recovery_secs: default_circuit_breaker_recovery_secs(),
            circuit_breaker_half_open_requests: default_circuit_breaker_half_open_requests(),
            max_hop_depth: default_max_hop_depth(),
            hop_relevance_threshold: default_hop_relevance_threshold(),
            max_query_budget: default_max_query_budget(),
        }
    }
}

fn default_promotion_threshold() -> f32 {
    0.8
}

fn default_decay_interval() -> u64 {
    86400 // 24 hours
}

fn default_decay_rate() -> f32 {
    0.05 // 5% decay
}

fn default_optimization_trigger_count() -> usize {
    100
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            promotion_threshold: default_promotion_threshold(),
            decay_interval_secs: default_decay_interval(),
            decay_rate: default_decay_rate(),
            optimization_trigger_count: default_optimization_trigger_count(),
            layer_summary_configs: std::collections::HashMap::new(),
            reasoning: ReasoningConfig::default(),
            rlm: RlmConfig::default(),
        }
    }
}

/// Job coordination configuration.
///
/// # M-CANONICAL-DOCS
///
/// ## Purpose
/// Manages configuration for background jobs, including locks, timeouts,
/// and checkpointing behavior.
///
/// ## Fields
/// - `lock_ttl_seconds`: Time-to-live for job locks (default: 300)
/// - `job_timeout_seconds`: Maximum execution time for a job (default: 3600)
/// - `deduplication_window_seconds`: Window for job deduplication (default: 60)
/// - `checkpoint_interval`: Items processed between checkpoints (default: 100)
/// - `graceful_shutdown_timeout_seconds`: Timeout for shutdown (default: 30)
#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct JobConfig {
    /// Time-to-live for job locks in seconds
    #[serde(default = "default_lock_ttl_seconds")]
    #[validate(range(min = 1, max = 3600))]
    pub lock_ttl_seconds: u64,

    /// Maximum execution time for a job in seconds
    #[serde(default = "default_job_timeout_seconds")]
    #[validate(range(min = 1, max = 86400))]
    pub job_timeout_seconds: u64,

    /// Window for job deduplication in seconds
    #[serde(default = "default_deduplication_window_seconds")]
    #[validate(range(min = 1, max = 3600))]
    pub deduplication_window_seconds: u64,

    /// Number of items processed between checkpoints
    #[serde(default = "default_checkpoint_interval")]
    #[validate(range(min = 1, max = 10000))]
    pub checkpoint_interval: usize,

    /// Timeout for graceful shutdown in seconds
    #[serde(default = "default_graceful_shutdown_timeout_seconds")]
    #[validate(range(min = 1, max = 300))]
    pub graceful_shutdown_timeout_seconds: u64,
}

fn default_lock_ttl_seconds() -> u64 {
    300
}

fn default_job_timeout_seconds() -> u64 {
    3600
}

fn default_deduplication_window_seconds() -> u64 {
    60
}

fn default_checkpoint_interval() -> usize {
    100
}

fn default_graceful_shutdown_timeout_seconds() -> u64 {
    30
}

impl Default for JobConfig {
    fn default() -> Self {
        Self {
            lock_ttl_seconds: default_lock_ttl_seconds(),
            job_timeout_seconds: default_job_timeout_seconds(),
            deduplication_window_seconds: default_deduplication_window_seconds(),
            checkpoint_interval: default_checkpoint_interval(),
            graceful_shutdown_timeout_seconds: default_graceful_shutdown_timeout_seconds(),
        }
    }
}

impl JobConfig {
    pub fn lock_key(&self, job_name: &str) -> String {
        format!("job_lock:{}", job_name)
    }

    pub fn should_checkpoint(&self, processed_count: usize) -> bool {
        processed_count > 0 && processed_count.is_multiple_of(self.checkpoint_interval)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert_eq!(config.providers.postgres.host, "localhost");
        assert!(config.sync.enabled);
        assert_eq!(config.tools.port, 8080);
        assert_eq!(config.observability.logging_level, "info");
    }

    #[test]
    fn test_provider_config_default() {
        let providers = ProviderConfig::default();
        assert_eq!(providers.postgres.port, 5432);
        assert_eq!(providers.qdrant.port, 6333);
        assert_eq!(providers.redis.port, 6379);
    }

    #[test]
    fn test_sync_config_default() {
        let sync = SyncConfig::default();
        assert!(sync.enabled);
        assert_eq!(sync.sync_interval_seconds, 60);
        assert_eq!(sync.conflict_resolution, "prefer_knowledge");
    }

    #[test]
    fn test_postgres_config_validation() {
        let mut postgres = PostgresConfig::default();
        postgres.host = "".to_string();
        assert!(postgres.validate().is_err());
    }

    #[test]
    fn test_qdrant_config_validation() {
        let mut qdrant = QdrantConfig::default();
        qdrant.port = 0;
        assert!(qdrant.validate().is_err());
    }

    #[test]
    fn test_redis_config_validation() {
        let mut redis = RedisConfig::default();
        redis.db = 16;
        assert!(redis.validate().is_err());
    }

    #[test]
    fn test_sync_config_conflict_resolution_validation() {
        let mut sync = SyncConfig::default();
        sync.conflict_resolution = "invalid".to_string();
        assert!(sync.validate().is_err());

        sync.conflict_resolution = "prefer_memory".to_string();
        assert!(sync.validate().is_ok());
    }

    #[test]
    fn test_observability_config_logging_level_validation() {
        let mut obs = ObservabilityConfig::default();
        obs.logging_level = "invalid".to_string();
        assert!(obs.validate().is_err());

        obs.logging_level = "debug".to_string();
        assert!(obs.validate().is_ok());
    }

    #[test]
    fn test_config_serialization() {
        let config = Config::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(
            config.providers.postgres.host,
            deserialized.providers.postgres.host
        );
    }

    #[test]
    fn test_deployment_config_default() {
        let config = DeploymentConfig::default();
        assert_eq!(config.mode, "local");
        assert!(config.remote_url.is_none());
        assert!(config.sync_enabled);
    }

    #[test]
    fn test_deployment_config_is_local() {
        let config = DeploymentConfig {
            mode: "local".to_string(),
            remote_url: None,
            sync_enabled: true,
        };
        assert!(config.is_local());
        assert!(!config.is_hybrid());
        assert!(!config.is_remote());
    }

    #[test]
    fn test_deployment_config_is_hybrid() {
        let config = DeploymentConfig {
            mode: "hybrid".to_string(),
            remote_url: Some("http://localhost:8080".to_string()),
            sync_enabled: true,
        };
        assert!(!config.is_local());
        assert!(config.is_hybrid());
        assert!(!config.is_remote());
    }

    #[test]
    fn test_deployment_config_is_remote() {
        let config = DeploymentConfig {
            mode: "remote".to_string(),
            remote_url: Some("http://localhost:8080".to_string()),
            sync_enabled: false,
        };
        assert!(!config.is_local());
        assert!(!config.is_hybrid());
        assert!(config.is_remote());
    }

    #[test]
    fn test_deployment_config_requires_remote_url() {
        let local = DeploymentConfig {
            mode: "local".to_string(),
            ..Default::default()
        };
        let hybrid = DeploymentConfig {
            mode: "hybrid".to_string(),
            ..Default::default()
        };
        let remote = DeploymentConfig {
            mode: "remote".to_string(),
            ..Default::default()
        };

        assert!(!local.requires_remote_url());
        assert!(hybrid.requires_remote_url());
        assert!(remote.requires_remote_url());
    }

    #[test]
    fn test_deployment_config_requires_local_engine() {
        let local = DeploymentConfig {
            mode: "local".to_string(),
            ..Default::default()
        };
        let hybrid = DeploymentConfig {
            mode: "hybrid".to_string(),
            ..Default::default()
        };
        let remote = DeploymentConfig {
            mode: "remote".to_string(),
            ..Default::default()
        };

        assert!(local.requires_local_engine());
        assert!(hybrid.requires_local_engine());
        assert!(!remote.requires_local_engine());
    }

    #[test]
    fn test_deployment_mode_validation() {
        assert!(validate_deployment_mode("local").is_ok());
        assert!(validate_deployment_mode("hybrid").is_ok());
        assert!(validate_deployment_mode("remote").is_ok());
        assert!(validate_deployment_mode("invalid").is_err());
    }

    #[test]
    fn test_graph_config_default() {
        let config = GraphConfig::default();
        assert!(config.enabled);
        assert_eq!(config.database_path, ":memory:");
        assert!(config.s3_bucket.is_none());
        assert!(config.s3_prefix.is_none());
        assert!(config.s3_endpoint.is_none());
        assert_eq!(config.s3_region, "us-east-1");
        assert_eq!(config.snapshot_full_interval_secs, 604800);
        assert_eq!(config.snapshot_delta_interval_secs, 300);
    }

    #[test]
    fn test_graph_config_validation() {
        let mut config = GraphConfig::default();
        config.database_path = "".to_string();
        assert!(config.validate().is_err());

        config.database_path = "/path/to/graph.duckdb".to_string();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_provider_config_includes_graph() {
        let providers = ProviderConfig::default();
        assert!(providers.graph.enabled);
        assert_eq!(providers.graph.database_path, ":memory:");
        assert_eq!(providers.graph.snapshot_full_interval_secs, 604800);
        assert_eq!(providers.graph.snapshot_delta_interval_secs, 300);
    }

    // ── ProvisioningConfig (B2 task 4.1) ────────────────────────────────

    #[test]
    fn provisioning_config_default_is_off() {
        let p = ProvisioningConfig::default();
        assert!(
            !p.allow_inline_secret,
            "default must be false so a bare config rejects inline plaintext"
        );
        assert!(
            !p.effective_allow_inline_secret(),
            "effective value on a default config must be false regardless of build"
        );
    }

    #[test]
    fn provisioning_config_effective_gate_matches_build_profile() {
        // With `allow_inline_secret = true`, the effective value depends on
        // whether `debug_assertions` is active. This asserts both halves of
        // task 4.1 in one run: dev builds honor the flag, release builds
        // force it off.
        let p = ProvisioningConfig {
            allow_inline_secret: true,
        };
        if cfg!(debug_assertions) {
            assert!(
                p.effective_allow_inline_secret(),
                "debug build must honor allow_inline_secret=true"
            );
        } else {
            assert!(
                !p.effective_allow_inline_secret(),
                "release build must ignore allow_inline_secret=true \
                 (task 4.1 hard-off)"
            );
        }
    }

    #[test]
    fn provisioning_config_round_trips_through_serde() {
        // Belt-and-braces that the YAML key stays `allowInlineSecret`
        // (camelCase via top-level field rename at the Config level is
        // not in play here — this struct uses default snake/camel?),
        // and that a `false` value round-trips to `false` without the
        // serializer dropping it silently.
        //
        // NB: `ProvisioningConfig` does not set `#[serde(rename_all)]`
        // itself — fields serialize as `allow_inline_secret` today. If
        // we later harmonise everything to camelCase, update this test
        // and the operator docs in lockstep.
        let yaml = "allow_inline_secret: true\n";
        let parsed: ProvisioningConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(parsed.allow_inline_secret);

        let roundtrip = serde_yaml::to_string(&parsed).unwrap();
        assert!(
            roundtrip.contains("allow_inline_secret: true"),
            "expected serialized YAML to round-trip the flag; got:\n{roundtrip}"
        );
    }

    #[test]
    fn top_level_config_default_provisioning_is_off() {
        let cfg = Config::default();
        assert!(
            !cfg.provisioning.effective_allow_inline_secret(),
            "Config::default() must reject inline secrets by default"
        );
    }
}
