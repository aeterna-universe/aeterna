//! # Policy Translator
//!
//! LLM-powered translation from natural language to Constraint DSL policies.
//! Implements the UX-First Architecture's Translation Layer.
//!
//! Pipeline: Natural Language → Structured Intent → PolicyRule (Constraint DSL)
//! → Validation
//!
//! ## Architecture Note
//!
//! This translator generates **Constraint DSL** rules for code-level
//! enforcement:
//! - `must_use`, `must_not_use` - Dependency/import requirements
//! - `must_match`, `must_not_match` - Pattern matching in code/config
//! - `must_exist`, `must_not_exist` - File/config presence checks
//!
//! Cedar policies are used separately for **Aeterna authorization** (who can do
//! what), not for code-level enforcement.

use async_trait::async_trait;
use knowledge::context_architect::{LlmClient, LlmError};
use mk_core::types::{
    ConstraintOperator, ConstraintSeverity, ConstraintTarget, PolicyRule, RuleType,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Policy action type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyAction {
    Allow,
    Deny,
}

impl std::fmt::Display for PolicyAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolicyAction::Allow => write!(f, "allow"),
            PolicyAction::Deny => write!(f, "deny"),
        }
    }
}

impl std::str::FromStr for PolicyAction {
    type Err = PolicyTranslatorError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "allow" | "permit" | "enable" | "require" => Ok(PolicyAction::Allow),
            "deny" | "forbid" | "block" | "prevent" | "disallow" => Ok(PolicyAction::Deny),
            _ => Err(PolicyTranslatorError::InvalidAction(s.to_string())),
        }
    }
}

/// Target type for policy rules
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetType {
    Dependency,
    File,
    Code,
    Import,
    Config,
}

impl std::fmt::Display for TargetType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TargetType::Dependency => write!(f, "dependency"),
            TargetType::File => write!(f, "file"),
            TargetType::Code => write!(f, "code"),
            TargetType::Import => write!(f, "import"),
            TargetType::Config => write!(f, "config"),
        }
    }
}

impl std::str::FromStr for TargetType {
    type Err = PolicyTranslatorError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "dependency" | "dep" | "package" | "library" => Ok(TargetType::Dependency),
            "file" | "path" => Ok(TargetType::File),
            "code" | "pattern" | "regex" => Ok(TargetType::Code),
            "import" | "module" => Ok(TargetType::Import),
            "config" | "configuration" | "setting" | "api" | "endpoint" | "route" => {
                Ok(TargetType::Config)
            }
            _ => Err(PolicyTranslatorError::InvalidTargetType(s.to_string())),
        }
    }
}

impl From<TargetType> for ConstraintTarget {
    fn from(tt: TargetType) -> Self {
        match tt {
            TargetType::Dependency => ConstraintTarget::Dependency,
            TargetType::File => ConstraintTarget::File,
            TargetType::Code => ConstraintTarget::Code,
            TargetType::Import => ConstraintTarget::Import,
            TargetType::Config => ConstraintTarget::Config,
        }
    }
}

impl From<ConstraintTarget> for TargetType {
    fn from(ct: ConstraintTarget) -> Self {
        match ct {
            ConstraintTarget::Dependency => TargetType::Dependency,
            ConstraintTarget::File => TargetType::File,
            ConstraintTarget::Code => TargetType::Code,
            ConstraintTarget::Import => TargetType::Import,
            ConstraintTarget::Config => TargetType::Config,
        }
    }
}

/// Severity level for policy violations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicySeverity {
    Info,
    Warn,
    Block,
}

impl std::fmt::Display for PolicySeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolicySeverity::Info => write!(f, "info"),
            PolicySeverity::Warn => write!(f, "warn"),
            PolicySeverity::Block => write!(f, "block"),
        }
    }
}

impl std::str::FromStr for PolicySeverity {
    type Err = PolicyTranslatorError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "info" | "information" => Ok(PolicySeverity::Info),
            "warn" | "warning" => Ok(PolicySeverity::Warn),
            "block" | "blocking" | "critical" | "error" | "err" => Ok(PolicySeverity::Block),
            _ => Err(PolicyTranslatorError::InvalidSeverity(s.to_string())),
        }
    }
}

impl From<PolicySeverity> for ConstraintSeverity {
    fn from(ps: PolicySeverity) -> Self {
        match ps {
            PolicySeverity::Info => ConstraintSeverity::Info,
            PolicySeverity::Warn => ConstraintSeverity::Warn,
            PolicySeverity::Block => ConstraintSeverity::Block,
        }
    }
}

impl From<ConstraintSeverity> for PolicySeverity {
    fn from(cs: ConstraintSeverity) -> Self {
        match cs {
            ConstraintSeverity::Info => PolicySeverity::Info,
            ConstraintSeverity::Warn => PolicySeverity::Warn,
            ConstraintSeverity::Block => PolicySeverity::Block,
        }
    }
}

/// Policy scope/layer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyScope {
    Tenant,
    Org,
    Team,
    Project,
}

impl std::fmt::Display for PolicyScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolicyScope::Tenant => write!(f, "tenant"),
            PolicyScope::Org => write!(f, "org"),
            PolicyScope::Team => write!(f, "team"),
            PolicyScope::Project => write!(f, "project"),
        }
    }
}

impl std::str::FromStr for PolicyScope {
    type Err = PolicyTranslatorError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "tenant" | "enterprise" | "global" => Ok(PolicyScope::Tenant),
            "org" | "organization" | "department" => Ok(PolicyScope::Org),
            "team" | "group" => Ok(PolicyScope::Team),
            "project" | "repository" | "repo" => Ok(PolicyScope::Project),
            _ => Err(PolicyTranslatorError::InvalidScope(s.to_string())),
        }
    }
}

/// Structured intent extracted from natural language
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredIntent {
    /// Original natural language input
    pub original: String,
    /// Interpreted description
    pub interpreted: String,
    /// Action to take (allow/deny)
    pub action: PolicyAction,
    /// Type of target (dependency, file, code, etc.)
    pub target_type: TargetType,
    /// Specific target value or pattern
    pub target_value: String,
    /// Optional condition
    pub condition: Option<String>,
    /// Severity level
    pub severity: PolicySeverity,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f32,
}

/// Translation context for policy creation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationContext {
    /// Policy scope
    pub scope: PolicyScope,
    /// Project name (if available)
    pub project: Option<String>,
    /// Team name (if available)
    pub team: Option<String>,
    /// Organization name (if available)
    pub org: Option<String>,
    /// Additional context hints
    pub hints: Vec<String>,
}

impl Default for TranslationContext {
    fn default() -> Self {
        Self {
            scope: PolicyScope::Project,
            project: None,
            team: None,
            org: None,
            hints: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub error_type: String,
    pub message: String,
    pub rule_index: Option<usize>,
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDraft {
    pub draft_id: String,
    pub status: DraftStatus,
    pub intent: StructuredIntent,
    pub rules: Vec<PolicyRule>,
    pub explanation: String,
    pub validation: ValidationResult,
    pub name: String,
}

/// Draft status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftStatus {
    PendingReview,
    Validated,
    ValidationFailed,
    Submitted,
    Approved,
    Rejected,
}

impl std::fmt::Display for DraftStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DraftStatus::PendingReview => write!(f, "pending_review"),
            DraftStatus::Validated => write!(f, "validated"),
            DraftStatus::ValidationFailed => write!(f, "validation_failed"),
            DraftStatus::Submitted => write!(f, "submitted"),
            DraftStatus::Approved => write!(f, "approved"),
            DraftStatus::Rejected => write!(f, "rejected"),
        }
    }
}

