//! Context auto-resolution for Aeterna.
//!
//! Resolves tenant context from multiple sources with precedence:
//! 1. Explicit overrides (CLI flags, API params)
//! 2. Environment variables (AETERNA_*)
//! 3. Context file (.aeterna/context.toml)
//! 4. Git remote URL -> project_id
//! 5. Git config user.email -> user_id
//! 6. Organization defaults
//! 7. System defaults

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use git2::Repository;
use mk_core::hints::OperationHints;
use thiserror::Error;
use tracing::{debug, trace, warn};

use crate::types::{ContextConfig, ContextSource, ResolvedContext, ResolvedValue};

/// Context resolution error types.
#[derive(Debug, Error)]
pub enum ContextError {
    #[error("Failed to read context file: {0}")]
    FileRead(#[from] std::io::Error),

    #[error("Failed to parse context file: {0}")]
    ParseError(#[from] toml::de::Error),

    #[error("Git error: {0}")]
    GitError(#[from] git2::Error),

    #[error("Invalid context configuration: {0}")]
    InvalidConfig(String),
}

/// Environment variable prefix for Aeterna configuration.
const ENV_PREFIX: &str = "AETERNA_";

/// Default context file name.
const CONTEXT_FILE: &str = "context.toml";

/// Default context directory name.
const CONTEXT_DIR: &str = ".aeterna";

/// Resolves tenant context from multiple sources with precedence.
///
/// # Precedence (highest to lowest)
///
/// 1. Explicit overrides (via `with_override()`)
/// 2. Environment variables (`AETERNA_TENANT_ID`, etc.)
/// 3. Context file (`.aeterna/context.toml`)
/// 4. Git remote URL -> project_id
/// 5. Git config user.email -> user_id
/// 6. Organization defaults (future: from server)
/// 7. System defaults ("default"/"default")
///
/// # Example
///
/// ```rust,ignore
/// use context::ContextResolver;
///
/// let resolver = ContextResolver::new()
///     .with_override("tenant_id", "acme-corp")
///     .with_override("hints", "no-llm,fast");
///
/// let ctx = resolver.resolve()?;
/// println!("Tenant: {} (from {})", ctx.tenant_id.value, ctx.tenant_id.source);
/// ```
#[derive(Debug, Clone)]
pub struct ContextResolver {
    /// Starting directory for context file search.
    start_dir: PathBuf,

    /// Explicit overrides (highest precedence).
    explicit_overrides: HashMap<String, String>,

    /// Whether to skip git detection.
    skip_git: bool,

    /// Whether to skip environment variables.
    skip_env: bool,

    /// Maximum depth to search for context.toml.
    max_search_depth: usize,
}

impl Default for ContextResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextResolver {
    /// Create a new resolver starting from current directory.
    #[must_use]
    pub fn new() -> Self {
        Self {
            start_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            explicit_overrides: HashMap::new(),
            skip_git: false,
            skip_env: false,
            max_search_depth: 10,
        }
    }

    /// Create a resolver starting from a specific directory.
    #[must_use]
    pub fn from_dir(dir: impl Into<PathBuf>) -> Self {
        Self {
            start_dir: dir.into(),
            ..Self::new()
        }
    }

    /// Add an explicit override (highest precedence).
    #[must_use]
    pub fn with_override(mut self, key: &str, value: &str) -> Self {
        self.explicit_overrides
            .insert(key.to_string(), value.to_string());
        self
    }

    /// Add multiple explicit overrides.
    #[must_use]
    pub fn with_overrides(mut self, overrides: HashMap<String, String>) -> Self {
        self.explicit_overrides.extend(overrides);
        self
    }

    /// Skip git detection (for testing or non-git environments).
    #[must_use]
    pub fn skip_git(mut self) -> Self {
        self.skip_git = true;
        self
    }

    /// Skip environment variable detection.
    #[must_use]
    pub fn skip_env(mut self) -> Self {
        self.skip_env = true;
        self
    }

    /// Set maximum depth for context.toml search.
    #[must_use]
    pub fn with_max_search_depth(mut self, depth: usize) -> Self {
        self.max_search_depth = depth;
        self
    }

    /// Resolve context from all sources.
    ///
    /// # Errors
    ///
    /// Returns error if context file exists but cannot be parsed.
    pub fn resolve(&self) -> Result<ResolvedContext, ContextError> {
        let mut ctx = ResolvedContext::default();

        // Find context file and git root
        let context_toml_path = self.find_context_toml();
        let git_root = if self.skip_git {
            None
        } else {
            self.find_git_root()
        };

        ctx.context_root = context_toml_path
            .as_ref()
            .and_then(|p| p.parent().map(PathBuf::from));
        ctx.git_root = git_root.clone();

        // Layer 7: System defaults (already in ResolvedContext::default())
        debug!("Starting context resolution from {:?}", self.start_dir);

        // Layer 6: Organization defaults (TODO: fetch from server)
        // For now, skip this layer

        // Layer 5: Git config user.email -> user_id
        if let Some(ref git_path) = git_root
            && let Some(email) = self.resolve_git_user_email(git_path)
        {
            trace!("Found git user.email: {}", email);
            ctx.user_id = ResolvedValue::new(email, ContextSource::GitConfig);
        }

        // Layer 4: Git remote URL -> project_id
        if let Some(ref git_path) = git_root
            && let Some(project_id) = self.resolve_git_project_id(git_path)
        {
            trace!("Found git project_id: {}", project_id);
            ctx.project_id = Some(ResolvedValue::new(project_id, ContextSource::GitRemote));
        }

        // Layer 3: Context file (.aeterna/context.toml)
        if let Some(ref toml_path) = context_toml_path {
            match self.load_context_toml(toml_path) {
                Ok(config) => {
                    self.apply_context_config(&mut ctx, &config, toml_path);
                }
                Err(e) => {
                    warn!("Failed to load context.toml: {}", e);
                    // Don't fail - continue with other sources
                }
            }
        }

        // Layer 2: Environment variables
        if !self.skip_env {
            self.apply_env_vars(&mut ctx);
        }

        // Layer 1: Explicit overrides (highest precedence)
        self.apply_explicit_overrides(&mut ctx);

        debug!(
            "Resolved context: tenant={} user={} project={:?}",
            ctx.tenant_id.value,
            ctx.user_id.value,
            ctx.project_id.as_ref().map(|p| &p.value)
        );

        Ok(ctx)
    }

    /// Find `.aeterna/context.toml` by walking up the directory tree.
    fn find_context_toml(&self) -> Option<PathBuf> {
        let mut current = self.start_dir.clone();
        let mut depth = 0;

        loop {
            let context_path = current.join(CONTEXT_DIR).join(CONTEXT_FILE);
            if context_path.exists() {
                debug!("Found context.toml at {:?}", context_path);
                return Some(context_path);
            }

            // Also check for context.toml directly (without .aeterna directory)
            let direct_path = current.join(CONTEXT_FILE);
            if direct_path.exists() {
                debug!("Found context.toml at {:?}", direct_path);
                return Some(direct_path);
            }

            depth += 1;
            if depth >= self.max_search_depth {
                break;
            }

            match current.parent() {
                Some(parent) if parent != current => {
                    current = parent.to_path_buf();
                }
                _ => break,
            }
        }

        trace!(
            "No context.toml found after searching {} levels",
            self.max_search_depth
        );
        None
    }

    /// Find git repository root.
    fn find_git_root(&self) -> Option<PathBuf> {
        Repository::discover(&self.start_dir)
            .ok()
            .and_then(|repo| repo.workdir().map(PathBuf::from))
    }

    /// Extract user email from git config.
    fn resolve_git_user_email(&self, git_root: &Path) -> Option<String> {
        let repo = Repository::open(git_root).ok()?;
        let config = repo.config().ok()?;

        // Try repository-level config first, then global
        config
            .get_string("user.email")
            .ok()
            .filter(|e| !e.is_empty())
    }

    /// Extract project_id from git remote URL.
    ///
    /// Parses URLs like:
    /// - `git@github.com:org/repo.git` -> `org/repo`
    /// - `https://github.com/org/repo.git` -> `org/repo`
    /// - `https://github.com/org/repo` -> `org/repo`
    fn resolve_git_project_id(&self, git_root: &Path) -> Option<String> {
        let repo = Repository::open(git_root).ok()?;
        let remote = repo.find_remote("origin").ok()?;
        let url = remote.url()?;

        parse_git_remote_url(url)
    }

    /// Load and parse context.toml file.
    fn load_context_toml(&self, path: &Path) -> Result<ContextConfig, ContextError> {
        let content = std::fs::read_to_string(path)?;
        let config: ContextConfig = toml::from_str(&content)?;
        Ok(config)
    }

    /// Apply values from context.toml to resolved context.
    fn apply_context_config(
        &self,
        ctx: &mut ResolvedContext,
        config: &ContextConfig,
        toml_path: &Path,
    ) {
        let source = ContextSource::ContextToml(toml_path.to_path_buf());

        if let Some(ref tenant_id) = config.tenant_id {
            ctx.tenant_id = ResolvedValue::new(tenant_id.clone(), source.clone());
        }

        if let Some(ref user_id) = config.user_id {
            ctx.user_id = ResolvedValue::new(user_id.clone(), source.clone());
        }

        if let Some(ref org_id) = config.org_id {
            ctx.org_id = Some(ResolvedValue::new(org_id.clone(), source.clone()));
        }

        if let Some(ref team_id) = config.team_id {
            ctx.team_id = Some(ResolvedValue::new(team_id.clone(), source.clone()));
        }

        if let Some(ref project_id) = config.project_id {
            ctx.project_id = Some(ResolvedValue::new(project_id.clone(), source.clone()));
        }

        if let Some(ref agent_id) = config.agent_id {
            ctx.agent_id = Some(ResolvedValue::new(agent_id.clone(), source.clone()));
        }

        // Apply hints
        let hints = config.to_hints();
        if hints != OperationHints::default() {
            ctx.hints = ResolvedValue::new(hints, source);
        }
    }

    /// Apply environment variables to resolved context.
    fn apply_env_vars(&self, ctx: &mut ResolvedContext) {
        // AETERNA_TENANT_ID
        if let Ok(value) = env::var(format!("{ENV_PREFIX}TENANT_ID")) {
            ctx.tenant_id = ResolvedValue::new(
                value,
                ContextSource::EnvVar(format!("{ENV_PREFIX}TENANT_ID")),
            );
        }

        // AETERNA_USER_ID
        if let Ok(value) = env::var(format!("{ENV_PREFIX}USER_ID")) {
            ctx.user_id =
                ResolvedValue::new(value, ContextSource::EnvVar(format!("{ENV_PREFIX}USER_ID")));
        }

        // AETERNA_ORG_ID
        if let Ok(value) = env::var(format!("{ENV_PREFIX}ORG_ID")) {
            ctx.org_id = Some(ResolvedValue::new(
                value,
                ContextSource::EnvVar(format!("{ENV_PREFIX}ORG_ID")),
            ));
        }

        // AETERNA_TEAM_ID
        if let Ok(value) = env::var(format!("{ENV_PREFIX}TEAM_ID")) {
            ctx.team_id = Some(ResolvedValue::new(
                value,
                ContextSource::EnvVar(format!("{ENV_PREFIX}TEAM_ID")),
            ));
        }

        // AETERNA_PROJECT_ID
        if let Ok(value) = env::var(format!("{ENV_PREFIX}PROJECT_ID")) {
            ctx.project_id = Some(ResolvedValue::new(
                value,
                ContextSource::EnvVar(format!("{ENV_PREFIX}PROJECT_ID")),
            ));
        }

        // AETERNA_AGENT_ID
        if let Ok(value) = env::var(format!("{ENV_PREFIX}AGENT_ID")) {
            ctx.agent_id = Some(ResolvedValue::new(
                value,
                ContextSource::EnvVar(format!("{ENV_PREFIX}AGENT_ID")),
            ));
        }

        // AETERNA_HINTS (merged with existing hints from env)
        let env_hints = OperationHints::from_env();
        if env_hints != OperationHints::default() {
            // Merge with existing hints
            let merged = ctx.hints.value.clone().merge(&env_hints);
            ctx.hints = ResolvedValue::new(
                merged,
                ContextSource::EnvVar(format!("{ENV_PREFIX}HINTS_*")),
            );
        }
    }

    /// Apply explicit overrides to resolved context.
    fn apply_explicit_overrides(&self, ctx: &mut ResolvedContext) {
        if let Some(value) = self.explicit_overrides.get("tenant_id") {
            ctx.tenant_id = ResolvedValue::explicit(value.clone());
        }

        if let Some(value) = self.explicit_overrides.get("user_id") {
            ctx.user_id = ResolvedValue::explicit(value.clone());
        }

        if let Some(value) = self.explicit_overrides.get("org_id") {
            ctx.org_id = Some(ResolvedValue::explicit(value.clone()));
        }

        if let Some(value) = self.explicit_overrides.get("team_id") {
            ctx.team_id = Some(ResolvedValue::explicit(value.clone()));
        }

        if let Some(value) = self.explicit_overrides.get("project_id") {
            ctx.project_id = Some(ResolvedValue::explicit(value.clone()));
        }

        if let Some(value) = self.explicit_overrides.get("agent_id") {
            ctx.agent_id = Some(ResolvedValue::explicit(value.clone()));
        }

        if let Some(value) = self.explicit_overrides.get("session_id") {
            ctx.session_id = Some(ResolvedValue::explicit(value.clone()));
        }

        // Parse hints from string (e.g., "no-llm,fast,no-reasoning")
        if let Some(value) = self.explicit_overrides.get("hints") {
            let hints = OperationHints::parse_hint_string(value);
            ctx.hints = ResolvedValue::explicit(hints);
        }

        // Handle preset override
        if let Some(value) = self.explicit_overrides.get("preset")
            && let Ok(preset) = value.parse()
        {
            let hints = OperationHints::from_preset(preset);
            ctx.hints = ResolvedValue::explicit(hints);
        }
    }
}

/// Resolves and enriches context using Cedar Agent.
///
/// This resolver wraps the standard `ContextResolver` and adds Cedar Agent
/// integration for:
/// - Resolving user by email (from git config)
/// - Resolving project by git remote URL
/// - Discovering accessible layers (tenant/org/team/project)
///
/// # Example
///
/// ```rust,ignore
/// use context::{ContextResolver, CedarContextResolver};
///
/// // First resolve local context
/// let local_ctx = ContextResolver::new().resolve()?;
///
/// // Then enrich with Cedar Agent data
/// let cedar = CedarContextResolver::new();
/// let enriched = cedar.enrich(local_ctx).await?;
/// ```
pub struct CedarContextResolver {
    client: crate::cedar::CedarClient,
}

impl CedarContextResolver {
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: crate::cedar::CedarClient::from_env(),
        }
    }

    #[must_use]
    pub fn with_config(config: crate::cedar::CedarConfig) -> Self {
        Self {
            client: crate::cedar::CedarClient::new(config),
        }
    }

    pub async fn health_check(&self) -> bool {
        self.client.health_check().await.unwrap_or(false)
    }

    /// Enrich resolved context with Cedar Agent data.
    ///
    /// Attempts to resolve user and project from Cedar Agent based on:
    /// - user_id (if it looks like an email, resolve to Cedar User entity)
    /// - project_id (if resolved from git remote, resolve to Cedar Project
    ///   entity)
    ///
    /// If Cedar Agent is unavailable, returns the original context unchanged.
    pub async fn enrich(&self, mut ctx: ResolvedContext) -> Result<ResolvedContext, ContextError> {
        if !self.client.health_check().await.unwrap_or(false) {
            warn!("Cedar Agent unavailable, using local context only");
            return Ok(ctx);
        }

        if let Some(ref project) = ctx.project_id
            && project.source == ContextSource::GitRemote
            && let Ok(cedar_project) = self
                .client
                .resolve_project_by_git_remote(&project.value)
                .await
        {
            debug!(
                "Resolved project from Cedar Agent: {}",
                cedar_project.uid.id
            );
            ctx.project_id = Some(ResolvedValue::new(
                cedar_project.uid.id.clone(),
                ContextSource::CedarAgent,
            ));

            if let Some(team_parent) = cedar_project
                .parents
                .iter()
                .find(|p| p.entity_type == "Aeterna::Team")
            {
                ctx.team_id = Some(ResolvedValue::new(
                    team_parent.id.clone(),
                    ContextSource::CedarAgent,
                ));
            }
        }

        if ctx.user_id.source == ContextSource::GitConfig
            && ctx.user_id.value.contains('@')
            && let Ok(cedar_user) = self.client.resolve_user_by_email(&ctx.user_id.value).await
        {
            debug!("Resolved user from Cedar Agent: {}", cedar_user.uid.id);
            ctx.user_id = ResolvedValue::new(cedar_user.uid.id.clone(), ContextSource::CedarAgent);

            if ctx.tenant_id.source == ContextSource::SystemDefault
                && let Some(tenant_slug) = cedar_user.get_attr_str("tenant_slug")
            {
                ctx.tenant_id =
                    ResolvedValue::new(tenant_slug.to_string(), ContextSource::CedarAgent);
            }
        }

        Ok(ctx)
    }

    /// Get accessible layers for the current user.
    pub async fn get_accessible_layers(
        &self,
        user_id: &str,
    ) -> Result<crate::cedar::AccessibleLayers, crate::cedar::CedarError> {
        self.client.get_accessible_layers(user_id).await
    }

    /// Check if a user is authorized to perform an action.
    pub async fn check_authorization(
        &self,
        principal: &crate::cedar::EntityUid,
        action: &str,
        resource: &crate::cedar::EntityUid,
    ) -> Result<bool, crate::cedar::CedarError> {
        self.client
            .check_authorization(principal, action, resource, None)
            .await
    }

    pub fn client(&self) -> &crate::cedar::CedarClient {
        &self.client
    }
}

impl Default for CedarContextResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse git remote URL to extract org/repo.
///
/// Supports:
/// - SSH: `git@github.com:org/repo.git`
/// - HTTPS: `https://github.com/org/repo.git`
/// - HTTPS without .git: `https://github.com/org/repo`
fn parse_git_remote_url(url: &str) -> Option<String> {
    // SSH format: git@github.com:org/repo.git
    if url.starts_with("git@") {
        let parts: Vec<&str> = url.split(':').collect();
        if parts.len() == 2 {
            let path = parts[1].trim_end_matches(".git");
            return Some(path.to_string());
        }
    }

    // HTTPS format: https://github.com/org/repo.git
    if url.starts_with("https://") || url.starts_with("http://") {
        let path = url
            .trim_start_matches("https://")
            .trim_start_matches("http://");

        // Skip the hostname
        let parts: Vec<&str> = path.splitn(2, '/').collect();
        if parts.len() == 2 {
            let repo_path = parts[1].trim_end_matches(".git");
            return Some(repo_path.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_parse_git_remote_ssh() {
        assert_eq!(
            parse_git_remote_url("git@github.com:acme-corp/payments-service.git"),
            Some("acme-corp/payments-service".to_string())
        );
    }

    #[test]
    fn test_parse_git_remote_https_with_git() {
        assert_eq!(
            parse_git_remote_url("https://github.com/acme-corp/payments-service.git"),
            Some("acme-corp/payments-service".to_string())
        );
    }

    #[test]
    fn test_parse_git_remote_https_without_git() {
        assert_eq!(
            parse_git_remote_url("https://github.com/acme-corp/payments-service"),
            Some("acme-corp/payments-service".to_string())
        );
    }

    #[test]
    fn test_parse_git_remote_gitlab() {
        assert_eq!(
            parse_git_remote_url("git@gitlab.com:my-org/my-project.git"),
            Some("my-org/my-project".to_string())
        );
    }

    #[test]
    fn test_resolver_default() {
        let resolver = ContextResolver::new();
        let ctx = resolver.skip_git().skip_env().resolve().unwrap();

        assert_eq!(ctx.tenant_id.value, "default");
        assert_eq!(ctx.user_id.value, "default");
        assert!(ctx.org_id.is_none());
    }

    #[test]
    fn test_resolver_explicit_overrides() {
        let resolver = ContextResolver::new()
            .skip_git()
            .skip_env()
            .with_override("tenant_id", "acme-corp")
            .with_override("user_id", "alice")
            .with_override("org_id", "platform");

        let ctx = resolver.resolve().unwrap();

        assert_eq!(ctx.tenant_id.value, "acme-corp");
        assert_eq!(ctx.tenant_id.source, ContextSource::Explicit);
        assert_eq!(ctx.user_id.value, "alice");
        assert_eq!(ctx.org_id.unwrap().value, "platform");
    }

    #[test]
    fn test_resolver_hints_override() {
        let resolver = ContextResolver::new()
            .skip_git()
            .skip_env()
            .with_override("hints", "fast,no-llm,no-reasoning");

        let ctx = resolver.resolve().unwrap();

        assert!(!ctx.hints.value.llm);
        assert!(!ctx.hints.value.reasoning);
    }

    #[test]
    fn test_resolver_preset_override() {
        let resolver = ContextResolver::new()
            .skip_git()
            .skip_env()
            .with_override("preset", "minimal");

        let ctx = resolver.resolve().unwrap();

        assert!(!ctx.hints.value.llm);
        assert!(!ctx.hints.value.reasoning);
        assert!(!ctx.hints.value.multi_hop);
    }

    #[test]
    fn test_resolver_with_context_toml() {
        let temp_dir = TempDir::new().unwrap();
        let aeterna_dir = temp_dir.path().join(".aeterna");
        fs::create_dir(&aeterna_dir).unwrap();

        let toml_content = r#"
tenant-id = "acme-corp"
user-id = "alice"
org-id = "platform"
project-id = "payments"

[hints]
preset = "fast"
verbose = true
"#;

        fs::write(aeterna_dir.join("context.toml"), toml_content).unwrap();

        let resolver = ContextResolver::from_dir(temp_dir.path())
            .skip_git()
            .skip_env();

        let ctx = resolver.resolve().unwrap();

        assert_eq!(ctx.tenant_id.value, "acme-corp");
        assert_eq!(ctx.user_id.value, "alice");
        assert_eq!(ctx.org_id.unwrap().value, "platform");
        assert_eq!(ctx.project_id.unwrap().value, "payments");
        assert!(ctx.hints.value.verbose);
        assert!(!ctx.hints.value.reasoning); // Fast preset disables reasoning
    }

    #[test]
    fn test_resolver_context_toml_without_aeterna_dir() {
        let temp_dir = TempDir::new().unwrap();

        let toml_content = r#"
tenant-id = "direct-tenant"
user-id = "direct-user"
"#;

        fs::write(temp_dir.path().join("context.toml"), toml_content).unwrap();

        let resolver = ContextResolver::from_dir(temp_dir.path())
            .skip_git()
            .skip_env();

        let ctx = resolver.resolve().unwrap();

        assert_eq!(ctx.tenant_id.value, "direct-tenant");
        assert_eq!(ctx.user_id.value, "direct-user");
    }

    #[test]
    fn test_resolver_walks_up_directory() {
        let temp_dir = TempDir::new().unwrap();
        let deep_dir = temp_dir.path().join("a/b/c/d");
        fs::create_dir_all(&deep_dir).unwrap();

        // Create context.toml at root
        let aeterna_dir = temp_dir.path().join(".aeterna");
        fs::create_dir(&aeterna_dir).unwrap();
        fs::write(
            aeterna_dir.join("context.toml"),
            "tenant-id = \"root-tenant\"",
        )
        .unwrap();

        // Resolve from deep directory
        let resolver = ContextResolver::from_dir(&deep_dir).skip_git().skip_env();

        let ctx = resolver.resolve().unwrap();

        assert_eq!(ctx.tenant_id.value, "root-tenant");
    }

    #[test]
    fn test_resolver_precedence() {
        let temp_dir = TempDir::new().unwrap();
        let aeterna_dir = temp_dir.path().join(".aeterna");
        fs::create_dir(&aeterna_dir).unwrap();

        // Context.toml sets tenant_id = "from-file"
        fs::write(
            aeterna_dir.join("context.toml"),
            "tenant-id = \"from-file\"",
        )
        .unwrap();

        // Explicit override should win
        let resolver = ContextResolver::from_dir(temp_dir.path())
            .skip_git()
            .skip_env()
            .with_override("tenant_id", "from-override");

        let ctx = resolver.resolve().unwrap();

        assert_eq!(ctx.tenant_id.value, "from-override");
        assert_eq!(ctx.tenant_id.source, ContextSource::Explicit);
    }

    #[test]
    fn test_resolver_explain() {
        let resolver = ContextResolver::new()
            .skip_git()
            .skip_env()
            .with_override("tenant_id", "test-tenant")
            .with_override("org_id", "test-org");

        let ctx = resolver.resolve().unwrap();
        let explanations = ctx.explain();

        assert!(
            explanations
                .iter()
                .any(|(name, value, _)| name == "tenant_id" && value == "test-tenant")
        );
        assert!(
            explanations
                .iter()
                .any(|(name, value, _)| name == "org_id" && value == "test-org")
        );
    }

    #[test]
    fn test_resolver_to_tenant_context() {
        let resolver = ContextResolver::new()
            .skip_git()
            .skip_env()
            .with_override("tenant_id", "acme")
            .with_override("user_id", "bob");

        let ctx = resolver.resolve().unwrap();
        let tenant_ctx = ctx.to_tenant_context();

        assert_eq!(tenant_ctx.tenant_id.as_str(), "acme");
        assert_eq!(tenant_ctx.user_id.as_str(), "bob");
    }

    #[test]
    fn test_resolver_with_agent_id() {
        let resolver = ContextResolver::new()
            .skip_git()
            .skip_env()
            .with_override("tenant_id", "acme")
            .with_override("user_id", "bob")
            .with_override("agent_id", "code-assistant-1");

        let ctx = resolver.resolve().unwrap();
        let tenant_ctx = ctx.to_tenant_context();

        assert!(tenant_ctx.agent_id.is_some());
        assert_eq!(tenant_ctx.agent_id.unwrap(), "code-assistant-1");
    }

    #[test]
    fn test_resolver_max_search_depth() {
        let temp_dir = TempDir::new().unwrap();
        let deep_dir = temp_dir.path().join("a/b/c/d/e");
        fs::create_dir_all(&deep_dir).unwrap();

        let aeterna_dir = temp_dir.path().join(".aeterna");
        fs::create_dir(&aeterna_dir).unwrap();
        fs::write(
            aeterna_dir.join("context.toml"),
            "tenant-id = \"root-tenant\"",
        )
        .unwrap();

        let resolver = ContextResolver::from_dir(&deep_dir)
            .skip_git()
            .skip_env()
            .with_max_search_depth(10);

        let ctx = resolver.resolve().unwrap();
        assert_eq!(ctx.tenant_id.value, "root-tenant");

        let resolver = ContextResolver::from_dir(&deep_dir)
            .skip_git()
            .skip_env()
            .with_max_search_depth(3);

        let ctx = resolver.resolve().unwrap();
        assert_eq!(ctx.tenant_id.value, "default");
    }

    #[test]
    fn test_context_root_tracking() {
        let temp_dir = TempDir::new().unwrap();
        let aeterna_dir = temp_dir.path().join(".aeterna");
        fs::create_dir(&aeterna_dir).unwrap();
        fs::write(aeterna_dir.join("context.toml"), "tenant-id = \"test\"").unwrap();

        let resolver = ContextResolver::from_dir(temp_dir.path())
            .skip_git()
            .skip_env();

        let ctx = resolver.resolve().unwrap();

        assert!(ctx.context_root.is_some());
        assert_eq!(ctx.context_root.unwrap(), aeterna_dir);
    }
}
