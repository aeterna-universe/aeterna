//! Cedar Agent client for authorization and entity resolution.
//!
//! This module provides a client for interacting with the Cedar Agent sidecar
//! that runs alongside the application (via OPAL Client). The Cedar Agent
//! maintains policies and entity data synchronized by OPAL.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │  OPAL Client (Sidecar)                                  │
//! │  ┌─────────────────────────────────────────────────┐   │
//! │  │  Cedar Agent (localhost:8180)                    │   │
//! │  │  - GET /v1/data (entities)                       │   │
//! │  │  - POST /v1/is_authorized (authz check)         │   │
//! │  └─────────────────────────────────────────────────┘   │
//! └─────────────────────────────────────────────────────────┘
//!                           ▲
//!                           │ HTTP (localhost)
//!                           │
//! ┌─────────────────────────┴───────────────────────────────┐
//! │  Aeterna Application                                     │
//! │  - CedarClient queries local Cedar Agent                │
//! │  - Low latency (no network hop)                         │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use context::cedar::{CedarClient, CedarConfig};
//!
//! let config = CedarConfig::default(); // localhost:8180
//! let client = CedarClient::new(config);
//!
//! // Resolve user by email
//! let user = client.resolve_user_by_email("alice@acme.com").await?;
//!
//! // Check authorization
//! let allowed = client.check_authorization(
//!     "Aeterna::User::\"user-uuid\"",
//!     "Aeterna::Action::\"ViewKnowledge\"",
//!     "Aeterna::Project::\"project-uuid\"",
//!     None,
//! ).await?;
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, trace, warn};

// ============================================================================
// Error Types
// ============================================================================

/// Errors that can occur when interacting with Cedar Agent.
#[derive(Debug, Error)]
pub enum CedarError {
    /// HTTP request failed.
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    /// Cedar Agent returned an error response.
    #[error("Cedar Agent error: {status} - {message}")]
    AgentError { status: u16, message: String },

    /// Failed to parse Cedar Agent response.
    #[error("Failed to parse response: {0}")]
    ParseError(String),

    /// Entity not found.
    #[error("Entity not found: {entity_type}::{id}")]
    EntityNotFound { entity_type: String, id: String },

    /// Authorization denied.
    #[error("Authorization denied: {reason}")]
    AuthorizationDenied { reason: String },

    /// Cedar Agent is unavailable (circuit breaker open).
    #[error("Cedar Agent unavailable: {0}")]
    Unavailable(String),

    /// Configuration error.
    #[error("Configuration error: {0}")]
    ConfigError(String),
}

pub type Result<T> = std::result::Result<T, CedarError>;

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for Cedar Agent client.
#[derive(Debug, Clone)]
pub struct CedarConfig {
    /// Base URL of the Cedar Agent (default: http://localhost:8180).
    pub base_url: String,

    /// Request timeout in seconds.
    pub timeout_secs: u64,

    /// Maximum number of retries for transient failures.
    pub max_retries: u32,

    /// Enable local caching of entities.
    pub cache_enabled: bool,

    /// Cache TTL in seconds.
    pub cache_ttl_secs: u64,

    /// Circuit breaker failure threshold.
    pub circuit_breaker_threshold: u32,

    /// Circuit breaker reset timeout in seconds.
    pub circuit_breaker_reset_secs: u64,
}

impl Default for CedarConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:8180".to_string(),
            timeout_secs: 5,
            max_retries: 3,
            cache_enabled: true,
            cache_ttl_secs: 300, // 5 minutes
            circuit_breaker_threshold: 5,
            circuit_breaker_reset_secs: 30,
        }
    }
}

impl CedarConfig {
    /// Create config from environment variables.
    ///
    /// Reads:
    /// - `CEDAR_AGENT_URL` (default: http://localhost:8180)
    /// - `CEDAR_AGENT_TIMEOUT` (default: 5)
    /// - `CEDAR_AGENT_CACHE_ENABLED` (default: true)
    #[must_use]
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(url) = std::env::var("CEDAR_AGENT_URL") {
            config.base_url = url;
        }

        if let Ok(timeout) = std::env::var("CEDAR_AGENT_TIMEOUT")
            && let Ok(secs) = timeout.parse()
        {
            config.timeout_secs = secs;
        }

        if let Ok(cache) = std::env::var("CEDAR_AGENT_CACHE_ENABLED") {
            config.cache_enabled = cache.parse().unwrap_or(true);
        }

        config
    }

    /// Create config for testing (shorter timeouts).
    #[must_use]
    pub fn for_testing() -> Self {
        Self {
            timeout_secs: 1,
            max_retries: 1,
            cache_enabled: false,
            circuit_breaker_threshold: 2,
            circuit_breaker_reset_secs: 5,
            ..Default::default()
        }
    }
}

// ============================================================================
// Cedar Agent API Types
// ============================================================================

/// Entity UID in Cedar format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct EntityUid {
    #[serde(rename = "type")]
    pub entity_type: String,
    pub id: String,
}

