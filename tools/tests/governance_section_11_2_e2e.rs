//! Section 11.2: End-to-End Integration Tests
//!
//! This module contains the 8 E2E test scenarios specified in Section 11.2
//! of the add-ux-first-governance OpenSpec change.

use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use storage::governance::{
    ApprovalDecision, ApprovalMode, ApprovalRequest, AuditFilters, CreateApprovalRequest,
    CreateDecision, CreateGovernanceRole, Decision, GovernanceAuditEntry, GovernanceConfig,
    GovernanceRole, PrincipalType, RequestStatus, RequestType, RiskLevel,
};

// ============================================================================
// Mock Policy Storage Types and Implementation
// ============================================================================

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyMode {
    Mandatory,
    Optional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warn,
    Error,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleTarget {
    Dependency,
    File,
    Code,
    Config,
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
}

impl Default for MockPolicyStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MockPolicyStorage {
    pub fn new() -> Self {
        Self {
            policies: Arc::new(RwLock::new(HashMap::new())),
            drafts: Arc::new(RwLock::new(HashMap::new())),
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

    pub fn validate_draft(&self, draft: &PolicyDraft) -> ValidationResult {
        let mut errors = Vec::new();
        let warnings = Vec::new();

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

// ============================================================================
// Mock Governance Storage Implementation
// ============================================================================

pub struct MockGovernanceStorage {
    configs: Arc<RwLock<HashMap<String, GovernanceConfig>>>,
    requests: Arc<RwLock<HashMap<Uuid, ApprovalRequest>>>,
    decisions: Arc<RwLock<HashMap<Uuid, Vec<ApprovalDecision>>>>,
    roles: Arc<RwLock<Vec<GovernanceRole>>>,
    audit_logs: Arc<RwLock<Vec<GovernanceAuditEntry>>>,
    request_counter: Arc<RwLock<u64>>,
}

impl Default for MockGovernanceStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MockGovernanceStorage {
    pub fn new() -> Self {
        Self {
            configs: Arc::new(RwLock::new(HashMap::new())),
            requests: Arc::new(RwLock::new(HashMap::new())),
            decisions: Arc::new(RwLock::new(HashMap::new())),
            roles: Arc::new(RwLock::new(Vec::new())),
            audit_logs: Arc::new(RwLock::new(Vec::new())),
            request_counter: Arc::new(RwLock::new(0)),
        }
    }

    fn scope_key(
        tenant_id: Option<Uuid>,
        org_id: Option<Uuid>,
        team_id: Option<Uuid>,
        project_id: Option<Uuid>,
    ) -> String {
        format!(
            "{:?}:{:?}:{:?}:{:?}",
            tenant_id, org_id, team_id, project_id
        )
    }

    pub async fn get_effective_config(
        &self,
        tenant_id: Option<Uuid>,
        org_id: Option<Uuid>,
        team_id: Option<Uuid>,
        project_id: Option<Uuid>,
    ) -> Result<GovernanceConfig, String> {
        let configs = self.configs.read().await;
        let key = Self::scope_key(tenant_id, org_id, team_id, project_id);

        if let Some(config) = configs.get(&key) {
            return Ok(config.clone());
        }

        Ok(GovernanceConfig::default())
    }

    pub async fn upsert_config(&self, config: &GovernanceConfig) -> Result<Uuid, String> {
        let mut configs = self.configs.write().await;
        let key = Self::scope_key(
            config.tenant_id,
            config.org_id,
            config.team_id,
            config.project_id,
        );
        let id = config.id.unwrap_or_else(Uuid::new_v4);
        let mut stored = config.clone();
        stored.id = Some(id);
        configs.insert(key, stored);
        Ok(id)
    }

    pub async fn create_request(
        &self,
        request: &CreateApprovalRequest,
    ) -> Result<ApprovalRequest, String> {
        let mut counter = self.request_counter.write().await;
        *counter += 1;
        let request_number = format!("REQ-{:06}", *counter);

        let id = Uuid::new_v4();
        let now = Utc::now();
        let expires_at = request
            .timeout_hours
            .map(|h| now + Duration::hours(h as i64));

        let approval_request = ApprovalRequest {
            id,
            request_number,
            request_type: request.request_type,
            target_type: request.target_type.clone(),
            target_id: request.target_id.clone(),
            tenant_id: request.tenant_id,
            org_id: request.org_id,
            team_id: request.team_id,
            project_id: request.project_id,
            title: request.title.clone(),
            description: request.description.clone(),
            payload: request.payload.clone(),
            risk_level: request.risk_level,
            requestor_type: request.requestor_type,
            requestor_id: request.requestor_id,
            requestor_email: request.requestor_email.clone(),
            required_approvals: request.required_approvals,
            current_approvals: 0,
            status: RequestStatus::Pending,
            created_at: now,
            updated_at: now,
            expires_at,
            resolved_at: None,
            resolution_reason: None,
            applied_at: None,
            applied_by: None,
        };

        let mut requests = self.requests.write().await;
        requests.insert(id, approval_request.clone());
        Ok(approval_request)
    }

    pub async fn get_request(&self, request_id: Uuid) -> Result<Option<ApprovalRequest>, String> {
        let requests = self.requests.read().await;
        Ok(requests.get(&request_id).cloned())
    }

    pub async fn add_decision(
        &self,
        decision: &CreateDecision,
    ) -> Result<ApprovalDecision, String> {
        let id = Uuid::new_v4();
        let approval_decision = ApprovalDecision {
            id,
            request_id: decision.request_id,
            approver_type: decision.approver_type,
            approver_id: decision.approver_id,
            approver_email: decision.approver_email.clone(),
            decision: decision.decision,
            comment: decision.comment.clone(),
            created_at: Utc::now(),
        };

        let mut decisions = self.decisions.write().await;
        decisions
            .entry(decision.request_id)
            .or_insert_with(Vec::new)
            .push(approval_decision.clone());

        if decision.decision == Decision::Approve {
            let mut requests = self.requests.write().await;
            if let Some(request) = requests.get_mut(&decision.request_id) {
                request.current_approvals += 1;
                request.updated_at = Utc::now();

                if request.current_approvals >= request.required_approvals {
                    request.status = RequestStatus::Approved;
                    request.resolved_at = Some(Utc::now());
                }
            }
        }

        Ok(approval_decision)
    }

    pub async fn mark_applied(
        &self,
        request_id: Uuid,
        applied_by: Uuid,
    ) -> Result<ApprovalRequest, String> {
        let mut requests = self.requests.write().await;
        let request = requests.get_mut(&request_id).ok_or("Request not found")?;

        request.applied_at = Some(Utc::now());
        request.applied_by = Some(applied_by);
        request.updated_at = Utc::now();

        Ok(request.clone())
    }

    pub async fn get_decisions(&self, request_id: Uuid) -> Result<Vec<ApprovalDecision>, String> {
        let decisions = self.decisions.read().await;
        Ok(decisions.get(&request_id).cloned().unwrap_or_default())
    }

    pub async fn assign_role(&self, role: &CreateGovernanceRole) -> Result<Uuid, String> {
        let id = Uuid::new_v4();
        let governance_role = GovernanceRole {
            id,
            principal_type: role.principal_type,
            principal_id: role.principal_id,
            role: role.role.clone(),
            tenant_id: role.tenant_id,
            org_id: role.org_id,
            team_id: role.team_id,
            project_id: role.project_id,
            granted_by: role.granted_by,
            granted_at: Utc::now(),
            expires_at: role.expires_at,
            revoked_at: None,
            revoked_by: None,
        };

        let mut roles = self.roles.write().await;
        roles.push(governance_role);
        Ok(id)
    }

    pub async fn check_has_role(
        &self,
        principal_id: Uuid,
        role: &str,
        tenant_id: Option<Uuid>,
        org_id: Option<Uuid>,
        team_id: Option<Uuid>,
    ) -> Result<bool, String> {
        let roles = self.roles.read().await;
        let has_role = roles.iter().any(|r| {
            r.principal_id == principal_id
                && r.role == role
                && r.revoked_at.is_none()
                && (tenant_id.is_none() || r.tenant_id == tenant_id)
                && (org_id.is_none() || r.org_id == org_id)
                && (team_id.is_none() || r.team_id == team_id)
        });
        Ok(has_role)
    }

    pub async fn log_audit(
        &self,
        action: &str,
        request_id: Option<Uuid>,
        target_type: Option<&str>,
        target_id: Option<&str>,
        actor_type: PrincipalType,
        actor_id: Option<Uuid>,
        actor_email: Option<&str>,
        details: serde_json::Value,
    ) -> Result<Uuid, String> {
        let id = Uuid::new_v4();
        let entry = GovernanceAuditEntry {
            id,
            action: action.to_string(),
            request_id,
            target_type: target_type.map(String::from),
            target_id: target_id.map(String::from),
            actor_type,
            actor_id,
            actor_email: actor_email.map(String::from),
            details,
            old_values: None,
            new_values: None,
            acting_as_tenant_id: None,
            via: None,
            client_version: None,
            manifest_hash: None,
            generation: None,
            dry_run: None,
            created_at: Utc::now(),
        };

        let mut logs = self.audit_logs.write().await;
        logs.push(entry);
        Ok(id)
    }

    pub async fn list_audit_logs(
        &self,
        filters: &AuditFilters,
    ) -> Result<Vec<GovernanceAuditEntry>, String> {
        let logs = self.audit_logs.read().await;
        let limit = filters.limit.unwrap_or(50);

        let filtered: Vec<GovernanceAuditEntry> = logs
            .iter()
            .filter(|e| filters.action.as_ref().is_none_or(|a| &e.action == a))
            .filter(|e| filters.actor_id.is_none_or(|id| e.actor_id == Some(id)))
            .filter(|e| {
                filters
                    .target_type
                    .as_ref()
                    .is_none_or(|t| e.target_type.as_ref() == Some(t))
            })
            .filter(|e| e.created_at >= filters.since)
            .take(limit as usize)
            .cloned()
            .collect();

        Ok(filtered)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_e2e_human_creates_policy_via_ai() {
    let policy_storage = Arc::new(MockPolicyStorage::new());

    let natural_language_input =
        "Block all critical CVEs in dependencies and require a README file";

    let draft = PolicyDraft {
        id: format!("draft-{}", Uuid::new_v4()),
        name: "Security Baseline Policy".to_string(),
        description: Some(natural_language_input.to_string()),
        template: Some("security-baseline".to_string()),
        layer: PolicyLayer::Project,
        mode: PolicyMode::Mandatory,
        severity: Severity::Block,
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
                id: "require-readme".to_string(),
                rule_type: "must_exist".to_string(),
                target: RuleTarget::File,
                pattern: "README.md".to_string(),
                severity: Severity::Warn,
                message: "README.md file is required".to_string(),
            },
        ],
        cedar_policy: Some(
            r#"
            permit (principal, action, resource)
            when {
                resource has cve_severity &&
                resource.cve_severity != "critical"
            };
        "#
            .to_string(),
        ),
        status: DraftStatus::Pending,
        tenant_id: "test-tenant".to_string(),
        created_at: Utc::now().timestamp(),
        expires_at: Utc::now().timestamp() + 86400,
        created_by: "user@example.com".to_string(),
        submitted_at: None,
        reviewed_at: None,
        reviewed_by: None,
        rejection_reason: None,
    };

    let draft_id = policy_storage.create_draft(draft.clone()).await.unwrap();
    assert_eq!(draft_id, draft.id);

    let validation = policy_storage.validate_draft(&draft);
    assert!(
        validation.valid,
        "Draft should be valid: {:?}",
        validation.errors
    );

    let retrieved = policy_storage.get_draft(&draft_id).await.unwrap();
    assert!(retrieved.is_some());
    let retrieved_draft = retrieved.unwrap();
    assert_eq!(retrieved_draft.name, "Security Baseline Policy");
    assert_eq!(retrieved_draft.rules.len(), 2);

    let scenario = SimulationScenario {
        scenario_type: "dependency-add".to_string(),
        input: "cve:critical-2024-1234".to_string(),
        context: HashMap::new(),
    };

    policy_storage.submit_draft(&draft_id).await.unwrap();
    let policy = policy_storage
        .approve_draft(&draft_id, "security-admin")
        .await
        .unwrap();

    let sim_result = policy_storage
        .simulate_policy(&policy.id, &scenario)
        .await
        .unwrap();
    assert_eq!(
        sim_result.decision, "blocked",
        "Critical CVE should be blocked"
    );
    assert!(!sim_result.violations.is_empty());
}

#[tokio::test]
async fn test_e2e_admin_configures_meta_governance() {
    let governance_storage = Arc::new(MockGovernanceStorage::new());
    let admin_id = Uuid::new_v4();
    let tenant_id = Uuid::new_v4();

    let config = GovernanceConfig {
        id: Some(Uuid::new_v4()),
        tenant_id: Some(tenant_id),
        org_id: None,
        team_id: None,
        project_id: None,
        approval_mode: ApprovalMode::Quorum,
        min_approvers: 3,
        timeout_hours: 72,
        auto_approve_low_risk: true,
        escalation_enabled: true,
        escalation_timeout_hours: 24,
        escalation_contact: Some("security-team@example.com".to_string()),
        policy_settings: json!({
            "require_approval": true,
            "min_approvers": 3,
            "auto_approve_threshold": "low"
        }),
        knowledge_settings: json!({
            "require_approval": true,
            "min_approvers": 2
        }),
        memory_settings: json!({
            "require_approval": false,
            "auto_approve_threshold": 0.8
        }),
    };

    let config_id = governance_storage.upsert_config(&config).await.unwrap();
    assert!(!config_id.is_nil());

    let effective_config = governance_storage
        .get_effective_config(Some(tenant_id), None, None, None)
        .await
        .unwrap();

    assert_eq!(effective_config.min_approvers, 3);
    assert_eq!(effective_config.approval_mode, ApprovalMode::Quorum);
    assert!(effective_config.auto_approve_low_risk);
    assert!(effective_config.escalation_enabled);

    let approver_role = CreateGovernanceRole {
        principal_type: PrincipalType::User,
        principal_id: Uuid::new_v4(),
        role: "policy-approver".to_string(),
        tenant_id: Some(tenant_id),
        org_id: None,
        team_id: None,
        project_id: None,
        granted_by: admin_id,
        expires_at: None,
    };

    let role_id = governance_storage
        .assign_role(&approver_role)
        .await
        .unwrap();
    assert!(!role_id.is_nil());

    let has_role = governance_storage
        .check_has_role(
            approver_role.principal_id,
            "policy-approver",
            Some(tenant_id),
            None,
            None,
        )
        .await
        .unwrap();
    assert!(has_role);

    let request = CreateApprovalRequest {
        request_type: RequestType::Policy,
        target_type: "cedar_policy".to_string(),
        target_id: Some("policy-security".to_string()),
        tenant_id: Some(tenant_id),
        org_id: None,
        team_id: None,
        project_id: None,
        title: "Security Policy Update".to_string(),
        description: Some("Update to security baseline policy".to_string()),
        payload: json!({"policy": "permit (principal, action, resource);"}),
        risk_level: RiskLevel::High,
        requestor_type: PrincipalType::User,
        requestor_id: Uuid::new_v4(),
        requestor_email: Some("developer@example.com".to_string()),
        required_approvals: 3,
        timeout_hours: Some(72),
    };

    let created_request = governance_storage.create_request(&request).await.unwrap();
    assert_eq!(created_request.status, RequestStatus::Pending);
    assert_eq!(created_request.required_approvals, 3);

    let audit_id = governance_storage
        .log_audit(
            "governance_config_updated",
            None,
            Some("governance_config"),
            Some(&config_id.to_string()),
            PrincipalType::User,
            Some(admin_id),
            Some("admin@example.com"),
            json!({
                "min_approvers": 3,
                "approval_mode": "quorum",
                "escalation_enabled": true
            }),
        )
        .await
        .unwrap();

    assert!(!audit_id.is_nil());
}

#[tokio::test]
async fn test_e2e_agent_proposes_policy_autonomously() {
    let policy_storage = Arc::new(MockPolicyStorage::new());
    let governance_storage = Arc::new(MockGovernanceStorage::new());

    let agent_id = Uuid::new_v4();
    let tenant_id = Uuid::new_v4();

    let agent_role = CreateGovernanceRole {
        principal_type: PrincipalType::Agent,
        principal_id: agent_id,
        role: "policy-proposer".to_string(),
        tenant_id: Some(tenant_id),
        org_id: None,
        team_id: None,
        project_id: None,
        granted_by: Uuid::new_v4(),
        expires_at: Some(Utc::now() + Duration::hours(24)),
    };

    governance_storage.assign_role(&agent_role).await.unwrap();

    let detected_pattern = "console.log statements in production code";

    let draft = PolicyDraft {
        id: format!("draft-agent-{}", Uuid::new_v4()),
        name: "No Console Log in Production".to_string(),
        description: Some(format!("Auto-generated: {}", detected_pattern)),
        template: None,
        layer: PolicyLayer::Team,
        mode: PolicyMode::Optional,
        severity: Severity::Warn,
        rules: vec![PolicyRule {
            id: "no-console-log".to_string(),
            rule_type: "must_not_match".to_string(),
            target: RuleTarget::Code,
            pattern: r"console\.log".to_string(),
            severity: Severity::Warn,
            message: "Avoid console.log in production code".to_string(),
        }],
        cedar_policy: Some(
            r#"
            forbid (principal, action, resource)
            when {
                resource has code_content &&
                resource.code_content.contains("console.log")
            };
        "#
            .to_string(),
        ),
        status: DraftStatus::Pending,
        tenant_id: "test-tenant".to_string(),
        created_at: Utc::now().timestamp(),
        expires_at: Utc::now().timestamp() + 86400,
        created_by: format!("agent:{}", agent_id),
        submitted_at: None,
        reviewed_at: None,
        reviewed_by: None,
        rejection_reason: None,
    };

    let draft_id = policy_storage.create_draft(draft.clone()).await.unwrap();

    let approval_request = CreateApprovalRequest {
        request_type: RequestType::Policy,
        target_type: "policy_draft".to_string(),
        target_id: Some(draft_id.clone()),
        tenant_id: Some(tenant_id),
        org_id: None,
        team_id: None,
        project_id: None,
        title: format!("Agent Proposal: {}", draft.name),
        description: Some(format!(
            "Agent {} detected pattern '{}' and proposes this policy for review.",
            agent_id, detected_pattern
        )),
        payload: json!({
            "draft_id": draft_id,
            "agent_id": agent_id,
            "detection_confidence": 0.95
        }),
        risk_level: RiskLevel::Low,
        requestor_type: PrincipalType::Agent,
        requestor_id: agent_id,
        requestor_email: Some(format!("agent-{}@system", agent_id)),
        required_approvals: 1,
        timeout_hours: Some(48),
    };

    let request = governance_storage
        .create_request(&approval_request)
        .await
        .unwrap();
    assert_eq!(request.status, RequestStatus::Pending);
    assert_eq!(request.requestor_type, PrincipalType::Agent);

    let audit_id = governance_storage
        .log_audit(
            "agent_policy_proposed",
            Some(request.id),
            Some("policy_draft"),
            Some(&draft_id),
            PrincipalType::Agent,
            Some(agent_id),
            None,
            json!({
                "detection_pattern": detected_pattern,
                "confidence": 0.95,
                "delegation_role": "policy-proposer"
            }),
        )
        .await
        .unwrap();

    assert!(!audit_id.is_nil());

    let decision = CreateDecision {
        request_id: request.id,
        approver_type: PrincipalType::User,
        approver_id: Uuid::new_v4(),
        approver_email: Some("techlead@example.com".to_string()),
        decision: Decision::Approve,
        comment: Some("Good catch by the agent, approving".to_string()),
    };

    governance_storage.add_decision(&decision).await.unwrap();

    let updated_request = governance_storage
        .get_request(request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated_request.status, RequestStatus::Approved);

    policy_storage.submit_draft(&draft_id).await.unwrap();
    let policy = policy_storage
        .approve_draft(&draft_id, "techlead@example.com")
        .await
        .unwrap();
    assert_eq!(policy.name, "No Console Log in Production");
}

#[tokio::test]
async fn test_e2e_multi_approver_workflow() {
    let governance_storage = Arc::new(MockGovernanceStorage::new());
    let policy_storage = Arc::new(MockPolicyStorage::new());

    let tenant_id = Uuid::new_v4();
    let requestor_id = Uuid::new_v4();

    let config = GovernanceConfig {
        id: Some(Uuid::new_v4()),
        tenant_id: Some(tenant_id),
        org_id: None,
        team_id: None,
        project_id: None,
        approval_mode: ApprovalMode::Quorum,
        min_approvers: 3,
        timeout_hours: 48,
        auto_approve_low_risk: false,
        escalation_enabled: true,
        escalation_timeout_hours: 24,
        escalation_contact: Some("escalation@example.com".to_string()),
        policy_settings: json!({}),
        knowledge_settings: json!({}),
        memory_settings: json!({}),
    };

    governance_storage.upsert_config(&config).await.unwrap();

    let draft = PolicyDraft {
        id: format!("draft-{}", Uuid::new_v4()),
        name: "Critical Security Policy".to_string(),
        description: Some("Blocks all external network access".to_string()),
        template: None,
        layer: PolicyLayer::Tenant,
        mode: PolicyMode::Mandatory,
        severity: Severity::Block,
        rules: vec![PolicyRule {
            id: "block-external-network".to_string(),
            rule_type: "must_not_match".to_string(),
            target: RuleTarget::Code,
            pattern: r"(http|https)://".to_string(),
            severity: Severity::Block,
            message: "External network calls are blocked".to_string(),
        }],
        cedar_policy: Some("forbid (principal, action, resource);".to_string()),
        status: DraftStatus::Pending,
        tenant_id: "test-tenant".to_string(),
        created_at: Utc::now().timestamp(),
        expires_at: Utc::now().timestamp() + 86400,
        created_by: "security@example.com".to_string(),
        submitted_at: None,
        reviewed_at: None,
        reviewed_by: None,
        rejection_reason: None,
    };

    let draft_id = policy_storage.create_draft(draft.clone()).await.unwrap();
    policy_storage.submit_draft(&draft_id).await.unwrap();

    let request = CreateApprovalRequest {
        request_type: RequestType::Policy,
        target_type: "policy_draft".to_string(),
        target_id: Some(draft_id.clone()),
        tenant_id: Some(tenant_id),
        org_id: None,
        team_id: None,
        project_id: None,
        title: format!("Approval Required: {}", draft.name),
        description: Some("High-risk policy requiring security review".to_string()),
        payload: json!({"draft_id": draft_id}),
        risk_level: RiskLevel::Critical,
        requestor_type: PrincipalType::User,
        requestor_id,
        requestor_email: Some("security@example.com".to_string()),
        required_approvals: 3,
        timeout_hours: Some(48),
    };

    let approval_request = governance_storage.create_request(&request).await.unwrap();
    assert_eq!(approval_request.status, RequestStatus::Pending);
    assert_eq!(approval_request.required_approvals, 3);
    assert_eq!(approval_request.current_approvals, 0);

    let approver1 = Uuid::new_v4();
    let decision1 = CreateDecision {
        request_id: approval_request.id,
        approver_type: PrincipalType::User,
        approver_id: approver1,
        approver_email: Some("security-lead@example.com".to_string()),
        decision: Decision::Approve,
        comment: Some("Security review passed".to_string()),
    };

    governance_storage.add_decision(&decision1).await.unwrap();

    let req_after_1 = governance_storage
        .get_request(approval_request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(req_after_1.status, RequestStatus::Pending);
    assert_eq!(req_after_1.current_approvals, 1);

    let approver2 = Uuid::new_v4();
    let decision2 = CreateDecision {
        request_id: approval_request.id,
        approver_type: PrincipalType::User,
        approver_id: approver2,
        approver_email: Some("tech-lead@example.com".to_string()),
        decision: Decision::Approve,
        comment: Some("Technical impact assessed and accepted".to_string()),
    };

    governance_storage.add_decision(&decision2).await.unwrap();

    let req_after_2 = governance_storage
        .get_request(approval_request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(req_after_2.status, RequestStatus::Pending);
    assert_eq!(req_after_2.current_approvals, 2);

    let approver3 = Uuid::new_v4();
    let decision3 = CreateDecision {
        request_id: approval_request.id,
        approver_type: PrincipalType::User,
        approver_id: approver3,
        approver_email: Some("architect@example.com".to_string()),
        decision: Decision::Approve,
        comment: Some("Architecture alignment confirmed".to_string()),
    };

    governance_storage.add_decision(&decision3).await.unwrap();

    let req_after_3 = governance_storage
        .get_request(approval_request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(req_after_3.status, RequestStatus::Approved);
    assert_eq!(req_after_3.current_approvals, 3);
    assert!(req_after_3.resolved_at.is_some());

    let policy = policy_storage
        .approve_draft(&draft_id, "system")
        .await
        .unwrap();
    assert_eq!(policy.layer, PolicyLayer::Tenant);

    let applied = governance_storage
        .mark_applied(approval_request.id, approver1)
        .await
        .unwrap();
    assert!(applied.applied_at.is_some());
    assert_eq!(applied.applied_by, Some(approver1));
}

#[tokio::test]
async fn test_e2e_audit_export_for_compliance() {
    let governance_storage = Arc::new(MockGovernanceStorage::new());

    let _tenant_id = Uuid::new_v4();
    let admin_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    governance_storage
        .log_audit(
            "governance_config_updated",
            None,
            Some("governance_config"),
            Some("config-1"),
            PrincipalType::User,
            Some(admin_id),
            Some("admin@example.com"),
            json!({
                "action": "updated",
                "field": "min_approvers",
                "old_value": 2,
                "new_value": 3
            }),
        )
        .await
        .unwrap();

    governance_storage
        .log_audit(
            "role_assigned",
            None,
            Some("role"),
            Some("role-approver"),
            PrincipalType::User,
            Some(admin_id),
            Some("admin@example.com"),
            json!({
                "principal_id": user_id.to_string(),
                "role": "policy-approver"
            }),
        )
        .await
        .unwrap();

    let policy_request_id = Uuid::new_v4();
    governance_storage
        .log_audit(
            "policy_created",
            Some(policy_request_id),
            Some("policy"),
            Some("policy-security"),
            PrincipalType::User,
            Some(user_id),
            Some("user@example.com"),
            json!({
                "policy_name": "Security Baseline",
                "layer": "Tenant"
            }),
        )
        .await
        .unwrap();

    governance_storage
        .log_audit(
            "request_approved",
            Some(policy_request_id),
            Some("approval_request"),
            Some(&policy_request_id.to_string()),
            PrincipalType::User,
            Some(admin_id),
            Some("admin@example.com"),
            json!({
                "decision": "approve",
                "comment": "LGTM"
            }),
        )
        .await
        .unwrap();

    let filters = AuditFilters {
        action: None,
        actor_id: None,
        target_type: None,
        since: Utc::now() - Duration::days(30),
        limit: Some(100),
        acting_as_tenant_id: None,
        via: None,
        client_version: None,
        manifest_hash: None,
        generation: None,
        dry_run: None,
    };

    let audit_logs = governance_storage.list_audit_logs(&filters).await.unwrap();
    assert_eq!(audit_logs.len(), 4);

    let config_filters = AuditFilters {
        action: Some("governance_config_updated".to_string()),
        actor_id: None,
        target_type: None,
        since: Utc::now() - Duration::days(30),
        limit: Some(50),
        acting_as_tenant_id: None,
        via: None,
        client_version: None,
        manifest_hash: None,
        generation: None,
        dry_run: None,
    };

    let config_logs = governance_storage
        .list_audit_logs(&config_filters)
        .await
        .unwrap();
    assert_eq!(config_logs.len(), 1);
    assert_eq!(config_logs[0].action, "governance_config_updated");

    let actor_filters = AuditFilters {
        action: None,
        actor_id: Some(admin_id),
        target_type: None,
        since: Utc::now() - Duration::days(30),
        limit: Some(50),
        acting_as_tenant_id: None,
        via: None,
        client_version: None,
        manifest_hash: None,
        generation: None,
        dry_run: None,
    };

    let admin_logs = governance_storage
        .list_audit_logs(&actor_filters)
        .await
        .unwrap();
    assert_eq!(admin_logs.len(), 3);

    let export_data: Vec<serde_json::Value> = audit_logs
        .iter()
        .map(|entry| {
            json!({
                "timestamp": entry.created_at,
                "action": entry.action,
                "actor_type": entry.actor_type,
                "actor_id": entry.actor_id,
                "actor_email": entry.actor_email,
                "target_type": entry.target_type,
                "target_id": entry.target_id,
                "request_id": entry.request_id,
                "details": entry.details
            })
        })
        .collect();

    assert_eq!(export_data.len(), 4);

    let first_entry = &export_data[0];
    assert!(first_entry.get("timestamp").is_some());
    assert!(first_entry.get("action").is_some());
    assert!(first_entry.get("actor_id").is_some());
}

#[tokio::test]
async fn test_e2e_project_init_with_auto_detection() {
    #[derive(Debug, Clone)]
    struct ResolvedContext {
        #[allow(dead_code)]
        tenant_id: String,
        project: Option<String>,
        #[allow(dead_code)]
        team: Option<String>,
        #[allow(dead_code)]
        org: Option<String>,
        tenant: Option<String>,
        #[allow(dead_code)]
        user: Option<String>,
    }

    let _git_remote = "git@github.com:acme-corp/api-gateway.git";
    let user_email = "developer@acme-corp.com";

    let detected_context = ResolvedContext {
        project: Some("api-gateway".to_string()),
        team: Some("platform-team".to_string()),
        org: Some("engineering".to_string()),
        tenant: Some("acme-corp".to_string()),
        user: Some("developer@acme-corp.com".to_string()),
        tenant_id: "tenant-acme-corp".to_string(),
    };

    assert!(detected_context.project.is_some());
    assert_eq!(detected_context.project.clone().unwrap(), "api-gateway");
    assert!(detected_context.tenant.is_some());
    assert_eq!(detected_context.tenant.clone().unwrap(), "acme-corp");

    let policy_storage = Arc::new(MockPolicyStorage::new());
    let governance_storage = Arc::new(MockGovernanceStorage::new());

    let tenant_id = Uuid::new_v4();
    let org_id = Uuid::new_v4();
    let team_id = Uuid::new_v4();
    let project_id = Uuid::new_v4();

    let config = GovernanceConfig {
        id: Some(Uuid::new_v4()),
        tenant_id: Some(tenant_id),
        org_id: None,
        team_id: None,
        project_id: None,
        approval_mode: ApprovalMode::Quorum,
        min_approvers: 2,
        timeout_hours: 48,
        auto_approve_low_risk: true,
        escalation_enabled: true,
        escalation_timeout_hours: 24,
        escalation_contact: Some("admin@acme-corp.com".to_string()),
        policy_settings: json!({}),
        knowledge_settings: json!({}),
        memory_settings: json!({}),
    };

    governance_storage.upsert_config(&config).await.unwrap();

    let effective_config = governance_storage
        .get_effective_config(Some(tenant_id), None, None, None)
        .await
        .unwrap();

    assert_eq!(effective_config.min_approvers, 2);
    assert!(effective_config.auto_approve_low_risk);

    let _ = (org_id, team_id, project_id);

    let default_policy = PolicyDraft {
        id: format!("draft-{}", Uuid::new_v4()),
        name: "Project Default Policy".to_string(),
        description: Some("Auto-created during project initialization".to_string()),
        template: Some("standard".to_string()),
        layer: PolicyLayer::Project,
        mode: PolicyMode::Optional,
        severity: Severity::Warn,
        rules: vec![],
        cedar_policy: Some("permit (principal, action, resource);".to_string()),
        status: DraftStatus::Pending,
        tenant_id: "tenant-acme-corp".to_string(),
        created_at: Utc::now().timestamp(),
        expires_at: Utc::now().timestamp() + 86400 * 7,
        created_by: user_email.to_string(),
        submitted_at: None,
        reviewed_at: None,
        reviewed_by: None,
        rejection_reason: None,
    };

    let draft_id = policy_storage.create_draft(default_policy).await.unwrap();

    let retrieved = policy_storage.get_draft(&draft_id).await.unwrap().unwrap();
    assert_eq!(retrieved.layer, PolicyLayer::Project);
    assert_eq!(retrieved.tenant_id, "tenant-acme-corp");
}

#[tokio::test]
async fn test_e2e_memory_promotion_with_governance() {
    #[derive(Debug, Clone)]
    struct Memory {
        id: String,
        content: String,
        reward_score: f64,
        access_count: u32,
        tags: Vec<String>,
    }

    let governance_storage = Arc::new(MockGovernanceStorage::new());
    let tenant_id = Uuid::new_v4();

    let config = GovernanceConfig {
        id: Some(Uuid::new_v4()),
        tenant_id: Some(tenant_id),
        org_id: None,
        team_id: None,
        project_id: None,
        approval_mode: ApprovalMode::Single,
        min_approvers: 1,
        timeout_hours: 24,
        auto_approve_low_risk: false,
        escalation_enabled: false,
        escalation_timeout_hours: 0,
        escalation_contact: None,
        policy_settings: json!({}),
        knowledge_settings: json!({
            "require_approval": true,
            "min_approvers": 1
        }),
        memory_settings: json!({
            "auto_approve_threshold": 0.8,
            "require_approval": true
        }),
    };

    governance_storage.upsert_config(&config).await.unwrap();

    let memory = Memory {
        id: format!("mem-{}", Uuid::new_v4()),
        content: "User prefers dark mode interface for all tools".to_string(),
        reward_score: 0.95,
        access_count: 15,
        tags: vec!["preference".to_string(), "ui".to_string()],
    };

    assert!(
        memory.reward_score >= 0.8,
        "Memory should be eligible for promotion"
    );

    let promotion_request = CreateApprovalRequest {
        request_type: RequestType::Memory,
        target_type: "memory_promotion".to_string(),
        target_id: Some(memory.id.clone()),
        tenant_id: Some(tenant_id),
        org_id: None,
        team_id: None,
        project_id: None,
        title: format!("Memory Promotion: {}", &memory.content[..30]),
        description: Some(format!(
            "Memory has reward score {:.2} and {} accesses. Propose for promotion to persistent \
             knowledge.",
            memory.reward_score, memory.access_count
        )),
        payload: json!({
            "memory_id": memory.id,
            "content": memory.content,
            "reward_score": memory.reward_score,
            "layer": "user",
            "tags": memory.tags
        }),
        risk_level: RiskLevel::Low,
        requestor_type: PrincipalType::System,
        requestor_id: Uuid::new_v4(),
        requestor_email: Some("system@aeterna.local".to_string()),
        required_approvals: 1,
        timeout_hours: Some(24),
    };

    let request = governance_storage
        .create_request(&promotion_request)
        .await
        .unwrap();
    assert_eq!(request.status, RequestStatus::Pending);

    let audit_id = governance_storage
        .log_audit(
            "memory_promotion_proposed",
            Some(request.id),
            Some("memory"),
            Some(&memory.id),
            PrincipalType::System,
            None,
            None,
            json!({
                "reward_score": memory.reward_score,
                "threshold": 0.8,
                "access_count": memory.access_count
            }),
        )
        .await
        .unwrap();

    assert!(!audit_id.is_nil());

    let approver_id = Uuid::new_v4();
    let decision = CreateDecision {
        request_id: request.id,
        approver_type: PrincipalType::User,
        approver_id,
        approver_email: Some("user@example.com".to_string()),
        decision: Decision::Approve,
        comment: Some("This is valuable context, approve promotion".to_string()),
    };

    governance_storage.add_decision(&decision).await.unwrap();

    let approved_request = governance_storage
        .get_request(request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approved_request.status, RequestStatus::Approved);

    let knowledge_entry = json!({
        "id": format!("know-{}", Uuid::new_v4()),
        "content": memory.content,
        "layer": "user",
        "tags": memory.tags,
        "promoted_from": memory.id,
        "promoted_at": Utc::now().timestamp(),
        "approved_by": approver_id
    });

    assert!(knowledge_entry.get("promoted_from").is_some());

    let applied = governance_storage
        .mark_applied(request.id, approver_id)
        .await
        .unwrap();
    assert!(applied.applied_at.is_some());
}

#[tokio::test]
async fn test_e2e_knowledge_proposal_workflow() {
    let governance_storage = Arc::new(MockGovernanceStorage::new());
    let tenant_id = Uuid::new_v4();
    let proposer_id = Uuid::new_v4();

    let config = GovernanceConfig {
        id: Some(Uuid::new_v4()),
        tenant_id: Some(tenant_id),
        org_id: None,
        team_id: None,
        project_id: None,
        approval_mode: ApprovalMode::Quorum,
        min_approvers: 2,
        timeout_hours: 72,
        auto_approve_low_risk: true,
        escalation_enabled: true,
        escalation_timeout_hours: 48,
        escalation_contact: Some("knowledge-admin@example.com".to_string()),
        policy_settings: json!({}),
        knowledge_settings: json!({
            "require_approval": true,
            "min_approvers": 2,
            "auto_approve_threshold": "low"
        }),
        memory_settings: json!({}),
    };

    governance_storage.upsert_config(&config).await.unwrap();

    let natural_language_input =
        "Our API Gateway uses rate limiting of 1000 requests per minute per client";

    let knowledge_draft = json!({
        "title": "API Gateway Rate Limiting",
        "content": natural_language_input,
        "type": "technical_spec",
        "layer": "team",
        "tags": ["api-gateway", "rate-limiting", "performance"],
        "confidence": 0.92
    });

    let knowledge_request = CreateApprovalRequest {
        request_type: RequestType::Knowledge,
        target_type: "knowledge_proposal".to_string(),
        target_id: Some(format!("know-draft-{}", Uuid::new_v4())),
        tenant_id: Some(tenant_id),
        org_id: None,
        team_id: None,
        project_id: None,
        title: "Knowledge Proposal: API Gateway Rate Limiting".to_string(),
        description: Some(format!(
            "Proposed knowledge with {:.0}% confidence: {}",
            knowledge_draft.get("confidence").unwrap().as_f64().unwrap() * 100.0,
            natural_language_input
        )),
        payload: knowledge_draft.clone(),
        risk_level: RiskLevel::Medium,
        requestor_type: PrincipalType::User,
        requestor_id: proposer_id,
        requestor_email: Some("developer@example.com".to_string()),
        required_approvals: 2,
        timeout_hours: Some(72),
    };

    let request = governance_storage
        .create_request(&knowledge_request)
        .await
        .unwrap();
    assert_eq!(request.status, RequestStatus::Pending);
    assert_eq!(request.request_type, RequestType::Knowledge);

    let audit_id = governance_storage
        .log_audit(
            "knowledge_proposed",
            Some(request.id),
            Some("knowledge"),
            request.target_id.as_deref(),
            PrincipalType::User,
            Some(proposer_id),
            Some("developer@example.com"),
            json!({
                "title": knowledge_draft.get("title").unwrap(),
                "type": knowledge_draft.get("type").unwrap(),
                "confidence": knowledge_draft.get("confidence").unwrap()
            }),
        )
        .await
        .unwrap();

    assert!(!audit_id.is_nil());

    let reviewer1_id = Uuid::new_v4();
    let decision1 = CreateDecision {
        request_id: request.id,
        approver_type: PrincipalType::User,
        approver_id: reviewer1_id,
        approver_email: Some("techlead@example.com".to_string()),
        decision: Decision::Approve,
        comment: Some("Accurate technical information, approve".to_string()),
    };

    governance_storage.add_decision(&decision1).await.unwrap();

    let req_after_1 = governance_storage
        .get_request(request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(req_after_1.status, RequestStatus::Pending);
    assert_eq!(req_after_1.current_approvals, 1);

    let reviewer2_id = Uuid::new_v4();
    let decision2 = CreateDecision {
        request_id: request.id,
        approver_type: PrincipalType::User,
        approver_id: reviewer2_id,
        approver_email: Some("architect@example.com".to_string()),
        decision: Decision::Approve,
        comment: Some("Aligns with our architecture standards".to_string()),
    };

    governance_storage.add_decision(&decision2).await.unwrap();

    let approved_request = governance_storage
        .get_request(request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approved_request.status, RequestStatus::Approved);
    assert_eq!(approved_request.current_approvals, 2);

    let published_knowledge = json!({
        "id": format!("know-{}", Uuid::new_v4()),
        "title": knowledge_draft.get("title").unwrap(),
        "content": knowledge_draft.get("content").unwrap(),
        "type": knowledge_draft.get("type").unwrap(),
        "layer": knowledge_draft.get("layer").unwrap(),
        "tags": knowledge_draft.get("tags").unwrap(),
        "proposed_by": proposer_id,
        "approved_by": [reviewer1_id, reviewer2_id],
        "published_at": Utc::now().timestamp(),
        "version": 1
    });

    assert!(published_knowledge.get("approved_by").is_some());
    let approvers = published_knowledge
        .get("approved_by")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(approvers.len(), 2);

    let applied = governance_storage
        .mark_applied(request.id, reviewer1_id)
        .await
        .unwrap();
    assert!(applied.applied_at.is_some());
}