/// Policy translator errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum PolicyTranslatorError {
    #[error("LLM error: {0}")]
    LlmError(String),

    #[error("Failed to parse LLM response: {0}")]
    ParseError(String),

    #[error("Invalid action: {0}")]
    InvalidAction(String),

    #[error("Invalid target type: {0}")]
    InvalidTargetType(String),

    #[error("Invalid severity: {0}")]
    InvalidSeverity(String),

    #[error("Invalid scope: {0}")]
    InvalidScope(String),

    #[error("Cedar validation failed: {0}")]
    CedarValidationFailed(String),

    #[error("Template not found: {0}")]
    TemplateNotFound(String),

    #[error("Intent extraction failed: {0}")]
    IntentExtractionFailed(String),
}

impl From<LlmError> for PolicyTranslatorError {
    fn from(err: LlmError) -> Self {
        PolicyTranslatorError::LlmError(err.to_string())
    }
}

/// Configuration for the policy translator
#[derive(Debug, Clone)]
pub struct PolicyTranslatorConfig {
    /// Use templates for simple pattern extraction (faster for common patterns)
    pub use_templates: bool,
    /// Minimum confidence threshold for LLM extraction
    pub min_confidence: f32,
    /// Maximum retries for LLM calls (including validation feedback loop)
    pub max_retries: u32,
    /// Validate Cedar syntax strictly
    pub strict_validation: bool,
    /// Number of few-shot examples to include in prompts
    pub few_shot_count: usize,
    /// Include Cedar schema in generation prompts
    pub include_schema: bool,
    /// Enable caching for repeated translations
    pub enable_cache: bool,
    /// Cache TTL in seconds (default: 1 hour)
    pub cache_ttl_secs: u64,
    /// Maximum cache entries (default: 1000)
    pub cache_max_entries: usize,
}

