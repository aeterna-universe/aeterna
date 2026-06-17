use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyLayer {
    Tenant,
    Org,
    Team,
    Project,
}

impl std::fmt::Display for PolicyLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolicyLayer::Tenant => write!(f, "tenant"),
            PolicyLayer::Org => write!(f, "org"),
            PolicyLayer::Team => write!(f, "team"),
            PolicyLayer::Project => write!(f, "project"),
        }
    }
}

impl std::str::FromStr for PolicyLayer {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "tenant" => Ok(PolicyLayer::Tenant),
            "org" | "organization" => Ok(PolicyLayer::Org),
            "team" => Ok(PolicyLayer::Team),
            "project" => Ok(PolicyLayer::Project),
            _ => Err(format!(
                "Invalid layer: {}. Use: tenant, org, team, project",
                s
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyMode {
    Mandatory,
    Optional,
}

impl std::str::FromStr for PolicyMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "mandatory" => Ok(PolicyMode::Mandatory),
            "optional" => Ok(PolicyMode::Optional),
            _ => Err(format!("Invalid mode: {}. Use: mandatory, optional", s)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warn,
    Error,
    Block,
}

impl std::str::FromStr for Severity {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "info" => Ok(Severity::Info),
            "warn" | "warning" => Ok(Severity::Warn),
            "error" => Ok(Severity::Error),
            "block" => Ok(Severity::Block),
            _ => Err(format!(
                "Invalid severity: {}. Use: info, warn, error, block",
                s
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleTarget {
    Dependency,
    File,
    Code,
    Config,
}

impl std::str::FromStr for RuleTarget {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "dependency" | "dep" => Ok(RuleTarget::Dependency),
            "file" => Ok(RuleTarget::File),
            "code" => Ok(RuleTarget::Code),
            "config" | "configuration" => Ok(RuleTarget::Config),
            _ => Err(format!(
                "Invalid target: {}. Use: dependency, file, code, config",
                s
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DraftStatus {
    Pending,
    Submitted,
    Approved,
    Rejected,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub id: String,
    pub rule_type: String,
    pub target: RuleTarget,
    pub pattern: String,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub layer: PolicyLayer,
    pub mode: PolicyMode,
    pub rules: Vec<PolicyRule>,
    pub cedar_policy: Option<String>,
    pub tenant_id: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub created_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDraft {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub template: Option<String>,
    pub layer: PolicyLayer,
    pub mode: PolicyMode,
    pub severity: Severity,
    pub rules: Vec<PolicyRule>,
    pub cedar_policy: Option<String>,
    pub status: DraftStatus,
    pub tenant_id: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub created_by: String,
    pub submitted_at: Option<i64>,
    pub reviewed_at: Option<i64>,
    pub reviewed_by: Option<String>,
    pub rejection_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SimulationScenario {
    pub scenario_type: String,
    pub input: String,
    pub context: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResult {
    pub policy_id: String,
    pub scenario_type: String,
    pub input: String,
    pub decision: String,
    pub matched_rules: Vec<String>,
    pub violations: Vec<SimulationViolation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationViolation {
    pub rule_id: String,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationWarning>,
}

#[derive(Debug, Clone)]
pub struct ValidationError {
    pub code: String,
    pub message: String,
    pub location: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ValidationWarning {
    pub code: String,
    pub message: String,
}

pub struct MockPolicyStorage {
    pub policies: Arc<RwLock<HashMap<String, Policy>>>,
    drafts: Arc<RwLock<HashMap<String, PolicyDraft>>>,
    templates: Arc<RwLock<HashMap<String, PolicyTemplate>>>,
}

#[derive(Debug, Clone)]
pub struct PolicyTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub rules: Vec<PolicyRule>,
    pub default_layer: PolicyLayer,
    pub default_mode: PolicyMode,
}

impl Default for MockPolicyStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MockPolicyStorage {
    pub fn new() -> Self {
        let mut templates = HashMap::new();

        templates.insert(
            "security-baseline".to_string(),
            PolicyTemplate {
                id: "security-baseline".to_string(),
                name: "Security Baseline".to_string(),
                description: "Blocks critical CVEs, requires SECURITY.md".to_string(),
                rules: vec![
                    PolicyRule {
                        id: "block-critical-cve".to_string(),
                        rule_type: "must_not_use".to_string(),
                        target: RuleTarget::Dependency,
                        pattern: "cve:critical".to_string(),
                        severity: Severity::Block,
                        message: "Critical CVE found in dependency".to_string(),
                    },
                    PolicyRule {
                        id: "require-security-md".to_string(),
                        rule_type: "must_exist".to_string(),
                        target: RuleTarget::File,
                        pattern: "SECURITY.md".to_string(),
                        severity: Severity::Warn,
                        message: "SECURITY.md file required".to_string(),
                    },
                ],
                default_layer: PolicyLayer::Tenant,
                default_mode: PolicyMode::Mandatory,
            },
        );

        templates.insert(
            "code-style".to_string(),
            PolicyTemplate {
                id: "code-style".to_string(),
                name: "Code Style".to_string(),
                description: "Enforces code style and formatting rules".to_string(),
                rules: vec![PolicyRule {
                    id: "no-console-log".to_string(),
                    rule_type: "must_not_match".to_string(),
                    target: RuleTarget::Code,
                    pattern: r"console\.log".to_string(),
                    severity: Severity::Warn,
                    message: "Avoid console.log in production code".to_string(),
                }],
                default_layer: PolicyLayer::Team,
                default_mode: PolicyMode::Optional,
            },
        );

        templates.insert(
            "dependency-audit".to_string(),
            PolicyTemplate {
                id: "dependency-audit".to_string(),
                name: "Dependency Audit".to_string(),
                description: "Audits dependencies for licenses and vulnerabilities".to_string(),
                rules: vec![PolicyRule {
                    id: "no-gpl".to_string(),
                    rule_type: "must_not_use".to_string(),
                    target: RuleTarget::Dependency,
                    pattern: "license:GPL".to_string(),
                    severity: Severity::Error,
                    message: "GPL licensed dependencies not allowed".to_string(),
                }],
                default_layer: PolicyLayer::Org,
                default_mode: PolicyMode::Mandatory,
            },
        );

        Self {
            policies: Arc::new(RwLock::new(HashMap::new())),
            drafts: Arc::new(RwLock::new(HashMap::new())),
            templates: Arc::new(RwLock::new(templates)),
        }
    }

    pub async fn create_draft(&self, draft: PolicyDraft) -> Result<String, String> {
        if draft.name.is_empty() {
            return Err("Policy name cannot be empty".to_string());
        }
        if draft.description.is_none() && draft.template.is_none() {
            return Err("Policy must have description or template".to_string());
        }

        let id = draft.id.clone();
        self.drafts.write().await.insert(id.clone(), draft);
        Ok(id)
    }

    pub async fn get_draft(&self, id: &str) -> Result<Option<PolicyDraft>, String> {
        let drafts = self.drafts.read().await;
        Ok(drafts.get(id).cloned())
    }

    pub async fn list_drafts(&self, tenant_id: &str) -> Result<Vec<PolicyDraft>, String> {
        let drafts = self.drafts.read().await;
        Ok(drafts
            .values()
            .filter(|d| d.tenant_id == tenant_id)
            .cloned()
            .collect())
    }

    pub async fn list_pending_drafts(&self, tenant_id: &str) -> Result<Vec<PolicyDraft>, String> {
        let drafts = self.drafts.read().await;
        Ok(drafts
            .values()
            .filter(|d| d.tenant_id == tenant_id && d.status == DraftStatus::Pending)
            .cloned()
            .collect())
    }

    pub async fn submit_draft(&self, id: &str) -> Result<(), String> {
        let mut drafts = self.drafts.write().await;
        let draft = drafts
            .get_mut(id)
            .ok_or_else(|| format!("Draft not found: {}", id))?;

        if draft.status != DraftStatus::Pending {
            return Err(format!("Cannot submit draft in {:?} status", draft.status));
        }

        draft.status = DraftStatus::Submitted;
        draft.submitted_at = Some(Utc::now().timestamp());
        Ok(())
    }

    pub async fn approve_draft(&self, id: &str, reviewer: &str) -> Result<Policy, String> {
        let mut drafts = self.drafts.write().await;
        let draft = drafts
            .get_mut(id)
            .ok_or_else(|| format!("Draft not found: {}", id))?;

        if draft.status != DraftStatus::Submitted {
            return Err(format!("Cannot approve draft in {:?} status", draft.status));
        }

        draft.status = DraftStatus::Approved;
        draft.reviewed_at = Some(Utc::now().timestamp());
        draft.reviewed_by = Some(reviewer.to_string());

        let policy = Policy {
            id: format!("policy-{}", draft.name.replace(" ", "-").to_lowercase()),
            name: draft.name.clone(),
            description: draft.description.clone(),
            layer: draft.layer,
            mode: draft.mode,
            rules: draft.rules.clone(),
            cedar_policy: draft.cedar_policy.clone(),
            tenant_id: draft.tenant_id.clone(),
            created_at: Utc::now().timestamp(),
            updated_at: Utc::now().timestamp(),
            created_by: draft.created_by.clone(),
        };

        drop(drafts);
        self.policies
            .write()
            .await
            .insert(policy.id.clone(), policy.clone());
        Ok(policy)
    }

    pub async fn reject_draft(&self, id: &str, reviewer: &str, reason: &str) -> Result<(), String> {
        let mut drafts = self.drafts.write().await;
        let draft = drafts
            .get_mut(id)
            .ok_or_else(|| format!("Draft not found: {}", id))?;

        if draft.status != DraftStatus::Submitted {
            return Err(format!("Cannot reject draft in {:?} status", draft.status));
        }

        draft.status = DraftStatus::Rejected;
        draft.reviewed_at = Some(Utc::now().timestamp());
        draft.reviewed_by = Some(reviewer.to_string());
        draft.rejection_reason = Some(reason.to_string());
        Ok(())
    }

    pub async fn get_policy(&self, id: &str) -> Result<Option<Policy>, String> {
        let policies = self.policies.read().await;
        Ok(policies.get(id).cloned())
    }

    pub async fn list_policies(
        &self,
        tenant_id: &str,
        layer: Option<PolicyLayer>,
        mode: Option<PolicyMode>,
    ) -> Result<Vec<Policy>, String> {
        let policies = self.policies.read().await;
        Ok(policies
            .values()
            .filter(|p| {
                p.tenant_id == tenant_id
                    && layer.is_none_or(|l| p.layer == l)
                    && mode.is_none_or(|m| p.mode == m)
            })
            .cloned()
            .collect())
    }

    pub async fn get_template(&self, id: &str) -> Result<Option<PolicyTemplate>, String> {
        let templates = self.templates.read().await;
        Ok(templates.get(id).cloned())
    }

    pub async fn list_templates(&self) -> Result<Vec<PolicyTemplate>, String> {
        let templates = self.templates.read().await;
        Ok(templates.values().cloned().collect())
    }

    pub fn validate_policy(&self, policy: &Policy) -> ValidationResult {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        if policy.name.is_empty() {
            errors.push(ValidationError {
                code: "E001".to_string(),
                message: "Policy name cannot be empty".to_string(),
                location: Some("name".to_string()),
            });
        }

        if policy.rules.is_empty() {
            warnings.push(ValidationWarning {
                code: "W001".to_string(),
                message: "Policy has no rules defined".to_string(),
            });
        }

        for rule in &policy.rules {
            if rule.pattern.is_empty() {
                errors.push(ValidationError {
                    code: "E002".to_string(),
                    message: format!("Rule '{}' has empty pattern", rule.id),
                    location: Some(format!("rules[{}].pattern", rule.id)),
                });
            }
        }

        if let Some(ref cedar) = policy.cedar_policy
            && !cedar.contains("permit")
            && !cedar.contains("forbid")
        {
            errors.push(ValidationError {
                code: "E003".to_string(),
                message: "Cedar policy must contain permit or forbid statement".to_string(),
                location: Some("cedar_policy".to_string()),
            });
        }

        ValidationResult {
            valid: errors.is_empty(),
            errors,
            warnings,
        }
    }

    pub fn validate_draft(&self, draft: &PolicyDraft) -> ValidationResult {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        if draft.name.is_empty() {
            errors.push(ValidationError {
                code: "E001".to_string(),
                message: "Draft name cannot be empty".to_string(),
                location: Some("name".to_string()),
            });
        }

        if draft.description.is_none() && draft.template.is_none() {
            errors.push(ValidationError {
                code: "E004".to_string(),
                message: "Draft must have description or template".to_string(),
                location: None,
            });
        }

        let now = Utc::now().timestamp();
        if draft.expires_at < now {
            warnings.push(ValidationWarning {
                code: "W002".to_string(),
                message: "Draft has expired".to_string(),
            });
        }

        ValidationResult {
            valid: errors.is_empty(),
            errors,
            warnings,
        }
    }

    pub async fn simulate_policy(
        &self,
        policy_id: &str,
        scenario: &SimulationScenario,
    ) -> Result<SimulationResult, String> {
        let policies = self.policies.read().await;
        let policy = policies
            .get(policy_id)
            .ok_or_else(|| format!("Policy not found: {}", policy_id))?;

        let mut matched_rules = Vec::new();
        let mut violations = Vec::new();

        for rule in &policy.rules {
            let matches = match scenario.scenario_type.as_str() {
                "dependency-add" => {
                    rule.target == RuleTarget::Dependency && scenario.input.contains(&rule.pattern)
                }
                "file-create" => {
                    rule.target == RuleTarget::File && scenario.input.contains(&rule.pattern)
                }
                "code-change" => {
                    rule.target == RuleTarget::Code && scenario.input.contains(&rule.pattern)
                }
                _ => false,
            };

            if matches {
                matched_rules.push(rule.id.clone());
                if rule.rule_type.starts_with("must_not") {
                    violations.push(SimulationViolation {
                        rule_id: rule.id.clone(),
                        severity: rule.severity,
                        message: rule.message.clone(),
                    });
                }
            }
        }

        let decision = if violations.iter().any(|v| v.severity == Severity::Block) {
            "blocked"
        } else if violations.iter().any(|v| v.severity == Severity::Error) {
            "error"
        } else if !violations.is_empty() {
            "warn"
        } else {
            "allow"
        };

        Ok(SimulationResult {
            policy_id: policy_id.to_string(),
            scenario_type: scenario.scenario_type.clone(),
            input: scenario.input.clone(),
            decision: decision.to_string(),
            matched_rules,
            violations,
        })
    }
}

fn create_test_draft(
    id: &str,
    name: &str,
    description: Option<&str>,
    template: Option<&str>,
    layer: PolicyLayer,
) -> PolicyDraft {
    let now = Utc::now().timestamp();
    PolicyDraft {
        id: id.to_string(),
        name: name.to_string(),
        description: description.map(String::from),
        template: template.map(String::from),
        layer,
        mode: PolicyMode::Mandatory,
        severity: Severity::Warn,
        rules: Vec::new(),
        cedar_policy: None,
        status: DraftStatus::Pending,
        tenant_id: "test-tenant".to_string(),
        created_at: now,
        expires_at: now + 86400,
        created_by: "test-user".to_string(),
        submitted_at: None,
        reviewed_at: None,
        reviewed_by: None,
        rejection_reason: None,
    }
}

fn create_test_policy(id: &str, name: &str, layer: PolicyLayer, rules: Vec<PolicyRule>) -> Policy {
    let now = Utc::now().timestamp();
    Policy {
        id: id.to_string(),
        name: name.to_string(),
        description: Some("Test policy".to_string()),
        layer,
        mode: PolicyMode::Mandatory,
        rules,
        cedar_policy: Some("permit (principal, action, resource);".to_string()),
        tenant_id: "test-tenant".to_string(),
        created_at: now,
        updated_at: now,
        created_by: "test-user".to_string(),
    }
}

#[cfg(test)]
mod layer_parsing_tests {
    use super::*;

    #[test]
    fn test_parse_valid_layers() {
        assert_eq!(
            "tenant".parse::<PolicyLayer>().unwrap(),
            PolicyLayer::Tenant
        );
        assert_eq!("org".parse::<PolicyLayer>().unwrap(), PolicyLayer::Org);
        assert_eq!(
            "organization".parse::<PolicyLayer>().unwrap(),
            PolicyLayer::Org
        );
        assert_eq!("team".parse::<PolicyLayer>().unwrap(), PolicyLayer::Team);
        assert_eq!(
            "project".parse::<PolicyLayer>().unwrap(),
            PolicyLayer::Project
        );
    }

    #[test]
    fn test_parse_layers_case_insensitive() {
        assert_eq!(
            "COMPANY".parse::<PolicyLayer>().unwrap(),
            PolicyLayer::Tenant
        );
        assert_eq!("Team".parse::<PolicyLayer>().unwrap(), PolicyLayer::Team);
        assert_eq!(
            "PROJECT".parse::<PolicyLayer>().unwrap(),
            PolicyLayer::Project
        );
    }

    #[test]
    fn test_parse_invalid_layer() {
        let result = "invalid".parse::<PolicyLayer>();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid layer"));
    }
}

#[cfg(test)]
mod mode_parsing_tests {
    use super::*;

    #[test]
    fn test_parse_valid_modes() {
        assert_eq!(
            "mandatory".parse::<PolicyMode>().unwrap(),
            PolicyMode::Mandatory
        );
        assert_eq!(
            "optional".parse::<PolicyMode>().unwrap(),
            PolicyMode::Optional
        );
    }

    #[test]
    fn test_parse_invalid_mode() {
        let result = "required".parse::<PolicyMode>();
        assert!(result.is_err());
    }
}

#[cfg(test)]
mod severity_parsing_tests {
    use super::*;

    #[test]
    fn test_parse_valid_severities() {
        assert_eq!("info".parse::<Severity>().unwrap(), Severity::Info);
        assert_eq!("warn".parse::<Severity>().unwrap(), Severity::Warn);
        assert_eq!("warning".parse::<Severity>().unwrap(), Severity::Warn);
        assert_eq!("error".parse::<Severity>().unwrap(), Severity::Error);
        assert_eq!("block".parse::<Severity>().unwrap(), Severity::Block);
    }

    #[test]
    fn test_parse_invalid_severity() {
        let result = "critical".parse::<Severity>();
        assert!(result.is_err());
    }
}

#[cfg(test)]
mod target_parsing_tests {
    use super::*;

    #[test]
    fn test_parse_valid_targets() {
        assert_eq!(
            "dependency".parse::<RuleTarget>().unwrap(),
            RuleTarget::Dependency
        );
        assert_eq!("dep".parse::<RuleTarget>().unwrap(), RuleTarget::Dependency);
        assert_eq!("file".parse::<RuleTarget>().unwrap(), RuleTarget::File);
        assert_eq!("code".parse::<RuleTarget>().unwrap(), RuleTarget::Code);
        assert_eq!("config".parse::<RuleTarget>().unwrap(), RuleTarget::Config);
        assert_eq!(
            "configuration".parse::<RuleTarget>().unwrap(),
            RuleTarget::Config
        );
    }

    #[test]
    fn test_parse_invalid_target() {
        let result = "unknown".parse::<RuleTarget>();
        assert!(result.is_err());
    }
}

#[cfg(test)]
mod draft_tests {
    use super::*;

    #[tokio::test]
    async fn test_create_draft_with_description() {
        let storage = MockPolicyStorage::new();
        let draft = create_test_draft(
            "draft-1",
            "Block CVEs",
            Some("Block all critical CVEs"),
            None,
            PolicyLayer::Project,
        );

        let result = storage.create_draft(draft).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "draft-1");
    }

    #[tokio::test]
    async fn test_create_draft_with_template() {
        let storage = MockPolicyStorage::new();
        let draft = create_test_draft(
            "draft-1",
            "Security Policy",
            None,
            Some("security-baseline"),
            PolicyLayer::Tenant,
        );

        let result = storage.create_draft(draft).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_create_draft_requires_description_or_template() {
        let storage = MockPolicyStorage::new();
        let draft = create_test_draft("draft-1", "Empty Policy", None, None, PolicyLayer::Project);

        let result = storage.create_draft(draft).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("description or template"));
    }

    #[tokio::test]
    async fn test_create_draft_requires_name() {
        let storage = MockPolicyStorage::new();
        let mut draft = create_test_draft(
            "draft-1",
            "Test",
            Some("Test policy"),
            None,
            PolicyLayer::Project,
        );
        draft.name = String::new();

        let result = storage.create_draft(draft).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("name cannot be empty"));
    }

    #[tokio::test]
    async fn test_get_draft() {
        let storage = MockPolicyStorage::new();
        let draft = create_test_draft(
            "draft-1",
            "Test Policy",
            Some("Test"),
            None,
            PolicyLayer::Project,
        );
        storage.create_draft(draft).await.unwrap();

        let result = storage.get_draft("draft-1").await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "Test Policy");
    }

    #[tokio::test]
    async fn test_get_draft_not_found() {
        let storage = MockPolicyStorage::new();
        let result = storage.get_draft("nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_list_drafts() {
        let storage = MockPolicyStorage::new();

        let draft1 = create_test_draft("d1", "Policy 1", Some("Desc"), None, PolicyLayer::Project);
        storage.create_draft(draft1).await.unwrap();

        let draft2 = create_test_draft("d2", "Policy 2", Some("Desc"), None, PolicyLayer::Team);
        storage.create_draft(draft2).await.unwrap();

        let drafts = storage.list_drafts("test-tenant").await.unwrap();
        assert_eq!(drafts.len(), 2);
    }

    #[tokio::test]
    async fn test_list_pending_drafts() {
        let storage = MockPolicyStorage::new();

        let draft1 = create_test_draft("d1", "Policy 1", Some("Desc"), None, PolicyLayer::Project);
        storage.create_draft(draft1).await.unwrap();

        let draft2 = create_test_draft("d2", "Policy 2", Some("Desc"), None, PolicyLayer::Team);
        storage.create_draft(draft2).await.unwrap();
        storage.submit_draft("d2").await.unwrap();

        let pending = storage.list_pending_drafts("test-tenant").await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "d1");
    }
}

#[cfg(test)]
mod draft_workflow_tests {
    use super::*;

    #[tokio::test]
    async fn test_submit_draft() {
        let storage = MockPolicyStorage::new();
        let draft = create_test_draft("d1", "Policy", Some("Desc"), None, PolicyLayer::Project);
        storage.create_draft(draft).await.unwrap();

        let result = storage.submit_draft("d1").await;
        assert!(result.is_ok());

        let submitted = storage.get_draft("d1").await.unwrap().unwrap();
        assert_eq!(submitted.status, DraftStatus::Submitted);
        assert!(submitted.submitted_at.is_some());
    }

    #[tokio::test]
    async fn test_cannot_submit_already_submitted() {
        let storage = MockPolicyStorage::new();
        let draft = create_test_draft("d1", "Policy", Some("Desc"), None, PolicyLayer::Project);
        storage.create_draft(draft).await.unwrap();
        storage.submit_draft("d1").await.unwrap();

        let result = storage.submit_draft("d1").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cannot submit"));
    }

    #[tokio::test]
    async fn test_submit_nonexistent_draft() {
        let storage = MockPolicyStorage::new();
        let result = storage.submit_draft("nonexistent").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[tokio::test]
    async fn test_approve_draft() {
        let storage = MockPolicyStorage::new();
        let draft = create_test_draft("d1", "My Policy", Some("Desc"), None, PolicyLayer::Project);
        storage.create_draft(draft).await.unwrap();
        storage.submit_draft("d1").await.unwrap();

        let result = storage.approve_draft("d1", "reviewer-1").await;
        assert!(result.is_ok());

        let policy = result.unwrap();
        assert_eq!(policy.name, "My Policy");

        let approved = storage.get_draft("d1").await.unwrap().unwrap();
        assert_eq!(approved.status, DraftStatus::Approved);
        assert_eq!(approved.reviewed_by.as_deref(), Some("reviewer-1"));
    }

    #[tokio::test]
    async fn test_cannot_approve_pending_draft() {
        let storage = MockPolicyStorage::new();
        let draft = create_test_draft("d1", "Policy", Some("Desc"), None, PolicyLayer::Project);
        storage.create_draft(draft).await.unwrap();

        let result = storage.approve_draft("d1", "reviewer").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cannot approve"));
    }

    #[tokio::test]
    async fn test_reject_draft() {
        let storage = MockPolicyStorage::new();
        let draft = create_test_draft("d1", "Policy", Some("Desc"), None, PolicyLayer::Project);
        storage.create_draft(draft).await.unwrap();
        storage.submit_draft("d1").await.unwrap();

        let result = storage
            .reject_draft("d1", "reviewer", "Does not meet standards")
            .await;
        assert!(result.is_ok());

        let rejected = storage.get_draft("d1").await.unwrap().unwrap();
        assert_eq!(rejected.status, DraftStatus::Rejected);
        assert_eq!(
            rejected.rejection_reason.as_deref(),
            Some("Does not meet standards")
        );
    }

    #[tokio::test]
    async fn test_cannot_reject_pending_draft() {
        let storage = MockPolicyStorage::new();
        let draft = create_test_draft("d1", "Policy", Some("Desc"), None, PolicyLayer::Project);
        storage.create_draft(draft).await.unwrap();

        let result = storage.reject_draft("d1", "reviewer", "reason").await;
        assert!(result.is_err());
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;

    #[tokio::test]
    async fn test_get_policy_after_approval() {
        let storage = MockPolicyStorage::new();
        let draft = create_test_draft(
            "d1",
            "Test Policy",
            Some("Desc"),
            None,
            PolicyLayer::Project,
        );
        storage.create_draft(draft).await.unwrap();
        storage.submit_draft("d1").await.unwrap();
        let policy = storage.approve_draft("d1", "reviewer").await.unwrap();

        let retrieved = storage.get_policy(&policy.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "Test Policy");
    }

    #[tokio::test]
    async fn test_list_policies_by_layer() {
        let storage = MockPolicyStorage::new();

        let d1 = create_test_draft("d1", "Tenant Policy", Some("D"), None, PolicyLayer::Tenant);
        storage.create_draft(d1).await.unwrap();
        storage.submit_draft("d1").await.unwrap();
        storage.approve_draft("d1", "r").await.unwrap();

        let d2 = create_test_draft(
            "d2",
            "Project Policy",
            Some("D"),
            None,
            PolicyLayer::Project,
        );
        storage.create_draft(d2).await.unwrap();
        storage.submit_draft("d2").await.unwrap();
        storage.approve_draft("d2", "r").await.unwrap();

        let company_policies = storage
            .list_policies("test-tenant", Some(PolicyLayer::Tenant), None)
            .await
            .unwrap();
        assert_eq!(company_policies.len(), 1);
        assert_eq!(company_policies[0].layer, PolicyLayer::Tenant);
    }

    #[tokio::test]
    async fn test_list_policies_by_mode() {
        let storage = MockPolicyStorage::new();

        let mut d1 = create_test_draft(
            "d1",
            "Mandatory Policy",
            Some("D"),
            None,
            PolicyLayer::Project,
        );
        d1.mode = PolicyMode::Mandatory;
        storage.create_draft(d1).await.unwrap();
        storage.submit_draft("d1").await.unwrap();
        storage.approve_draft("d1", "r").await.unwrap();

        let mandatory = storage
            .list_policies("test-tenant", None, Some(PolicyMode::Mandatory))
            .await
            .unwrap();
        assert_eq!(mandatory.len(), 1);
    }
}

#[cfg(test)]
mod template_tests {
    use super::*;

    #[tokio::test]
    async fn test_get_template() {
        let storage = MockPolicyStorage::new();
        let template = storage.get_template("security-baseline").await.unwrap();
        assert!(template.is_some());

        let t = template.unwrap();
        assert_eq!(t.name, "Security Baseline");
        assert!(!t.rules.is_empty());
    }

    #[tokio::test]
    async fn test_get_nonexistent_template() {
        let storage = MockPolicyStorage::new();
        let template = storage.get_template("nonexistent").await.unwrap();
        assert!(template.is_none());
    }

    #[tokio::test]
    async fn test_list_templates() {
        let storage = MockPolicyStorage::new();
        let templates = storage.list_templates().await.unwrap();
        assert_eq!(templates.len(), 3);

        let names: Vec<_> = templates.iter().map(|t| t.id.as_str()).collect();
        assert!(names.contains(&"security-baseline"));
        assert!(names.contains(&"code-style"));
        assert!(names.contains(&"dependency-audit"));
    }

    #[tokio::test]
    async fn test_template_default_values() {
        let storage = MockPolicyStorage::new();
        let template = storage
            .get_template("security-baseline")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(template.default_layer, PolicyLayer::Tenant);
        assert_eq!(template.default_mode, PolicyMode::Mandatory);
    }
}

#[cfg(test)]
mod validation_tests {
    use super::*;

    #[test]
    fn test_validate_valid_policy() {
        let storage = MockPolicyStorage::new();
        let rules = vec![PolicyRule {
            id: "r1".to_string(),
            rule_type: "must_exist".to_string(),
            target: RuleTarget::File,
            pattern: "README.md".to_string(),
            severity: Severity::Warn,
            message: "README required".to_string(),
        }];
        let policy = create_test_policy("p1", "Test", PolicyLayer::Project, rules);

        let result = storage.validate_policy(&policy);
        assert!(result.valid);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_validate_policy_empty_name() {
        let storage = MockPolicyStorage::new();
        let mut policy = create_test_policy("p1", "Test", PolicyLayer::Project, Vec::new());
        policy.name = String::new();

        let result = storage.validate_policy(&policy);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.code == "E001"));
    }

    #[test]
    fn test_validate_policy_no_rules_warning() {
        let storage = MockPolicyStorage::new();
        let policy = create_test_policy("p1", "Test", PolicyLayer::Project, Vec::new());

        let result = storage.validate_policy(&policy);
        assert!(result.valid);
        assert!(result.warnings.iter().any(|w| w.code == "W001"));
    }

    #[test]
    fn test_validate_policy_empty_pattern() {
        let storage = MockPolicyStorage::new();
        let rules = vec![PolicyRule {
            id: "r1".to_string(),
            rule_type: "must_exist".to_string(),
            target: RuleTarget::File,
            pattern: String::new(),
            severity: Severity::Warn,
            message: "Test".to_string(),
        }];
        let policy = create_test_policy("p1", "Test", PolicyLayer::Project, rules);

        let result = storage.validate_policy(&policy);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.code == "E002"));
    }

    #[test]
    fn test_validate_policy_invalid_cedar() {
        let storage = MockPolicyStorage::new();
        let mut policy = create_test_policy("p1", "Test", PolicyLayer::Project, Vec::new());
        policy.cedar_policy = Some("invalid cedar syntax".to_string());

        let result = storage.validate_policy(&policy);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.code == "E003"));
    }

    #[test]
    fn test_validate_draft() {
        let storage = MockPolicyStorage::new();
        let draft = create_test_draft("d1", "Test", Some("Desc"), None, PolicyLayer::Project);

        let result = storage.validate_draft(&draft);
        assert!(result.valid);
    }

    #[test]
    fn test_validate_draft_no_description_or_template() {
        let storage = MockPolicyStorage::new();
        let draft = create_test_draft("d1", "Test", None, None, PolicyLayer::Project);

        let result = storage.validate_draft(&draft);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.code == "E004"));
    }
}

#[cfg(test)]
mod simulation_tests {
    use super::*;

    async fn setup_policy_with_rules(storage: &MockPolicyStorage) -> String {
        let rules = vec![
            PolicyRule {
                id: "block-lodash".to_string(),
                rule_type: "must_not_use".to_string(),
                target: RuleTarget::Dependency,
                pattern: "lodash@3".to_string(),
                severity: Severity::Block,
                message: "Lodash 3.x has critical vulnerabilities".to_string(),
            },
            PolicyRule {
                id: "warn-console".to_string(),
                rule_type: "must_not_match".to_string(),
                target: RuleTarget::Code,
                pattern: "console.log".to_string(),
                severity: Severity::Warn,
                message: "Avoid console.log".to_string(),
            },
        ];

        let mut draft =
            create_test_draft("d1", "Test Policy", Some("D"), None, PolicyLayer::Project);
        draft.rules = rules;
        storage.create_draft(draft).await.unwrap();
        storage.submit_draft("d1").await.unwrap();
        let policy = storage.approve_draft("d1", "r").await.unwrap();
        policy.id
    }

    #[tokio::test]
    async fn test_simulate_allow() {
        let storage = MockPolicyStorage::new();
        let policy_id = setup_policy_with_rules(&storage).await;

        let scenario = SimulationScenario {
            scenario_type: "dependency-add".to_string(),
            input: "react@18.0.0".to_string(),
            context: HashMap::new(),
        };

        let result = storage
            .simulate_policy(&policy_id, &scenario)
            .await
            .unwrap();
        assert_eq!(result.decision, "allow");
        assert!(result.violations.is_empty());
    }

    #[tokio::test]
    async fn test_simulate_blocked() {
        let storage = MockPolicyStorage::new();
        let policy_id = setup_policy_with_rules(&storage).await;

        let scenario = SimulationScenario {
            scenario_type: "dependency-add".to_string(),
            input: "lodash@3.10.1".to_string(),
            context: HashMap::new(),
        };

        let result = storage
            .simulate_policy(&policy_id, &scenario)
            .await
            .unwrap();
        assert_eq!(result.decision, "blocked");
        assert!(!result.violations.is_empty());
        assert!(
            result
                .violations
                .iter()
                .any(|v| v.severity == Severity::Block)
        );
    }

    #[tokio::test]
    async fn test_simulate_warn() {
        let storage = MockPolicyStorage::new();
        let policy_id = setup_policy_with_rules(&storage).await;

        let scenario = SimulationScenario {
            scenario_type: "code-change".to_string(),
            input: "console.log('debug')".to_string(),
            context: HashMap::new(),
        };

        let result = storage
            .simulate_policy(&policy_id, &scenario)
            .await
            .unwrap();
        assert_eq!(result.decision, "warn");
        assert!(!result.violations.is_empty());
    }

    #[tokio::test]
    async fn test_simulate_nonexistent_policy() {
        let storage = MockPolicyStorage::new();

        let scenario = SimulationScenario {
            scenario_type: "dependency-add".to_string(),
            input: "test".to_string(),
            context: HashMap::new(),
        };

        let result = storage.simulate_policy("nonexistent", &scenario).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_simulate_multiple_scenarios() {
        let storage = MockPolicyStorage::new();
        let policy_id = setup_policy_with_rules(&storage).await;

        let scenarios = vec![
            ("dependency-add", "safe-package@1.0.0", "allow"),
            ("dependency-add", "lodash@3.5.0", "blocked"),
            ("code-change", "const x = 1;", "allow"),
            ("code-change", "console.log(x);", "warn"),
            ("file-create", "test.js", "allow"),
        ];

        for (scenario_type, input, expected_decision) in scenarios {
            let scenario = SimulationScenario {
                scenario_type: scenario_type.to_string(),
                input: input.to_string(),
                context: HashMap::new(),
            };

            let result = storage
                .simulate_policy(&policy_id, &scenario)
                .await
                .unwrap();
            assert_eq!(
                result.decision, expected_decision,
                "Failed for scenario: {} with input: {}",
                scenario_type, input
            );
        }
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[tokio::test]
    async fn test_full_policy_lifecycle() {
        let storage = MockPolicyStorage::new();

        let template = storage
            .get_template("security-baseline")
            .await
            .unwrap()
            .unwrap();

        let mut draft = create_test_draft(
            "draft-security",
            "My Security Policy",
            Some("Custom security policy based on baseline"),
            Some("security-baseline"),
            template.default_layer,
        );
        draft.mode = template.default_mode;
        draft.rules = template.rules.clone();

        storage.create_draft(draft).await.unwrap();

        let validation =
            storage.validate_draft(&storage.get_draft("draft-security").await.unwrap().unwrap());
        assert!(validation.valid);

        storage.submit_draft("draft-security").await.unwrap();

        let policy = storage
            .approve_draft("draft-security", "security-admin")
            .await
            .unwrap();

        assert_eq!(policy.name, "My Security Policy");
        assert_eq!(policy.layer, PolicyLayer::Tenant);
        assert!(!policy.rules.is_empty());

        let scenario = SimulationScenario {
            scenario_type: "dependency-add".to_string(),
            input: "cve:critical-2024-1234".to_string(),
            context: HashMap::new(),
        };
        let sim_result = storage
            .simulate_policy(&policy.id, &scenario)
            .await
            .unwrap();
        assert_eq!(sim_result.decision, "blocked");
    }

    #[tokio::test]
    async fn test_policy_inheritance_simulation() {
        let storage = MockPolicyStorage::new();

        let company_draft = create_test_draft(
            "d-tenant",
            "Tenant Security",
            Some("Tenant-wide security"),
            None,
            PolicyLayer::Tenant,
        );
        storage.create_draft(company_draft).await.unwrap();
        storage.submit_draft("d-tenant").await.unwrap();
        storage.approve_draft("d-tenant", "admin").await.unwrap();

        let project_draft = create_test_draft(
            "d-project",
            "Project Rules",
            Some("Project-specific rules"),
            None,
            PolicyLayer::Project,
        );
        storage.create_draft(project_draft).await.unwrap();
        storage.submit_draft("d-project").await.unwrap();
        storage.approve_draft("d-project", "lead").await.unwrap();

        let all_policies = storage
            .list_policies("test-tenant", None, None)
            .await
            .unwrap();
        assert_eq!(all_policies.len(), 2);

        let company_only = storage
            .list_policies("test-tenant", Some(PolicyLayer::Tenant), None)
            .await
            .unwrap();
        assert_eq!(company_only.len(), 1);

        let project_only = storage
            .list_policies("test-tenant", Some(PolicyLayer::Project), None)
            .await
            .unwrap();
        assert_eq!(project_only.len(), 1);
    }
}