impl EntityUid {
    /// Create a new entity UID.
    #[must_use]
    pub fn new(entity_type: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            entity_type: entity_type.into(),
            id: id.into(),
        }
    }

    /// Create a User entity UID.
    #[must_use]
    pub fn user(id: impl Into<String>) -> Self {
        Self::new("Aeterna::User", id)
    }

    /// Create an Agent entity UID.
    #[must_use]
    pub fn agent(id: impl Into<String>) -> Self {
        Self::new("Aeterna::Agent", id)
    }

    /// Create a Project entity UID.
    #[must_use]
    pub fn project(id: impl Into<String>) -> Self {
        Self::new("Aeterna::Project", id)
    }

    /// Create a Team entity UID.
    #[must_use]
    pub fn team(id: impl Into<String>) -> Self {
        Self::new("Aeterna::Team", id)
    }

    /// Create an Organization entity UID.
    #[must_use]
    pub fn organization(id: impl Into<String>) -> Self {
        Self::new("Aeterna::Organization", id)
    }

    /// Create a Tenant entity UID.
    #[must_use]
    pub fn tenant(id: impl Into<String>) -> Self {
        Self::new("Aeterna::Tenant", id)
    }

    /// Create an Action entity UID.
    #[must_use]
    pub fn action(name: impl Into<String>) -> Self {
        Self::new("Aeterna::Action", name)
    }

    /// Format as Cedar entity reference string: `Type::"id"`.
    #[must_use]
    pub fn to_cedar_string(&self) -> String {
        format!("{}::\"{}\"", self.entity_type, self.id)
    }
}

impl std::fmt::Display for EntityUid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}::\"{}\"", self.entity_type, self.id)
    }
}

/// A Cedar entity with attributes and parent relationships.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    /// Entity UID.
    pub uid: EntityUid,

    /// Entity attributes (JSON object).
    pub attrs: serde_json::Value,

    /// Parent entity UIDs (for hierarchy).
    #[serde(default)]
    pub parents: Vec<EntityUid>,
}

impl Entity {
    /// Get an attribute value as a string.
    #[must_use]
    pub fn get_attr_str(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).and_then(|v| v.as_str())
    }

    /// Get an attribute value as a string array.
    #[must_use]
    pub fn get_attr_str_array(&self, key: &str) -> Option<Vec<&str>> {
        self.attrs.get(key).and_then(|v| {
            v.as_array()
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        })
    }
}

/// Authorization request to Cedar Agent.
#[derive(Debug, Clone, Serialize)]
pub struct AuthorizationRequest {
    /// Principal making the request (e.g., `Aeterna::User::"user-id"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal: Option<String>,

    /// Action being performed (e.g., `Aeterna::Action::"ViewKnowledge"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,

    /// Resource being accessed (e.g., `Aeterna::Project::"project-id"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,

    /// Additional context for the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,

    /// Inline entities to use for this request (overrides stored data).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entities: Option<Vec<Entity>>,

    /// Additional entities to merge with stored data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_entities: Option<Vec<Entity>>,
}

impl AuthorizationRequest {
    /// Create a new authorization request.
    #[must_use]
    pub fn new() -> Self {
        Self {
            principal: None,
            action: None,
            resource: None,
            context: None,
            entities: None,
            additional_entities: None,
        }
    }

    /// Set the principal.
    #[must_use]
    pub fn with_principal(mut self, principal: &EntityUid) -> Self {
        self.principal = Some(principal.to_cedar_string());
        self
    }

    /// Set the action.
    #[must_use]
    pub fn with_action(mut self, action: &str) -> Self {
        self.action = Some(EntityUid::action(action).to_cedar_string());
        self
    }

    /// Set the resource.
    #[must_use]
    pub fn with_resource(mut self, resource: &EntityUid) -> Self {
        self.resource = Some(resource.to_cedar_string());
        self
    }

    /// Set the context.
    #[must_use]
    pub fn with_context(mut self, context: serde_json::Value) -> Self {
        self.context = Some(context);
        self
    }
}

impl Default for AuthorizationRequest {
    fn default() -> Self {
        Self::new()
    }
}

/// Authorization response from Cedar Agent.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthorizationResponse {
    /// The authorization decision.
    pub decision: AuthorizationDecision,

    /// Diagnostic information.
    #[serde(default)]
    pub diagnostics: AuthorizationDiagnostics,
}

/// Authorization decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum AuthorizationDecision {
    /// Request is allowed.
    Allow,
    /// Request is denied.
    Deny,
}

/// Diagnostic information from authorization.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AuthorizationDiagnostics {
    /// Policy IDs that contributed to the decision.
    #[serde(default)]
    pub reason: Vec<String>,

    /// Error messages (if any).
    #[serde(default)]
    pub errors: Vec<String>,
}

// ============================================================================
// Circuit Breaker
// ============================================================================

/// Circuit breaker state for resilience.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

struct CircuitBreaker {
    state: CircuitState,
    failure_count: u32,
    last_failure: Option<std::time::Instant>,
    threshold: u32,
    reset_timeout: Duration,
}

impl CircuitBreaker {
    fn new(threshold: u32, reset_timeout: Duration) -> Self {
        Self {
            state: CircuitState::Closed,
            failure_count: 0,
            last_failure: None,
            threshold,
            reset_timeout,
        }
    }

    fn can_execute(&mut self) -> bool {
        match self.state {
            CircuitState::Open => {
                if let Some(last) = self.last_failure {
                    if last.elapsed() >= self.reset_timeout {
                        self.state = CircuitState::HalfOpen;
                        true
                    } else {
                        false
                    }
                } else {
                    true
                }
            }
            CircuitState::Closed | CircuitState::HalfOpen => true,
        }
    }

    fn record_success(&mut self) {
        self.failure_count = 0;
        self.state = CircuitState::Closed;
    }