impl Default for PolicyTranslatorConfig {
    fn default() -> Self {
        Self {
            use_templates: true,
            min_confidence: 0.7,
            max_retries: 3,
            strict_validation: true,
            few_shot_count: 10,
            include_schema: true,
            enable_cache: true,
            cache_ttl_secs: 3600,
            cache_max_entries: 1000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TranslationExample {
    pub natural_language: String,
    pub structured_intent: StructuredIntent,
    pub rules: Vec<PolicyRule>,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    draft: PolicyDraft,
    created_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheKey {
    intent: String,
    scope: String,
    project: String,
}

impl Hash for CacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.intent.hash(state);
        self.scope.hash(state);
        self.project.hash(state);
    }
}

impl CacheKey {
    fn from_context(intent: &str, ctx: &TranslationContext) -> Self {
        Self {
            intent: intent.to_lowercase().trim().to_string(),
            scope: ctx.scope.to_string(),
            project: ctx.project.clone().unwrap_or_default(),
        }
    }
}

#[allow(dead_code)]
pub struct PolicyTranslator<C: LlmClient> {
    client: Arc<C>,
    config: PolicyTranslatorConfig,
    examples: Vec<TranslationExample>,
    cache: RwLock<HashMap<CacheKey, CacheEntry>>,
}

impl<C: LlmClient> PolicyTranslator<C> {
    pub fn new(client: Arc<C>, config: PolicyTranslatorConfig) -> Self {
        Self {
            client,
            examples: Self::default_examples(config.few_shot_count),
            config,
            cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_examples(
        client: Arc<C>,
        config: PolicyTranslatorConfig,
        examples: Vec<TranslationExample>,
    ) -> Self {
        Self {
            client,
            examples,
            config,
            cache: RwLock::new(HashMap::new()),
        }
    }

    pub async fn translate(
        &self,
        intent: &str,
        context: &TranslationContext,
    ) -> Result<PolicyDraft, PolicyTranslatorError> {
        if self.config.enable_cache
            && let Some(cached) = self.get_cached(intent, context)
        {
            return Ok(cached);
        }

        let draft = self.translate_uncached(intent, context).await?;

        if self.config.enable_cache {
            self.cache_result(intent, context, &draft);
        }

        Ok(draft)
    }

    async fn translate_uncached(
        &self,
        intent: &str,
        context: &TranslationContext,
    ) -> Result<PolicyDraft, PolicyTranslatorError> {
        let structured = self.extract_intent(intent, context).await?;

        let rules = self.generate_rules(&structured, context).await?;

        let validation = self.validate_rules(&rules);

        let explanation = self.generate_explanation(&structured, &rules).await?;

        let draft_id = format!(
            "draft-{}-{}",
            self.generate_policy_name(&structured),
            chrono::Utc::now().timestamp()
        );

        let status = if validation.is_valid {
            DraftStatus::Validated
        } else {
            DraftStatus::ValidationFailed
        };

        Ok(PolicyDraft {
            draft_id,
            status,
            name: self.generate_policy_name(&structured),
            intent: structured,
            rules,
            explanation,
            validation,
        })
    }

    fn get_cached(&self, intent: &str, ctx: &TranslationContext) -> Option<PolicyDraft> {
        let key = CacheKey::from_context(intent, ctx);
        let cache = self.cache.read().ok()?;
        let entry = cache.get(&key)?;

        let ttl = Duration::from_secs(self.config.cache_ttl_secs);
        if entry.created_at.elapsed() > ttl {
            return None;
        }

        let mut draft = entry.draft.clone();
        draft.draft_id = format!("draft-{}-{}", draft.name, chrono::Utc::now().timestamp());
        Some(draft)
    }

    fn cache_result(&self, intent: &str, ctx: &TranslationContext, draft: &PolicyDraft) {
        let key = CacheKey::from_context(intent, ctx);

        if let Ok(mut cache) = self.cache.write() {
            if cache.len() >= self.config.cache_max_entries {
                self.evict_expired(&mut cache);
            }

            if cache.len() < self.config.cache_max_entries {
                cache.insert(
                    key,
                    CacheEntry {
                        draft: draft.clone(),
                        created_at: Instant::now(),
                    },
                );
            }
        }
    }

    fn evict_expired(&self, cache: &mut HashMap<CacheKey, CacheEntry>) {
        let ttl = Duration::from_secs(self.config.cache_ttl_secs);
        cache.retain(|_, entry| entry.created_at.elapsed() <= ttl);
    }

    pub fn clear_cache(&self) {
        if let Ok(mut cache) = self.cache.write() {
            cache.clear();
        }
    }

    pub fn cache_stats(&self) -> (usize, usize) {
        let cache = self.cache.read().ok();
        let size = cache.as_ref().map(|c| c.len()).unwrap_or(0);
        (size, self.config.cache_max_entries)
    }

    /// Extract structured intent from natural language using LLM
    async fn extract_intent(
        &self,
        natural: &str,
        ctx: &TranslationContext,
    ) -> Result<StructuredIntent, PolicyTranslatorError> {
        // First, try template-based extraction for common patterns
        if self.config.use_templates
            && let Some(intent) = self.template_extract(natural, ctx)
        {
            return Ok(intent);
        }

        // Fall back to LLM extraction
        let system_prompt = r#"You are a policy intent extractor. Given a natural language description of a policy, extract structured information.

Output ONLY valid JSON with this exact structure:
{
  "action": "allow" or "deny",
  "target_type": "dependency" | "file" | "code" | "import" | "config" | "api",
  "target_value": "specific value or pattern",
  "condition": "optional condition or null",
  "severity": "info" | "warn" | "error" | "block",
  "interpreted": "clear interpretation of the intent",
  "confidence": 0.0 to 1.0
}

Rules:
- "block", "forbid", "prevent", "disallow" → action: "deny"
- "allow", "permit", "require", "must have" → action: "allow"
- For dependencies: target_type is "dependency"
- For file requirements: target_type is "file"
- For code patterns: target_type is "code"
- severity "block" means the action is prevented
- severity "warn" means warning only
- Always provide a clear "interpreted" explanation"#;

        let user_prompt = format!(
            "Extract policy intent from: \"{}\"\n\nContext: scope={}, project={}, team={}",
            natural,
            ctx.scope,
            ctx.project.as_deref().unwrap_or("unknown"),
            ctx.team.as_deref().unwrap_or("unknown")
        );

        let response = self
            .client
            .complete_with_system(system_prompt, &user_prompt)
            .await?;

        self.parse_intent_response(&response, natural)
    }

    /// Template-based extraction for common patterns (faster, deterministic)
    fn template_extract(
        &self,
        natural: &str,
        _ctx: &TranslationContext,
    ) -> Option<StructuredIntent> {
        let lower = natural.to_lowercase();

        // Pattern: "block/forbid/prevent X" for dependencies
        let block_dep_patterns = [
            (r"block\s+(\w+)", PolicyAction::Deny),
            (r"forbid\s+(\w+)", PolicyAction::Deny),
            (r"prevent\s+(\w+)", PolicyAction::Deny),
            (r"disallow\s+(\w+)", PolicyAction::Deny),
            (r"no\s+(\w+)", PolicyAction::Deny),
            (r"ban\s+(\w+)", PolicyAction::Deny),
        ];

        for (pattern, action) in block_dep_patterns {
            if let Some(caps) = regex::Regex::new(pattern).ok()?.captures(&lower) {
                let target = caps.get(1)?.as_str();
                // Check if it looks like a dependency name
                if self.is_likely_dependency(target) {
                    return Some(StructuredIntent {
                        original: natural.to_string(),
                        interpreted: format!("{} usage of {} dependency", action, target),
                        action,
                        target_type: TargetType::Dependency,
                        target_value: target.to_string(),
                        condition: None,
                        severity: PolicySeverity::Block,
                        confidence: 0.95,
                    });
                }
            }
        }

        // Pattern: "require X" for files
        let require_file_patterns = [
            (r"require\s+([\w\.]+)", "file"),
            (r"must\s+have\s+([\w\.]+)", "file"),
            (r"needs?\s+([\w\.]+)", "file"),
        ];

        for (pattern, _) in require_file_patterns {
            if let Some(caps) = regex::Regex::new(pattern).ok()?.captures(&lower) {
                let target = caps.get(1)?.as_str();
                if self.is_likely_file(target) {
                    return Some(StructuredIntent {
                        original: natural.to_string(),
                        interpreted: format!("Require {} file to exist", target),
                        action: PolicyAction::Allow,
                        target_type: TargetType::File,
                        target_value: target.to_string(),
                        condition: Some("must_exist".to_string()),
                        severity: PolicySeverity::Warn,
                        confidence: 0.9,
                    });
                }
            }
        }

        None
    }

    /// Check if a string looks like a dependency name
    fn is_likely_dependency(&self, s: &str) -> bool {
        let common_deps = [
            "mysql",
            "mysql2",
            "mongodb",
            "postgres",
            "postgresql",
            "redis",
            "lodash",
            "jquery",
            "moment",
            "express",
            "react",
            "vue",
            "angular",
            "axios",
            "fetch",
            "request",
            "sqlite",
            "mariadb",
            "oracle",
        ];
        common_deps.contains(&s.to_lowercase().as_str()) ||
            // npm-style package names
            s.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '@' || c == '/')
    }

    /// Check if a string looks like a file name
    fn is_likely_file(&self, s: &str) -> bool {
        s.contains('.')
            || [
                "readme",
                "license",
                "changelog",
                "security",
                "contributing",
                "dockerfile",
                "makefile",
            ]
            .contains(&s.to_lowercase().as_str())
    }

    /// Parse LLM response into structured intent
    fn parse_intent_response(
        &self,
        response: &str,
        original: &str,
    ) -> Result<StructuredIntent, PolicyTranslatorError> {
        // Try to extract JSON from response (handle markdown code blocks)
        let json_str = if response.contains("```json") {
            response
                .split("```json")
                .nth(1)
                .and_then(|s| s.split("```").next())
                .unwrap_or(response)
        } else if response.contains("```") {
            response.split("```").nth(1).unwrap_or(response)
        } else {
            response
        };

        #[derive(Deserialize)]
        struct LlmIntentResponse {
            action: String,
            target_type: String,
            target_value: String,
            condition: Option<String>,
            severity: String,
            interpreted: String,
            confidence: f32,
        }

        let parsed: LlmIntentResponse = serde_json::from_str(json_str.trim())
            .map_err(|e| PolicyTranslatorError::ParseError(format!("JSON parse error: {}", e)))?;

        Ok(StructuredIntent {
            original: original.to_string(),
            interpreted: parsed.interpreted,
            action: parsed.action.parse()?,
            target_type: parsed.target_type.parse()?,
            target_value: parsed.target_value,
            condition: parsed.condition,
            severity: parsed.severity.parse()?,
            confidence: parsed.confidence,
        })
    }

    async fn generate_rules(
        &self,
        intent: &StructuredIntent,
        _ctx: &TranslationContext,
    ) -> Result<Vec<PolicyRule>, PolicyTranslatorError> {
        let operator = self.intent_to_operator(intent);
        let target: ConstraintTarget = intent.target_type.into();
        let severity: ConstraintSeverity = intent.severity.into();

        let rule_type = match intent.action {
            PolicyAction::Allow => RuleType::Allow,
            PolicyAction::Deny => RuleType::Deny,
        };

        let rule = PolicyRule {
            id: self.generate_policy_name(intent),
            rule_type,
            target,
            operator,
            value: serde_json::Value::String(intent.target_value.clone()),
            severity,
            message: intent.interpreted.clone(),
        };

        Ok(vec![rule])
    }

    fn intent_to_operator(&self, intent: &StructuredIntent) -> ConstraintOperator {
        match (&intent.action, &intent.target_type) {
            (PolicyAction::Deny, TargetType::Dependency) => ConstraintOperator::MustNotUse,
            (PolicyAction::Allow, TargetType::Dependency) => ConstraintOperator::MustUse,
            (PolicyAction::Deny, TargetType::Import) => ConstraintOperator::MustNotUse,
            (PolicyAction::Allow, TargetType::Import) => ConstraintOperator::MustUse,
            (PolicyAction::Deny, TargetType::Code) => ConstraintOperator::MustNotMatch,
            (PolicyAction::Allow, TargetType::Code) => ConstraintOperator::MustMatch,
            (PolicyAction::Deny, TargetType::Config) => ConstraintOperator::MustNotMatch,
            (PolicyAction::Allow, TargetType::Config) => ConstraintOperator::MustMatch,
            (PolicyAction::Deny, TargetType::File) => ConstraintOperator::MustNotExist,
            (PolicyAction::Allow, TargetType::File) => ConstraintOperator::MustExist,
        }
    }

    fn validate_rules(&self, rules: &[PolicyRule]) -> ValidationResult {
        let mut errors = Vec::new();
        let warnings = Vec::new();

        for (idx, rule) in rules.iter().enumerate() {
            if rule.id.is_empty() {
                errors.push(ValidationError {
                    error_type: "missing_id".to_string(),
                    message: "Rule must have an ID".to_string(),
                    rule_index: Some(idx),
                    suggestion: Some("Add a unique rule ID".to_string()),
                });
            }

            if rule.message.is_empty() {
                errors.push(ValidationError {
                    error_type: "missing_message".to_string(),
                    message: "Rule should have a descriptive message".to_string(),
                    rule_index: Some(idx),
                    suggestion: Some("Add a human-readable message".to_string()),
                });
            }

            if let serde_json::Value::String(ref s) = rule.value
                && s.is_empty()
            {
                errors.push(ValidationError {
                    error_type: "empty_value".to_string(),
                    message: "Rule value cannot be empty".to_string(),
                    rule_index: Some(idx),
                    suggestion: Some("Provide a pattern or value to match".to_string()),
                });
            }
        }

        ValidationResult {
            is_valid: errors.is_empty(),
            errors,
            warnings,
        }
    }

    async fn generate_explanation(
        &self,
        intent: &StructuredIntent,
        _rules: &[PolicyRule],
    ) -> Result<String, PolicyTranslatorError> {
        let explanation = match (&intent.action, &intent.target_type) {
            (PolicyAction::Deny, TargetType::Dependency) => {
                format!(
                    "This rule blocks the use of '{}' as a dependency. Violations will be {}.",
                    intent.target_value,
                    severity_effect(&intent.severity)
                )
            }
            (PolicyAction::Allow, TargetType::Dependency) => {
                format!(
                    "This rule requires '{}' to be used as a dependency.",
                    intent.target_value
                )
            }
            (PolicyAction::Allow, TargetType::File) => {
                format!(
                    "This rule requires the file '{}' to exist. When missing, a {} will be raised.",
                    intent.target_value, intent.severity
                )
            }
            (PolicyAction::Deny, TargetType::File) => {
                format!(
                    "This rule blocks the file '{}' from existing.",
                    intent.target_value
                )
            }
            (PolicyAction::Deny, TargetType::Code) => {
                format!(
                    "This rule prevents code matching pattern '{}'. Violations will be {}.",
                    intent.target_value,
                    severity_effect(&intent.severity)
                )
            }
            (PolicyAction::Allow, TargetType::Code) => {
                format!(
                    "This rule requires code to match pattern '{}'.",
                    intent.target_value
                )
            }
            (PolicyAction::Deny, TargetType::Import) => {
                format!(
                    "This rule blocks imports of '{}'. Violations will be {}.",
                    intent.target_value,
                    severity_effect(&intent.severity)
                )
            }
            (PolicyAction::Allow, TargetType::Import) => {
                format!("This rule requires the import '{}'.", intent.target_value)
            }
            (PolicyAction::Deny, TargetType::Config) => {
                format!(
                    "This rule blocks config matching '{}'. Violations will be {}.",
                    intent.target_value,
                    severity_effect(&intent.severity)
                )
            }
            (PolicyAction::Allow, TargetType::Config) => {
                format!(
                    "This rule requires config to match '{}'.",
                    intent.target_value
                )
            }
        };

        Ok(explanation)
    }

    /// Generate a policy name from intent
    fn generate_policy_name(&self, intent: &StructuredIntent) -> String {
        let action_prefix = match intent.action {
            PolicyAction::Allow => "require",
            PolicyAction::Deny => "no",
        };

        let target = intent
            .target_value
            .replace('@', "")
            .replace(['/', '.'], "-")
            .to_lowercase();

        let target = if target.len() > 20 {
            target[..20].to_string()
        } else {
            target
        };

        format!("{}-{}", action_prefix, target)
    }

    fn default_examples(count: usize) -> Vec<TranslationExample> {
        crate::translation_examples::few_shot_examples(count)
    }

    pub async fn explain_errors(&self, validation_result: &ValidationResult) -> Vec<String> {
        let mut explanations = Vec::new();

        for error in &validation_result.errors {
            let explanation = self.explain_single_error(error).await;
            explanations.push(explanation);
        }

        explanations
    }

    async fn explain_single_error(&self, error: &ValidationError) -> String {
        match error.error_type.as_str() {
            "syntax" => self.explain_syntax_error(&error.message),
            "schema" => self.explain_schema_error(&error.message).await,
            "semantic" => self.explain_semantic_error(&error.message),
            _ => self.explain_unknown_error(&error.message).await,
        }
    }

    fn explain_syntax_error(&self, message: &str) -> String {
        let lower = message.to_lowercase();

        if lower.contains("unbalanced braces") {
            return "Your policy has mismatched curly braces { }. Check that every opening brace \
                    has a matching closing brace."
                .to_string();
        }

        if lower.contains("permit") || lower.contains("forbid") {
            return "Your policy must start with either 'permit' (to allow) or 'forbid' (to \
                    deny). Example: forbid(principal, action, resource);"
                .to_string();
        }

        if lower.contains("semicolon") {
            return "Cedar policies must end with a semicolon (;). Add one at the end of your \
                    policy."
                .to_string();
        }

        if lower.contains("parenthesis") || lower.contains("paren") {
            return "Check your parentheses - every opening ( needs a closing ).".to_string();
        }

        if lower.contains("string") || lower.contains("quote") {
            return "String values must be wrapped in double quotes. Example: resource.name == \
                    \"value\""
                .to_string();
        }

        format!(
            "Syntax error: {}. Check that your policy follows Cedar syntax rules.",
            message
        )
    }

    async fn explain_schema_error(&self, message: &str) -> String {
        let lower = message.to_lowercase();

        if lower.contains("unknown entity") || lower.contains("entity type") {
            return format!(
                "The entity type you used isn't defined in the schema. Valid types include: User, \
                 Agent, Memory, Knowledge, Policy. Error: {}",
                message
            );
        }

        if lower.contains("unknown action") || lower.contains("action type") {
            return format!(
                "The action you used isn't defined. Valid actions include: UseDependency, Commit, \
                 Deploy, Build, ViewMemory, CreateMemory, etc. Error: {}",
                message
            );
        }

        if lower.contains("attribute") {
            return format!(
                "The attribute you referenced doesn't exist on that entity type. Check the schema \
                 for valid attributes. Error: {}",
                message
            );
        }

        let prompt = format!(
            "Explain this Cedar schema error in simple terms (1-2 sentences):\n\n{}",
            message
        );
        match self.client.complete(&prompt).await {
            Ok(explanation) => explanation,
            Err(_) => format!("Schema validation error: {}", message),
        }
    }

    fn explain_semantic_error(&self, message: &str) -> String {
        let lower = message.to_lowercase();

        if lower.contains("condition") || lower.contains("when") {
            return "The condition in your 'when' block is invalid. Conditions must evaluate to \
                    true/false."
                .to_string();
        }

        if lower.contains("type mismatch") {
            return "You're comparing incompatible types. For example, you can't compare a string \
                    to a number."
                .to_string();
        }

        if lower.contains("undefined") || lower.contains("not defined") {
            return format!(
                "You're referencing something that hasn't been defined. {}",
                message
            );
        }

        format!("Policy logic error: {}", message)
    }

    async fn explain_unknown_error(&self, message: &str) -> String {
        let prompt = format!(
            "A user received this Cedar policy error. Explain what went wrong and how to fix it \
             in 1-2 simple sentences:\n\n{}",
            message
        );

        match self.client.complete(&prompt).await {
            Ok(explanation) => explanation,
            Err(_) => format!(
                "Error: {}. Try reviewing your policy syntax or consult the Cedar documentation.",
                message
            ),
        }
    }

    pub fn explain_validation_warnings(&self, validation_result: &ValidationResult) -> Vec<String> {
        validation_result
            .warnings
            .iter()
            .map(|w| self.explain_warning(w))
            .collect()
    }

    fn explain_warning(&self, warning: &str) -> String {
        let lower = warning.to_lowercase();

        if lower.contains("principal") {
            return "Tip: Your policy doesn't reference 'principal' (who is performing the \
                    action). This means it applies to everyone."
                .to_string();
        }

        if lower.contains("action") {
            return "Tip: Your policy doesn't specify which action it applies to. Consider adding \
                    an action condition like 'action == Action::\"UseDependency\"'."
                .to_string();
        }

        if lower.contains("resource") {
            return "Tip: Your policy doesn't reference 'resource' (what is being accessed). This \
                    might be intentional for broad policies."
                .to_string();
        }

        if lower.contains("semicolon") {
            return "Tip: Cedar policies should end with a semicolon (;) for consistency."
                .to_string();
        }

        format!("Note: {}", warning)
    }
}

fn severity_effect(severity: &PolicySeverity) -> &'static str {
    match severity {
        PolicySeverity::Info => "logged as information",
        PolicySeverity::Warn => "warned",
        PolicySeverity::Block => "blocked",
    }
}

/// Trait for policy translation (allows mocking)
#[async_trait]
pub trait PolicyTranslate: Send + Sync {
    async fn translate(
        &self,
        intent: &str,
        context: &TranslationContext,
    ) -> Result<PolicyDraft, PolicyTranslatorError>;

    fn validate_policy_rules(&self, rules: &[PolicyRule]) -> ValidationResult;
}

#[async_trait]
impl<C: LlmClient> PolicyTranslate for PolicyTranslator<C> {
    async fn translate(
        &self,
        intent: &str,
        context: &TranslationContext,
    ) -> Result<PolicyDraft, PolicyTranslatorError> {
        PolicyTranslator::translate(self, intent, context).await
    }

    fn validate_policy_rules(&self, rules: &[PolicyRule]) -> ValidationResult {
        self.validate_rules(rules)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct MockLlmClient {
        responses: Mutex<VecDeque<String>>,
    }

    impl MockLlmClient {
        fn new(responses: Vec<String>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
            }
        }
    }

    #[async_trait]
    impl LlmClient for MockLlmClient {
        async fn complete(&self, _prompt: &str) -> Result<String, LlmError> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| LlmError::RequestFailed("No mock response".to_string()))
        }

        async fn complete_with_system(
            &self,
            _system: &str,
            _user: &str,
        ) -> Result<String, LlmError> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| LlmError::RequestFailed("No mock response".to_string()))
        }
    }

    #[test]
    fn test_policy_action_parse() {
        assert_eq!("block".parse::<PolicyAction>().unwrap(), PolicyAction::Deny);
        assert_eq!(
            "allow".parse::<PolicyAction>().unwrap(),
            PolicyAction::Allow
        );
        assert_eq!(
            "forbid".parse::<PolicyAction>().unwrap(),
            PolicyAction::Deny
        );
        assert_eq!(
            "permit".parse::<PolicyAction>().unwrap(),
            PolicyAction::Allow
        );
    }

    #[test]
    fn test_target_type_parse() {
        assert_eq!(
            "dependency".parse::<TargetType>().unwrap(),
            TargetType::Dependency
        );
        assert_eq!("file".parse::<TargetType>().unwrap(), TargetType::File);
        assert_eq!("code".parse::<TargetType>().unwrap(), TargetType::Code);
        assert_eq!(
            "package".parse::<TargetType>().unwrap(),
            TargetType::Dependency
        );
    }

    #[test]
    fn test_severity_parse() {
        assert_eq!(
            "block".parse::<PolicySeverity>().unwrap(),
            PolicySeverity::Block
        );
        assert_eq!(
            "warn".parse::<PolicySeverity>().unwrap(),
            PolicySeverity::Warn
        );
        assert_eq!(
            "info".parse::<PolicySeverity>().unwrap(),
            PolicySeverity::Info
        );
    }

    #[test]
    fn test_scope_parse() {
        assert_eq!(
            "project".parse::<PolicyScope>().unwrap(),
            PolicyScope::Project
        );
        assert_eq!("team".parse::<PolicyScope>().unwrap(), PolicyScope::Team);
        assert_eq!("org".parse::<PolicyScope>().unwrap(), PolicyScope::Org);
        assert_eq!(
            "tenant".parse::<PolicyScope>().unwrap(),
            PolicyScope::Tenant
        );
    }

    #[tokio::test]
    async fn test_template_extraction_block_mysql() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());
        let ctx = TranslationContext::default();

        let intent = translator.template_extract("Block MySQL", &ctx);
        assert!(intent.is_some());
        let intent = intent.unwrap();
        assert_eq!(intent.action, PolicyAction::Deny);
        assert_eq!(intent.target_type, TargetType::Dependency);
        assert_eq!(intent.target_value, "mysql");
    }

    #[tokio::test]
    async fn test_template_extraction_require_readme() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());
        let ctx = TranslationContext::default();

        let intent = translator.template_extract("Require README.md", &ctx);
        assert!(intent.is_some());
        let intent = intent.unwrap();
        assert_eq!(intent.action, PolicyAction::Allow);
        assert_eq!(intent.target_type, TargetType::File);
        assert_eq!(intent.target_value, "readme.md");
    }

    #[tokio::test]
    async fn test_translate_with_template() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());
        let ctx = TranslationContext::default();

        let draft = translator.translate("Block MySQL", &ctx).await.unwrap();
        assert_eq!(draft.intent.action, PolicyAction::Deny);
        assert_eq!(draft.intent.target_type, TargetType::Dependency);
        assert!(!draft.rules.is_empty());
        assert!(draft.validation.is_valid);
    }

    #[tokio::test]
    async fn test_translate_with_llm() {
        let llm_response = r#"{
            "action": "deny",
            "target_type": "dependency",
            "target_value": "lodash",
            "condition": null,
            "severity": "warn",
            "interpreted": "Deny usage of lodash library",
            "confidence": 0.85
        }"#;

        let client = Arc::new(MockLlmClient::new(vec![llm_response.to_string()]));

        let mut config = PolicyTranslatorConfig::default();
        config.use_templates = false;

        let translator = PolicyTranslator::new(client, config);
        let ctx = TranslationContext::default();

        let draft = translator
            .translate("Warn about lodash usage", &ctx)
            .await
            .unwrap();
        assert_eq!(draft.intent.action, PolicyAction::Deny);
        assert_eq!(draft.intent.target_value, "lodash");
        assert_eq!(draft.intent.severity, PolicySeverity::Warn);
        assert!(!draft.rules.is_empty());
    }

    #[test]
    fn test_generate_policy_name() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

        let intent = StructuredIntent {
            original: "test".to_string(),
            interpreted: "test".to_string(),
            action: PolicyAction::Deny,
            target_type: TargetType::Dependency,
            target_value: "mysql".to_string(),
            condition: None,
            severity: PolicySeverity::Block,
            confidence: 0.9,
        };

        let name = translator.generate_policy_name(&intent);
        assert_eq!(name, "no-mysql");

        let intent2 = StructuredIntent {
            original: "test".to_string(),
            interpreted: "test".to_string(),
            action: PolicyAction::Allow,
            target_type: TargetType::File,
            target_value: "README.md".to_string(),
            condition: None,
            severity: PolicySeverity::Warn,
            confidence: 0.9,
        };

        let name2 = translator.generate_policy_name(&intent2);
        assert_eq!(name2, "require-readme-md");
    }

    #[test]
    fn test_parse_intent_response_with_code_block() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

        let response = r#"```json
{
    "action": "deny",
    "target_type": "dependency",
    "target_value": "mysql",
    "condition": null,
    "severity": "block",
    "interpreted": "Block mysql",
    "confidence": 0.9
}
```"#;

        let intent = translator
            .parse_intent_response(response, "Block mysql")
            .unwrap();
        assert_eq!(intent.action, PolicyAction::Deny);
        assert_eq!(intent.target_value, "mysql");
    }

    #[test]
    fn test_is_likely_dependency() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

        assert!(translator.is_likely_dependency("mysql"));
        assert!(translator.is_likely_dependency("lodash"));
        assert!(translator.is_likely_dependency("@types/node"));
        assert!(translator.is_likely_dependency("my-package"));
    }

    #[test]
    fn test_is_likely_file() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

        assert!(translator.is_likely_file("README.md"));
        assert!(translator.is_likely_file("package.json"));
        assert!(translator.is_likely_file("Dockerfile"));
        assert!(translator.is_likely_file("SECURITY"));
    }

    #[test]
    fn test_explain_syntax_error_unbalanced_braces() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

        let explanation = translator.explain_syntax_error("Unbalanced braces in policy");
        assert!(explanation.contains("mismatched curly braces"));
    }

    #[test]
    fn test_explain_syntax_error_missing_keyword() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

        let explanation = translator.explain_syntax_error("Missing permit or forbid keyword");
        assert!(explanation.contains("permit"));
        assert!(explanation.contains("forbid"));
    }

    #[test]
    fn test_explain_syntax_error_missing_semicolon() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

        let explanation = translator.explain_syntax_error("Missing semicolon at end");
        assert!(explanation.contains("semicolon"));
    }

    #[test]
    fn test_explain_semantic_error_type_mismatch() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

        let explanation =
            translator.explain_semantic_error("Type mismatch: expected string, got int");
        assert!(explanation.contains("comparing incompatible types"));
    }

    #[test]
    fn test_explain_warning_no_principal() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

        let explanation = translator.explain_warning("Policy does not reference 'principal'");
        assert!(explanation.contains("principal"));
        assert!(explanation.contains("everyone"));
    }

    #[test]
    fn test_explain_validation_warnings() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

        let result = ValidationResult {
            is_valid: true,
            errors: vec![],
            warnings: vec![
                "Policy does not reference 'principal'".to_string(),
                "Policy does not reference 'action'".to_string(),
            ],
        };

        let explanations = translator.explain_validation_warnings(&result);
        assert_eq!(explanations.len(), 2);
        assert!(explanations[0].contains("principal"));
        assert!(explanations[1].contains("action"));
    }

    #[tokio::test]
    async fn test_explain_errors_multiple() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

        let result = ValidationResult {
            is_valid: false,
            errors: vec![
                ValidationError {
                    error_type: "syntax".to_string(),
                    message: "Unbalanced braces".to_string(),
                    rule_index: None,
                    suggestion: None,
                },
                ValidationError {
                    error_type: "syntax".to_string(),
                    message: "Missing semicolon".to_string(),
                    rule_index: None,
                    suggestion: None,
                },
            ],
            warnings: vec![],
        };

        let explanations = translator.explain_errors(&result).await;
        assert_eq!(explanations.len(), 2);
        assert!(explanations[0].contains("braces"));
        assert!(explanations[1].contains("semicolon"));
    }

    #[tokio::test]
    async fn test_cache_hit() {
        let client = Arc::new(MockLlmClient::new(vec![]));

        let mut config = PolicyTranslatorConfig::default();
        config.enable_cache = true;
        let translator = PolicyTranslator::new(client, config);

        let ctx = TranslationContext::default();

        let draft1 = translator.translate("Block mysql", &ctx).await.unwrap();
        assert!(!draft1.rules.is_empty());

        let (cache_size, _) = translator.cache_stats();
        assert_eq!(cache_size, 1);

        let draft2 = translator.translate("Block mysql", &ctx).await.unwrap();
        assert!(!draft2.rules.is_empty());
        assert_eq!(draft1.rules.len(), draft2.rules.len());
    }

    #[tokio::test]
    async fn test_cache_disabled() {
        let client = Arc::new(MockLlmClient::new(vec![]));

        let mut config = PolicyTranslatorConfig::default();
        config.enable_cache = false;
        let translator = PolicyTranslator::new(client, config);

        let ctx = TranslationContext::default();

        let _draft1 = translator.translate("Block mysql", &ctx).await.unwrap();
        let (cache_size, _) = translator.cache_stats();
        assert_eq!(cache_size, 0);
    }

    #[test]
    fn test_clear_cache() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

        let key = CacheKey {
            intent: "test".to_string(),
            scope: "project".to_string(),
            project: "".to_string(),
        };

        {
            let mut cache = translator.cache.write().unwrap();
            cache.insert(
                key,
                CacheEntry {
                    draft: PolicyDraft {
                        draft_id: "test".to_string(),
                        status: DraftStatus::Validated,
                        name: "test".to_string(),
                        intent: StructuredIntent {
                            original: "test".to_string(),
                            interpreted: "test".to_string(),
                            action: PolicyAction::Deny,
                            target_type: TargetType::Dependency,
                            target_value: "test".to_string(),
                            condition: None,
                            severity: PolicySeverity::Block,
                            confidence: 0.9,
                        },
                        rules: vec![],
                        explanation: "test".to_string(),
                        validation: ValidationResult {
                            is_valid: true,
                            errors: vec![],
                            warnings: vec![],
                        },
                    },
                    created_at: Instant::now(),
                },
            );
        }

        let (size_before, _) = translator.cache_stats();
        assert_eq!(size_before, 1);

        translator.clear_cache();

        let (size_after, _) = translator.cache_stats();
        assert_eq!(size_after, 0);
    }

    #[test]
    fn test_cache_key_normalization() {
        let ctx = TranslationContext::default();

        let key1 = CacheKey::from_context("Block MySQL", &ctx);
        let key2 = CacheKey::from_context("block mysql", &ctx);
        let key3 = CacheKey::from_context("  BLOCK MYSQL  ", &ctx);

        assert_eq!(key1, key2);
        assert_eq!(key2, key3);
    }

    mod translation_accuracy {
        use super::*;
        use mk_core::types::ConstraintOperator;

        #[tokio::test]
        async fn test_deny_dependency_produces_must_not_use() {
            let client = Arc::new(MockLlmClient::new(vec![]));
            let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());
            let ctx = TranslationContext::default();

            let draft = translator.translate("Block mysql", &ctx).await.unwrap();
            assert_eq!(draft.intent.action, PolicyAction::Deny);
            assert!(!draft.rules.is_empty());
            assert_eq!(draft.rules[0].operator, ConstraintOperator::MustNotUse);
        }

        #[tokio::test]
        async fn test_allow_file_produces_must_exist() {
            let client = Arc::new(MockLlmClient::new(vec![]));
            let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());
            let ctx = TranslationContext::default();

            let draft = translator
                .translate("Require README.md", &ctx)
                .await
                .unwrap();
            assert_eq!(draft.intent.action, PolicyAction::Allow);
            assert!(!draft.rules.is_empty());
            assert_eq!(draft.rules[0].operator, ConstraintOperator::MustExist);
        }

        #[test]
        fn test_validate_rules_catches_empty_id() {
            let client = Arc::new(MockLlmClient::new(vec![]));
            let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

            let rules = vec![PolicyRule {
                id: "".to_string(),
                rule_type: RuleType::Deny,
                target: ConstraintTarget::Dependency,
                operator: ConstraintOperator::MustNotUse,
                value: serde_json::Value::String("mysql".to_string()),
                severity: ConstraintSeverity::Block,
                message: "Block mysql".to_string(),
            }];

            let result = translator.validate_rules(&rules);
            assert!(!result.is_valid);
            assert!(result.errors.iter().any(|e| e.message.contains("ID")));
        }

        #[test]
        fn test_validate_rules_catches_empty_value() {
            let client = Arc::new(MockLlmClient::new(vec![]));
            let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

            let rules = vec![PolicyRule {
                id: "no-mysql".to_string(),
                rule_type: RuleType::Deny,
                target: ConstraintTarget::Dependency,
                operator: ConstraintOperator::MustNotUse,
                value: serde_json::Value::String("".to_string()),
                severity: ConstraintSeverity::Block,
                message: "Block mysql".to_string(),
            }];

            let result = translator.validate_rules(&rules);
            assert!(!result.is_valid);
            assert!(result.errors.iter().any(|e| e.message.contains("empty")));
        }

        #[test]
        fn test_validate_rules_passes_valid_rule() {
            let client = Arc::new(MockLlmClient::new(vec![]));
            let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

            let rules = vec![PolicyRule {
                id: "no-mysql".to_string(),
                rule_type: RuleType::Deny,
                target: ConstraintTarget::Dependency,
                operator: ConstraintOperator::MustNotUse,
                value: serde_json::Value::String("mysql".to_string()),
                severity: ConstraintSeverity::Block,
                message: "Block mysql".to_string(),
            }];

            let result = translator.validate_rules(&rules);
            assert!(result.is_valid);
        }

        #[tokio::test]
        async fn test_template_extraction_accuracy_block_patterns() {
            let client = Arc::new(MockLlmClient::new(vec![]));
            let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());
            let ctx = TranslationContext::default();

            let test_cases = [
                ("Block mysql", "mysql", PolicyAction::Deny),
                ("Forbid lodash", "lodash", PolicyAction::Deny),
                ("Prevent axios", "axios", PolicyAction::Deny),
            ];

            for (input, expected_target, expected_action) in test_cases {
                if let Some(intent) = translator.template_extract(input, &ctx) {
                    assert_eq!(intent.action, expected_action, "Input: {}", input);
                    assert!(
                        intent
                            .target_value
                            .to_lowercase()
                            .contains(&expected_target.to_lowercase()),
                        "Input '{}' should extract target containing '{}', got '{}'",
                        input,
                        expected_target,
                        intent.target_value
                    );
                }
            }
        }

        #[tokio::test]
        async fn test_template_extraction_accuracy_require_patterns() {
            let client = Arc::new(MockLlmClient::new(vec![]));
            let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());
            let ctx = TranslationContext::default();

            let test_cases = [
                ("Require README.md", "README.md", PolicyAction::Allow),
                ("Must have LICENSE", "LICENSE", PolicyAction::Allow),
                ("Need CHANGELOG.md", "CHANGELOG.md", PolicyAction::Allow),
            ];

            for (input, expected_target, expected_action) in test_cases {
                if let Some(intent) = translator.template_extract(input, &ctx) {
                    assert_eq!(intent.action, expected_action, "Input: {}", input);
                    assert!(
                        intent
                            .target_value
                            .to_uppercase()
                            .contains(&expected_target.to_uppercase()),
                        "Input '{}' should extract target containing '{}', got '{}'",
                        input,
                        expected_target,
                        intent.target_value
                    );
                }
            }
        }

        #[test]
        fn test_policy_name_generation_accuracy() {
            let client = Arc::new(MockLlmClient::new(vec![]));
            let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

            let test_cases = [
                (PolicyAction::Deny, "mysql", "no-mysql"),
                (PolicyAction::Deny, "@babel/core", "no-babel-core"),
                (PolicyAction::Allow, "README.md", "require-readme-md"),
                (PolicyAction::Allow, "tests/", "require-tests-"),
            ];

            for (action, target, expected_prefix) in test_cases {
                let intent = StructuredIntent {
                    original: "test".to_string(),
                    interpreted: "test".to_string(),
                    action,
                    target_type: TargetType::Dependency,
                    target_value: target.to_string(),
                    condition: None,
                    severity: PolicySeverity::Block,
                    confidence: 0.9,
                };

                let name = translator.generate_policy_name(&intent);
                assert!(
                    name.starts_with(expected_prefix),
                    "Action {:?} with target '{}' should generate name starting with '{}', got \
                     '{}'",
                    action,
                    target,
                    expected_prefix,
                    name
                );
            }
        }

        #[test]
        fn test_intent_to_operator_mapping() {
            let client = Arc::new(MockLlmClient::new(vec![]));
            let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

            let test_cases = [
                (
                    PolicyAction::Deny,
                    TargetType::Dependency,
                    ConstraintOperator::MustNotUse,
                ),
                (
                    PolicyAction::Allow,
                    TargetType::Dependency,
                    ConstraintOperator::MustUse,
                ),
                (
                    PolicyAction::Deny,
                    TargetType::File,
                    ConstraintOperator::MustNotExist,
                ),
                (
                    PolicyAction::Allow,
                    TargetType::File,
                    ConstraintOperator::MustExist,
                ),
                (
                    PolicyAction::Deny,
                    TargetType::Code,
                    ConstraintOperator::MustNotMatch,
                ),
                (
                    PolicyAction::Allow,
                    TargetType::Code,
                    ConstraintOperator::MustMatch,
                ),
            ];

            for (action, target_type, expected_operator) in test_cases {
                let intent = StructuredIntent {
                    original: "test".to_string(),
                    interpreted: "test".to_string(),
                    action,
                    target_type,
                    target_value: "test".to_string(),
                    condition: None,
                    severity: PolicySeverity::Block,
                    confidence: 0.9,
                };

                let operator = translator.intent_to_operator(&intent);
                assert_eq!(
                    operator, expected_operator,
                    "Action {:?} + TargetType {:?} should map to {:?}",
                    action, target_type, expected_operator
                );
            }
        }
    }

    #[test]
    fn test_policy_action_display_and_invalid_parse() {
        assert_eq!(PolicyAction::Allow.to_string(), "allow");
        assert_eq!(PolicyAction::Deny.to_string(), "deny");
        assert!("maybe".parse::<PolicyAction>().is_err());
    }

    #[test]
    fn test_target_type_display_roundtrip_and_invalid_parse() {
        let cases = [
            (TargetType::Dependency, "dependency"),
            (TargetType::File, "file"),
            (TargetType::Code, "code"),
            (TargetType::Import, "import"),
            (TargetType::Config, "config"),
        ];

        for (target_type, value) in cases {
            assert_eq!(target_type.to_string(), value);
            assert_eq!(value.parse::<TargetType>().unwrap(), target_type);
        }

        assert_eq!("route".parse::<TargetType>().unwrap(), TargetType::Config);
        assert!("database-row".parse::<TargetType>().is_err());
    }

    #[test]
    fn test_constraint_target_conversion_roundtrip() {
        let cases = [
            ConstraintTarget::Dependency,
            ConstraintTarget::File,
            ConstraintTarget::Code,
            ConstraintTarget::Import,
            ConstraintTarget::Config,
        ];

        for target in cases {
            let translated: TargetType = target.into();
            let back: ConstraintTarget = translated.into();
            assert_eq!(back, target);
        }
    }

    #[test]
    fn test_policy_severity_display_conversion_and_invalid_parse() {
        assert_eq!(PolicySeverity::Info.to_string(), "info");
        assert_eq!(PolicySeverity::Warn.to_string(), "warn");
        assert_eq!(PolicySeverity::Block.to_string(), "block");
        assert_eq!(
            "critical".parse::<PolicySeverity>().unwrap(),
            PolicySeverity::Block
        );
        assert_eq!(PolicySeverity::Warn, ConstraintSeverity::Warn.into());
        let converted: ConstraintSeverity = PolicySeverity::Block.into();
        assert_eq!(converted, ConstraintSeverity::Block);
        assert!("fatalish".parse::<PolicySeverity>().is_err());
    }

    #[test]
    fn test_policy_scope_display_aliases_and_invalid_parse() {
        assert_eq!(PolicyScope::Tenant.to_string(), "tenant");
        assert_eq!(PolicyScope::Org.to_string(), "org");
        assert_eq!(PolicyScope::Team.to_string(), "team");
        assert_eq!(PolicyScope::Project.to_string(), "project");
        assert_eq!(
            "enterprise".parse::<PolicyScope>().unwrap(),
            PolicyScope::Tenant
        );
        assert_eq!(
            "department".parse::<PolicyScope>().unwrap(),
            PolicyScope::Org
        );
        assert_eq!("repo".parse::<PolicyScope>().unwrap(), PolicyScope::Project);
        assert!("workspace".parse::<PolicyScope>().is_err());
    }

    #[test]
    fn test_policy_translator_config_default_values() {
        let config = PolicyTranslatorConfig::default();
        assert!(config.use_templates);
        assert_eq!(config.min_confidence, 0.7);
        assert_eq!(config.max_retries, 3);
        assert!(config.strict_validation);
        assert_eq!(config.few_shot_count, 10);
        assert!(config.include_schema);
        assert!(config.enable_cache);
        assert_eq!(config.cache_ttl_secs, 3600);
        assert_eq!(config.cache_max_entries, 1000);
    }

    #[test]
    fn test_parse_intent_response_handles_plain_code_block_and_invalid_json() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

        let response = r#"```
{
    "action": "allow",
    "target_type": "file",
    "target_value": "README.md",
    "condition": null,
    "severity": "info",
    "interpreted": "Require README",
    "confidence": 0.91
}
```"#;

        let intent = translator
            .parse_intent_response(response, "Require README")
            .unwrap();
        assert_eq!(intent.action, PolicyAction::Allow);
        assert_eq!(intent.target_type, TargetType::File);
        assert_eq!(intent.severity, PolicySeverity::Info);

        let err = translator
            .parse_intent_response("{ definitely not json", "broken")
            .unwrap_err();
        assert!(matches!(err, PolicyTranslatorError::ParseError(_)));
    }

    #[test]
    fn test_explain_syntax_error_additional_branches() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

        assert!(
            translator
                .explain_syntax_error("Missing parenthesis around condition")
                .contains("parentheses")
        );
        assert!(
            translator
                .explain_syntax_error("Quote string literal properly")
                .contains("double quotes")
        );
        assert!(
            translator
                .explain_syntax_error("Unexpected token near resource")
                .contains("Syntax error:")
        );
    }

    #[test]
    fn test_explain_semantic_error_additional_branches() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

        assert!(
            translator
                .explain_semantic_error("when condition must evaluate to bool")
                .contains("when")
        );
        assert!(
            translator
                .explain_semantic_error("variable not defined in scope")
                .contains("hasn't been defined")
        );
        assert!(
            translator
                .explain_semantic_error("contradictory rule semantics")
                .contains("Policy logic error")
        );
    }

    #[test]
    fn test_explain_warning_additional_branches() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let translator = PolicyTranslator::new(client, PolicyTranslatorConfig::default());

        assert!(
            translator
                .explain_warning("Policy does not reference resource")
                .contains("resource")
        );
        assert!(
            translator
                .explain_warning("Missing semicolon for consistency")
                .contains("semicolon")
        );
        assert!(
            translator
                .explain_warning("Custom warning text")
                .contains("Note:")
        );
    }

    #[test]
    fn test_cache_stats_and_ttl_expiry_behavior() {
        let client = Arc::new(MockLlmClient::new(vec![]));
        let mut config = PolicyTranslatorConfig::default();
        config.cache_ttl_secs = 0;
        let translator = PolicyTranslator::new(client, config);

        let ctx = TranslationContext::default();
        let key = CacheKey::from_context("Block mysql", &ctx);
        let draft = PolicyDraft {
            draft_id: "draft-1".to_string(),
            status: DraftStatus::Validated,
            name: "no-mysql".to_string(),
            intent: StructuredIntent {
                original: "Block mysql".to_string(),
                interpreted: "Block mysql".to_string(),
                action: PolicyAction::Deny,
                target_type: TargetType::Dependency,
                target_value: "mysql".to_string(),
                condition: None,
                severity: PolicySeverity::Block,
                confidence: 0.9,
            },
            rules: vec![],
            explanation: "test".to_string(),
            validation: ValidationResult {
                is_valid: true,
                errors: vec![],
                warnings: vec![],
            },
        };

        translator.cache.write().unwrap().insert(
            key,
            CacheEntry {
                draft,
                created_at: Instant::now(),
            },
        );

        assert_eq!(translator.cache_stats(), (1, 1000));
        assert!(translator.get_cached("Block mysql", &ctx).is_none());
    }
}