    fn record_failure(&mut self) {
        self.failure_count += 1;
        self.last_failure = Some(std::time::Instant::now());

        if self.failure_count >= self.threshold {
            self.state = CircuitState::Open;
            warn!(
                "Circuit breaker opened after {} failures",
                self.failure_count
            );
        }
    }
}

// ============================================================================
// Entity Cache
// ============================================================================

struct EntityCache {
    entities: HashMap<EntityUid, (Entity, std::time::Instant)>,
    ttl: Duration,
}

impl EntityCache {
    fn new(ttl: Duration) -> Self {
        Self {
            entities: HashMap::new(),
            ttl,
        }
    }

    fn get(&self, uid: &EntityUid) -> Option<&Entity> {
        self.entities.get(uid).and_then(|(entity, inserted)| {
            if inserted.elapsed() < self.ttl {
                Some(entity)
            } else {
                None
            }
        })
    }

    fn insert(&mut self, entity: Entity) {
        self.entities
            .insert(entity.uid.clone(), (entity, std::time::Instant::now()));
    }

    fn insert_all(&mut self, entities: Vec<Entity>) {
        for entity in entities {
            self.insert(entity);
        }
    }

    fn clear(&mut self) {
        self.entities.clear();
    }
}

// ============================================================================
// Cedar Client
// ============================================================================

/// Client for interacting with Cedar Agent sidecar.
///
/// Provides methods for:
/// - Entity resolution (users, projects, teams, etc.)
/// - Authorization checks
/// - Accessible layer discovery
///
/// Includes circuit breaker for resilience and local caching for performance.
pub struct CedarClient {
    config: CedarConfig,
    http: Client,
    circuit_breaker: Arc<RwLock<CircuitBreaker>>,
    cache: Arc<RwLock<EntityCache>>,
}

impl CedarClient {
    /// Create a new Cedar client.
    ///
    /// # Panics
    /// Panics if HTTP client cannot be created.
    #[must_use]
    pub fn new(config: CedarConfig) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .expect("Failed to create HTTP client");

        let circuit_breaker = CircuitBreaker::new(
            config.circuit_breaker_threshold,
            Duration::from_secs(config.circuit_breaker_reset_secs),
        );

        let cache = EntityCache::new(Duration::from_secs(config.cache_ttl_secs));

        Self {
            config,
            http,
            circuit_breaker: Arc::new(RwLock::new(circuit_breaker)),
            cache: Arc::new(RwLock::new(cache)),
        }
    }

    /// Create a client with default configuration.
    #[must_use]
    pub fn default_client() -> Self {
        Self::new(CedarConfig::default())
    }

    /// Create a client from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        Self::new(CedarConfig::from_env())
    }

    // ========================================================================
    // Health Check
    // ========================================================================

    /// Check if Cedar Agent is healthy.
    pub async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/health", self.config.base_url);

        match self.http.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => Ok(true),
            Ok(resp) => {
                warn!("Cedar Agent health check failed: {}", resp.status());
                Ok(false)
            }
            Err(e) => {
                warn!("Cedar Agent health check error: {}", e);
                Ok(false)
            }
        }
    }

    // ========================================================================
    // Entity Resolution
    // ========================================================================

    /// Fetch all entities from Cedar Agent.
    ///
    /// This retrieves all entities loaded into Cedar Agent's data store.
    pub async fn get_all_entities(&self) -> Result<Vec<Entity>> {
        self.execute_with_circuit_breaker(|| async {
            let url = format!("{}/v1/data", self.config.base_url);

            let resp = self.http.get(&url).send().await?;

            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                let message = resp.text().await.unwrap_or_default();
                return Err(CedarError::AgentError { status, message });
            }

            let entities: Vec<Entity> = resp
                .json()
                .await
                .map_err(|e| CedarError::ParseError(format!("Failed to parse entities: {e}")))?;

            // Update cache
            if self.config.cache_enabled {
                let mut cache = self.cache.write().await;
                cache.insert_all(entities.clone());
            }

            Ok(entities)
        })
        .await
    }

    /// Resolve a user by email address.
    ///
    /// Searches the entities for a User with matching email attribute.
    pub async fn resolve_user_by_email(&self, email: &str) -> Result<Entity> {
        debug!("Resolving user by email: {}", email);

        // Check cache first
        if self.config.cache_enabled {
            let cache = self.cache.read().await;
            for (uid, (entity, _)) in &cache.entities {
                if uid.entity_type == "Aeterna::User"
                    && let Some(e) = entity.get_attr_str("email")
                    && e == email
                {
                    trace!("User found in cache: {}", uid);
                    return Ok(entity.clone());
                }
            }
        }

        // Fetch from Cedar Agent
        let entities = self.get_all_entities().await?;

        entities
            .into_iter()
            .find(|e| {
                e.uid.entity_type == "Aeterna::User" && e.get_attr_str("email") == Some(email)
            })
            .ok_or_else(|| CedarError::EntityNotFound {
                entity_type: "Aeterna::User".to_string(),
                id: format!("email={email}"),
            })
    }

    /// Resolve a project by git remote URL.
    ///
    /// Searches the entities for a Project with matching git_remote attribute.
    pub async fn resolve_project_by_git_remote(&self, git_remote: &str) -> Result<Entity> {
        debug!("Resolving project by git remote: {}", git_remote);

        // Check cache first
        if self.config.cache_enabled {
            let cache = self.cache.read().await;
            for (uid, (entity, _)) in &cache.entities {
                if uid.entity_type == "Aeterna::Project"
                    && let Some(remote) = entity.get_attr_str("git_remote")
                    && remote == git_remote
                {
                    trace!("Project found in cache: {}", uid);
                    return Ok(entity.clone());
                }
            }
        }

        // Fetch from Cedar Agent
        let entities = self.get_all_entities().await?;

        entities
            .into_iter()
            .find(|e| {
                e.uid.entity_type == "Aeterna::Project"
                    && e.get_attr_str("git_remote") == Some(git_remote)
            })
            .ok_or_else(|| CedarError::EntityNotFound {
                entity_type: "Aeterna::Project".to_string(),
                id: format!("git_remote={git_remote}"),
            })
    }

    /// Resolve an entity by its UID.
    pub async fn resolve_entity(&self, uid: &EntityUid) -> Result<Entity> {
        debug!("Resolving entity: {}", uid);

        // Check cache first
        if self.config.cache_enabled {
            let cache = self.cache.read().await;
            if let Some(entity) = cache.get(uid) {
                trace!("Entity found in cache: {}", uid);
                return Ok(entity.clone());
            }
        }

        // Fetch from Cedar Agent
        let entities = self.get_all_entities().await?;

        entities
            .into_iter()
            .find(|e| e.uid == *uid)
            .ok_or_else(|| CedarError::EntityNotFound {
                entity_type: uid.entity_type.clone(),
                id: uid.id.clone(),
            })
    }

    /// Resolve an agent by ID.
    pub async fn resolve_agent(&self, agent_id: &str) -> Result<Entity> {
        self.resolve_entity(&EntityUid::agent(agent_id)).await
    }

    // ========================================================================
    // Authorization
    // ========================================================================

    /// Check if an action is authorized.
    ///
    /// # Arguments
    ///
    /// * `principal` - The entity making the request (User or Agent)
    /// * `action` - The action being performed (e.g., "ViewKnowledge")
    /// * `resource` - The resource being accessed (e.g., Project)
    /// * `context` - Optional additional context
    ///
    /// # Returns
    ///
    /// `Ok(true)` if allowed, `Ok(false)` if denied.
    pub async fn check_authorization(
        &self,
        principal: &EntityUid,
        action: &str,
        resource: &EntityUid,
        context: Option<serde_json::Value>,
    ) -> Result<bool> {
        let request = AuthorizationRequest::new()
            .with_principal(principal)
            .with_action(action)
            .with_resource(resource);

        let request = if let Some(ctx) = context {
            request.with_context(ctx)
        } else {
            request
        };

        let response = self.authorize(request).await?;

        Ok(response.decision == AuthorizationDecision::Allow)
    }

    /// Perform an authorization request with full diagnostic info.
    pub async fn authorize(&self, request: AuthorizationRequest) -> Result<AuthorizationResponse> {
        debug!(
            "Authorization request: principal={:?} action={:?} resource={:?}",
            request.principal, request.action, request.resource
        );

        self.execute_with_circuit_breaker(|| async {
            let url = format!("{}/v1/is_authorized", self.config.base_url);

            let resp = self.http.post(&url).json(&request).send().await?;

            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                let message = resp.text().await.unwrap_or_default();
                return Err(CedarError::AgentError { status, message });
            }

            let response: AuthorizationResponse = resp.json().await.map_err(|e| {
                CedarError::ParseError(format!("Failed to parse authorization response: {e}"))
            })?;

            debug!(
                "Authorization response: decision={:?} reason={:?}",
                response.decision, response.diagnostics.reason
            );

            Ok(response)
        })
        .await
    }

    /// Check if user can perform action on any resource of a given type.
    ///
    /// Useful for UI to show/hide features based on permissions.
    pub async fn can_user_perform(&self, user_id: &str, action: &str) -> Result<bool> {
        // For now, we check against a wildcard resource
        // Cedar Agent may support this differently
        let request = AuthorizationRequest {
            principal: Some(EntityUid::user(user_id).to_cedar_string()),
            action: Some(EntityUid::action(action).to_cedar_string()),
            resource: None,
            context: None,
            entities: None,
            additional_entities: None,
        };

        let response = self.authorize(request).await?;
        Ok(response.decision == AuthorizationDecision::Allow)
    }

    // ========================================================================
    // Layer Discovery
    // ========================================================================

    /// Get accessible memory layers for a user.
    ///
    /// Returns the hierarchy of accessible entities (Tenant -> Org -> Team ->
    /// Project).
    pub async fn get_accessible_layers(&self, user_id: &str) -> Result<AccessibleLayers> {
        let user = self.resolve_entity(&EntityUid::user(user_id)).await?;
        let all_entities = self.get_all_entities().await?;

        let mut layers = AccessibleLayers::default();

        // Find all teams the user is a member of (via parents)
        for parent in &user.parents {
            if parent.entity_type == "Aeterna::Team" {
                layers.team_ids.push(parent.id.clone());

                // Find the team to get its org
                if let Some(team) = all_entities.iter().find(|e| e.uid == *parent) {
                    for team_parent in &team.parents {
                        if team_parent.entity_type == "Aeterna::Organization" {
                            if !layers.org_ids.contains(&team_parent.id) {
                                layers.org_ids.push(team_parent.id.clone());
                            }

                            // Find the org to get its tenant root
                            if let Some(org) = all_entities.iter().find(|e| e.uid == *team_parent) {
                                for org_parent in &org.parents {
                                    if org_parent.entity_type == "Aeterna::Tenant"
                                        && !layers.tenant_ids.contains(&org_parent.id)
                                    {
                                        layers.tenant_ids.push(org_parent.id.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Find all projects in accessible teams
        for entity in &all_entities {
            if entity.uid.entity_type == "Aeterna::Project" {
                for parent in &entity.parents {
                    if parent.entity_type == "Aeterna::Team" && layers.team_ids.contains(&parent.id)
                    {
                        layers.project_ids.push(entity.uid.id.clone());
                        break;
                    }
                }
            }
        }

        Ok(layers)
    }

    // ========================================================================
    // Cache Management
    // ========================================================================

    /// Clear the entity cache.
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }

    /// Refresh the entity cache from Cedar Agent.
    pub async fn refresh_cache(&self) -> Result<()> {
        self.clear_cache().await;
        let _ = self.get_all_entities().await?;
        Ok(())
    }

    // ========================================================================
    // Circuit Breaker
    // ========================================================================

    async fn execute_with_circuit_breaker<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        {
            let mut cb = self.circuit_breaker.write().await;
            if !cb.can_execute() {
                return Err(CedarError::Unavailable(
                    "Circuit breaker is open".to_string(),
                ));
            }
        }

        let max_retries = self.config.max_retries;
        let mut last_error = None;

        for attempt in 1..=max_retries {
            match f().await {
                Ok(v) => {
                    let mut cb = self.circuit_breaker.write().await;
                    cb.record_success();
                    return Ok(v);
                }
                Err(e) => {
                    if Self::is_transient_error(&e) && attempt < max_retries {
                        warn!(
                            "Transient error (attempt {}/{}): {}",
                            attempt, max_retries, e
                        );
                        let delay = Duration::from_millis(100 * (1 << (attempt - 1)));
                        tokio::time::sleep(delay).await;
                        last_error = Some(e);
                    } else {
                        let mut cb = self.circuit_breaker.write().await;
                        cb.record_failure();
                        return Err(e);
                    }
                }
            }
        }

        let mut cb = self.circuit_breaker.write().await;
        cb.record_failure();
        Err(last_error.unwrap_or_else(|| {
            CedarError::Unavailable(format!("Max retries ({max_retries}) exceeded"))
        }))
    }

    fn is_transient_error(e: &CedarError) -> bool {
        matches!(
            e,
            CedarError::HttpError(_)
                | CedarError::AgentError {
                    status: 502..=504,
                    ..
                }
        )
    }
}

// ============================================================================
// Accessible Layers
// ============================================================================

/// Represents the layers accessible to a user.
#[derive(Debug, Clone, Default)]
pub struct AccessibleLayers {
    /// Accessible tenant IDs.
    pub tenant_ids: Vec<String>,

    /// Accessible organization IDs.
    pub org_ids: Vec<String>,

    /// Accessible team IDs.
    pub team_ids: Vec<String>,

    /// Accessible project IDs.
    pub project_ids: Vec<String>,
}

impl AccessibleLayers {
    /// Check if any layers are accessible.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tenant_ids.is_empty()
            && self.org_ids.is_empty()
            && self.team_ids.is_empty()
            && self.project_ids.is_empty()
    }

    /// Get total count of accessible entities.
    #[must_use]
    pub fn total_count(&self) -> usize {
        self.tenant_ids.len() + self.org_ids.len() + self.team_ids.len() + self.project_ids.len()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entity_uid_constructors() {
        let user = EntityUid::user("123");
        assert_eq!(user.entity_type, "Aeterna::User");
        assert_eq!(user.id, "123");

        let project = EntityUid::project("proj-1");
        assert_eq!(project.entity_type, "Aeterna::Project");

        let action = EntityUid::action("ViewKnowledge");
        assert_eq!(action.entity_type, "Aeterna::Action");
        assert_eq!(action.id, "ViewKnowledge");
    }

    #[test]
    fn test_entity_uid_to_cedar_string() {
        let uid = EntityUid::user("alice-123");
        assert_eq!(uid.to_cedar_string(), "Aeterna::User::\"alice-123\"");

        let action = EntityUid::action("EditKnowledge");
        assert_eq!(
            action.to_cedar_string(),
            "Aeterna::Action::\"EditKnowledge\""
        );
    }

    #[test]
    fn test_entity_uid_display() {
        let uid = EntityUid::new("Aeterna::Team", "team-1");
        assert_eq!(format!("{uid}"), "Aeterna::Team::\"team-1\"");
    }

    #[test]
    fn test_authorization_request_builder() {
        let principal = EntityUid::user("user-1");
        let resource = EntityUid::project("proj-1");

        let request = AuthorizationRequest::new()
            .with_principal(&principal)
            .with_action("ViewKnowledge")
            .with_resource(&resource)
            .with_context(serde_json::json!({"ip": "192.168.1.1"}));

        assert_eq!(
            request.principal,
            Some("Aeterna::User::\"user-1\"".to_string())
        );
        assert_eq!(
            request.action,
            Some("Aeterna::Action::\"ViewKnowledge\"".to_string())
        );
        assert_eq!(
            request.resource,
            Some("Aeterna::Project::\"proj-1\"".to_string())
        );
        assert!(request.context.is_some());
    }

    #[test]
    fn test_cedar_config_default() {
        let config = CedarConfig::default();
        assert_eq!(config.base_url, "http://localhost:8180");
        assert_eq!(config.timeout_secs, 5);
        assert!(config.cache_enabled);
    }

    #[test]
    fn test_cedar_config_for_testing() {
        let config = CedarConfig::for_testing();
        assert_eq!(config.timeout_secs, 1);
        assert_eq!(config.max_retries, 1);
        assert!(!config.cache_enabled);
    }

    #[test]
    fn test_entity_get_attr_str() {
        let entity = Entity {
            uid: EntityUid::user("test"),
            attrs: serde_json::json!({
                "email": "test@example.com",
                "name": "Test User",
                "count": 42
            }),
            parents: vec![],
        };

        assert_eq!(entity.get_attr_str("email"), Some("test@example.com"));
        assert_eq!(entity.get_attr_str("name"), Some("Test User"));
        assert_eq!(entity.get_attr_str("count"), None); // Not a string
        assert_eq!(entity.get_attr_str("missing"), None);
    }

    #[test]
    fn test_entity_get_attr_str_array() {
        let entity = Entity {
            uid: EntityUid::user("test"),
            attrs: serde_json::json!({
                "roles": ["admin", "developer"],
                "single": "value"
            }),
            parents: vec![],
        };

        let roles = entity.get_attr_str_array("roles");
        assert_eq!(roles, Some(vec!["admin", "developer"]));

        let single = entity.get_attr_str_array("single");
        assert_eq!(single, None); // Not an array
    }

    #[test]
    fn test_circuit_breaker_initial_state() {
        let mut cb = CircuitBreaker::new(3, Duration::from_secs(30));
        assert!(cb.can_execute());
        assert_eq!(cb.state, CircuitState::Closed);
    }

    #[test]
    fn test_circuit_breaker_opens_after_threshold() {
        let mut cb = CircuitBreaker::new(3, Duration::from_secs(30));

        cb.record_failure();
        assert!(cb.can_execute());

        cb.record_failure();
        assert!(cb.can_execute());

        cb.record_failure();
        assert!(!cb.can_execute());
        assert_eq!(cb.state, CircuitState::Open);
    }

    #[test]
    fn test_circuit_breaker_success_resets() {
        let mut cb = CircuitBreaker::new(3, Duration::from_secs(30));

        cb.record_failure();
        cb.record_failure();
        cb.record_success();

        assert_eq!(cb.failure_count, 0);
        assert_eq!(cb.state, CircuitState::Closed);
    }

    #[test]
    fn test_accessible_layers_is_empty() {
        let layers = AccessibleLayers::default();
        assert!(layers.is_empty());

        let layers = AccessibleLayers {
            team_ids: vec!["team-1".to_string()],
            ..Default::default()
        };
        assert!(!layers.is_empty());
    }

    #[test]
    fn test_accessible_layers_total_count() {
        let layers = AccessibleLayers {
            tenant_ids: vec!["c1".to_string()],
            org_ids: vec!["o1".to_string(), "o2".to_string()],
            team_ids: vec!["t1".to_string()],
            project_ids: vec!["p1".to_string(), "p2".to_string(), "p3".to_string()],
        };
        assert_eq!(layers.total_count(), 7);
    }

    #[test]
    fn test_authorization_decision_deserialize() {
        let allow: AuthorizationDecision = serde_json::from_str("\"Allow\"").unwrap();
        assert_eq!(allow, AuthorizationDecision::Allow);

        let deny: AuthorizationDecision = serde_json::from_str("\"Deny\"").unwrap();
        assert_eq!(deny, AuthorizationDecision::Deny);
    }

    #[test]
    fn test_authorization_response_deserialize() {
        let json = r#"{
            "decision": "Allow",
            "diagnostics": {
                "reason": ["policy-1", "policy-2"],
                "errors": []
            }
        }"#;

        let response: AuthorizationResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.decision, AuthorizationDecision::Allow);
        assert_eq!(response.diagnostics.reason.len(), 2);
        assert!(response.diagnostics.errors.is_empty());
    }
}

// ============================================================================
// Integration Tests (with wiremock)
// ============================================================================

#[cfg(test)]
mod integration_tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Helper to create test entities matching OPAL fetcher format.
    fn create_test_entities() -> serde_json::Value {
        serde_json::json!([
            {
                "uid": {"type": "Aeterna::Tenant", "id": "acme-corp"},
                "attrs": {"name": "Acme Corporation", "slug": "acme"},
                "parents": []
            },
            {
                "uid": {"type": "Aeterna::Organization", "id": "org-platform"},
                "attrs": {"name": "Platform Engineering"},
                "parents": [{"type": "Aeterna::Tenant", "id": "acme-corp"}]
            },
            {
                "uid": {"type": "Aeterna::Team", "id": "team-api"},
                "attrs": {"name": "API Team"},
                "parents": [{"type": "Aeterna::Organization", "id": "org-platform"}]
            },
            {
                "uid": {"type": "Aeterna::Project", "id": "proj-payments"},
                "attrs": {"name": "Payments Service", "git_remote": "github.com/acme/payments"},
                "parents": [{"type": "Aeterna::Team", "id": "team-api"}]
            },
            {
                "uid": {"type": "Aeterna::User", "id": "user-alice"},
                "attrs": {"email": "alice@acme.com", "name": "Alice", "tenant_slug": "acme"},
                "parents": [{"type": "Aeterna::Team", "id": "team-api"}]
            },
            {
                "uid": {"type": "Aeterna::Agent", "id": "agent-opencode"},
                "attrs": {"name": "OpenCode Assistant"},
                "parents": [
                    {"type": "Aeterna::User", "id": "user-alice"},
                    {"type": "Aeterna::Project", "id": "proj-payments"}
                ]
            }
        ])
    }

    fn create_client_for_mock(mock_server: &MockServer) -> CedarClient {
        let config = CedarConfig {
            base_url: mock_server.uri(),
            timeout_secs: 5,
            cache_enabled: false,
            cache_ttl_secs: 60,
            max_retries: 1,
            circuit_breaker_threshold: 5,
            circuit_breaker_reset_secs: 30,
        };
        CedarClient::new(config)
    }

    #[tokio::test]
    async fn test_health_check_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
            .mount(&mock_server)
            .await;

        let client = create_client_for_mock(&mock_server);
        let result = client.health_check().await;

        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn test_health_check_failure() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let client = create_client_for_mock(&mock_server);
        let result = client.health_check().await;

        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_get_all_entities() {
        let mock_server = MockServer::start().await;
        let entities = create_test_entities();

        Mock::given(method("GET"))
            .and(path("/v1/data"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&entities))
            .mount(&mock_server)
            .await;

        let client = create_client_for_mock(&mock_server);
        let result = client.get_all_entities().await;

        assert!(result.is_ok());
        let entities = result.unwrap();
        assert_eq!(entities.len(), 6);

        // Verify entity types
        let types: Vec<&str> = entities
            .iter()
            .map(|e| e.uid.entity_type.as_str())
            .collect();
        assert!(types.contains(&"Aeterna::Tenant"));
        assert!(types.contains(&"Aeterna::Organization"));
        assert!(types.contains(&"Aeterna::Team"));
        assert!(types.contains(&"Aeterna::Project"));
        assert!(types.contains(&"Aeterna::User"));
        assert!(types.contains(&"Aeterna::Agent"));
    }

    #[tokio::test]
    async fn test_resolve_user_by_email() {
        let mock_server = MockServer::start().await;
        let entities = create_test_entities();

        Mock::given(method("GET"))
            .and(path("/v1/data"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&entities))
            .mount(&mock_server)
            .await;

        let client = create_client_for_mock(&mock_server);
        let result = client.resolve_user_by_email("alice@acme.com").await;

        assert!(result.is_ok());
        let user = result.unwrap();
        assert_eq!(user.uid.entity_type, "Aeterna::User");
        assert_eq!(user.uid.id, "user-alice");
        assert_eq!(user.get_attr_str("email"), Some("alice@acme.com"));
        assert_eq!(user.get_attr_str("tenant_slug"), Some("acme"));
    }

    #[tokio::test]
    async fn test_resolve_user_by_email_not_found() {
        let mock_server = MockServer::start().await;
        let entities = create_test_entities();

        Mock::given(method("GET"))
            .and(path("/v1/data"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&entities))
            .mount(&mock_server)
            .await;

        let client = create_client_for_mock(&mock_server);
        let result = client.resolve_user_by_email("unknown@example.com").await;

        assert!(result.is_err());
        match result.unwrap_err() {
            CedarError::EntityNotFound { entity_type, id } => {
                assert_eq!(entity_type, "Aeterna::User");
                assert_eq!(id, "email=unknown@example.com");
            }
            other => panic!("Expected EntityNotFound, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_resolve_project_by_git_remote() {
        let mock_server = MockServer::start().await;
        let entities = create_test_entities();

        Mock::given(method("GET"))
            .and(path("/v1/data"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&entities))
            .mount(&mock_server)
            .await;

        let client = create_client_for_mock(&mock_server);
        let result = client
            .resolve_project_by_git_remote("github.com/acme/payments")
            .await;

        assert!(result.is_ok());
        let project = result.unwrap();
        assert_eq!(project.uid.entity_type, "Aeterna::Project");
        assert_eq!(project.uid.id, "proj-payments");
        assert_eq!(project.get_attr_str("name"), Some("Payments Service"));

        // Verify parent relationship
        assert!(!project.parents.is_empty());
        assert_eq!(project.parents[0].entity_type, "Aeterna::Team");
        assert_eq!(project.parents[0].id, "team-api");
    }

    #[tokio::test]
    async fn test_check_authorization_allow() {
        let mock_server = MockServer::start().await;

        let auth_response = serde_json::json!({
            "decision": "Allow",
            "diagnostics": {
                "reason": ["rbac-policy"],
                "errors": []
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/is_authorized"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&auth_response))
            .mount(&mock_server)
            .await;

        let client = create_client_for_mock(&mock_server);
        let principal = EntityUid::user("user-alice");
        let resource = EntityUid::project("proj-payments");

        let result = client
            .check_authorization(&principal, "ViewKnowledge", &resource, None)
            .await;

        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn test_check_authorization_deny() {
        let mock_server = MockServer::start().await;

        let auth_response = serde_json::json!({
            "decision": "Deny",
            "diagnostics": {
                "reason": [],
                "errors": []
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/is_authorized"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&auth_response))
            .mount(&mock_server)
            .await;

        let client = create_client_for_mock(&mock_server);
        let principal = EntityUid::user("user-bob");
        let resource = EntityUid::project("proj-secret");

        let result = client
            .check_authorization(&principal, "EditKnowledge", &resource, None)
            .await;

        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_check_authorization_with_context() {
        let mock_server = MockServer::start().await;

        let auth_response = serde_json::json!({
            "decision": "Allow",
            "diagnostics": {
                "reason": ["abac-policy"],
                "errors": []
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/is_authorized"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&auth_response))
            .mount(&mock_server)
            .await;

        let client = create_client_for_mock(&mock_server);
        let principal = EntityUid::user("user-alice");
        let resource = EntityUid::project("proj-payments");
        let context = serde_json::json!({"ip": "192.168.1.1", "time": "09:00"});

        let result = client
            .check_authorization(&principal, "ViewKnowledge", &resource, Some(context))
            .await;

        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn test_get_accessible_layers() {
        let mock_server = MockServer::start().await;
        let entities = create_test_entities();

        Mock::given(method("GET"))
            .and(path("/v1/data"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&entities))
            .mount(&mock_server)
            .await;

        let client = create_client_for_mock(&mock_server);
        let result = client.get_accessible_layers("user-alice").await;

        assert!(result.is_ok());
        let layers = result.unwrap();

        // Alice is member of team-api
        assert!(layers.team_ids.contains(&"team-api".to_string()));

        // team-api is in org-platform
        assert!(layers.org_ids.contains(&"org-platform".to_string()));

        // org-platform is in acme-corp
        assert!(layers.tenant_ids.contains(&"acme-corp".to_string()));

        // proj-payments is in team-api
        assert!(layers.project_ids.contains(&"proj-payments".to_string()));

        assert!(!layers.is_empty());
        assert!(layers.total_count() >= 4);
    }

    #[tokio::test]
    async fn test_resolve_agent() {
        let mock_server = MockServer::start().await;
        let entities = create_test_entities();

        Mock::given(method("GET"))
            .and(path("/v1/data"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&entities))
            .mount(&mock_server)
            .await;

        let client = create_client_for_mock(&mock_server);
        let result = client.resolve_agent("agent-opencode").await;

        assert!(result.is_ok());
        let agent = result.unwrap();
        assert_eq!(agent.uid.entity_type, "Aeterna::Agent");
        assert_eq!(agent.uid.id, "agent-opencode");
        assert_eq!(agent.get_attr_str("name"), Some("OpenCode Assistant"));

        // Agent should have user and project as parents
        assert_eq!(agent.parents.len(), 2);
        let parent_types: Vec<&str> = agent
            .parents
            .iter()
            .map(|p| p.entity_type.as_str())
            .collect();
        assert!(parent_types.contains(&"Aeterna::User"));
        assert!(parent_types.contains(&"Aeterna::Project"));
    }

    #[tokio::test]
    async fn test_circuit_breaker_opens_on_failures() {
        let mock_server = MockServer::start().await;

        // Return 503 to simulate service unavailable
        Mock::given(method("GET"))
            .and(path("/v1/data"))
            .respond_with(ResponseTemplate::new(503).set_body_string("Service Unavailable"))
            .mount(&mock_server)
            .await;

        let config = CedarConfig {
            base_url: mock_server.uri(),
            timeout_secs: 1,
            cache_enabled: false,
            cache_ttl_secs: 60,
            max_retries: 1,
            circuit_breaker_threshold: 2, // Open after 2 failures
            circuit_breaker_reset_secs: 30,
        };
        let client = CedarClient::new(config);

        // First failure
        let _ = client.get_all_entities().await;

        // Second failure - should open circuit
        let _ = client.get_all_entities().await;

        // Third attempt should fail immediately with circuit open
        let result = client.get_all_entities().await;
        match result {
            Err(CedarError::Unavailable(msg)) => {
                assert!(msg.contains("Circuit breaker is open"));
            }
            other => panic!("Expected Unavailable error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_error_handling_agent_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/v1/data"))
            .respond_with(ResponseTemplate::new(400).set_body_string("Bad Request"))
            .mount(&mock_server)
            .await;

        let client = create_client_for_mock(&mock_server);
        let result = client.get_all_entities().await;

        assert!(result.is_err());
        match result.unwrap_err() {
            CedarError::AgentError { status, message } => {
                assert_eq!(status, 400);
                assert_eq!(message, "Bad Request");
            }
            other => panic!("Expected AgentError, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_cache_reuse() {
        let mock_server = MockServer::start().await;
        let entities = create_test_entities();

        Mock::given(method("GET"))
            .and(path("/v1/data"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&entities))
            .expect(1)
            .mount(&mock_server)
            .await;

        let config = CedarConfig {
            base_url: mock_server.uri(),
            timeout_secs: 5,
            cache_enabled: true,
            cache_ttl_secs: 300,
            max_retries: 1,
            circuit_breaker_threshold: 5,
            circuit_breaker_reset_secs: 30,
        };
        let client = CedarClient::new(config);

        let uid = EntityUid::user("user-alice");

        let result1 = client.resolve_entity(&uid).await;
        assert!(result1.is_ok());

        let result2 = client.resolve_entity(&uid).await;
        assert!(result2.is_ok());

        assert_eq!(result1.unwrap().uid.id, result2.unwrap().uid.id);
    }

    #[tokio::test]
    async fn test_cache_clear() {
        let mock_server = MockServer::start().await;
        let entities = create_test_entities();

        Mock::given(method("GET"))
            .and(path("/v1/data"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&entities))
            .expect(2) // Should be called twice after cache clear
            .mount(&mock_server)
            .await;

        let config = CedarConfig {
            base_url: mock_server.uri(),
            timeout_secs: 5,
            cache_enabled: true,
            cache_ttl_secs: 300,
            max_retries: 1,
            circuit_breaker_threshold: 5,
            circuit_breaker_reset_secs: 30,
        };
        let client = CedarClient::new(config);

        // First call
        let _ = client.get_all_entities().await;

        // Clear cache
        client.clear_cache().await;

        // Second call - should hit server again
        let _ = client.get_all_entities().await;
    }
}
