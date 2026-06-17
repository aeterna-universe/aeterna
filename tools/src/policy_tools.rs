//! # Policy Tools
//!
//! MCP tools for policy management: propose, approve, reject, list.

use crate::policy_translator::{
    DraftStatus, PolicyDraft, PolicyScope, PolicySeverity, StructuredIntent,
};
use crate::tools::Tool;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use mk_core::types::PolicyRule;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use storage::approval_workflow::{
    ApprovalEvent, ApprovalModeKind, ApprovalWorkflow, ApprovalWorkflowContext, RiskLevelKind,
    WorkflowState,
};
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyProposal {
    pub proposal_id: String,
    pub draft_id: String,
    pub name: String,
    pub rules: Vec<PolicyRule>,
    pub scope: PolicyScope,
    pub severity: PolicySeverity,
    pub intent: StructuredIntent,
    pub justification: Option<String>,
    pub proposed_by: String,
    pub proposed_at: DateTime<Utc>,
    pub workflow: ApprovalWorkflow,
    pub notified_approvers: Vec<String>,
    pub expires_at: DateTime<Utc>,
}

pub trait PolicyProposalStorage: Send + Sync {
    fn store_proposal(
        &self,
        proposal: PolicyProposal,
    ) -> impl std::future::Future<Output = Result<(), PolicyToolError>> + Send;

    fn get_proposal(
        &self,
        proposal_id: &str,
    ) -> impl std::future::Future<Output = Result<Option<PolicyProposal>, PolicyToolError>> + Send;

    fn get_proposal_by_draft(
        &self,
        draft_id: &str,
    ) -> impl std::future::Future<Output = Result<Option<PolicyProposal>, PolicyToolError>> + Send;

    fn update_proposal(
        &self,
        proposal: PolicyProposal,
    ) -> impl std::future::Future<Output = Result<(), PolicyToolError>> + Send;

    fn list_pending(
        &self,
        scope: Option<PolicyScope>,
    ) -> impl std::future::Future<Output = Result<Vec<PolicyProposal>, PolicyToolError>> + Send;

    fn get_draft(
        &self,
        draft_id: &str,
    ) -> impl std::future::Future<Output = Result<Option<PolicyDraft>, PolicyToolError>> + Send;
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum PolicyToolError {
    #[error("Draft not found: {0}")]
    DraftNotFound(String),

    #[error("Proposal not found: {0}")]
    ProposalNotFound(String),

    #[error("Draft not validated: {0}")]
    DraftNotValidated(String),

    #[error("Draft already submitted: {0}")]
    DraftAlreadySubmitted(String),

    #[error("Missing required field: {0}")]
    MissingField(String),

    #[error("Invalid state transition: {0}")]
    InvalidStateTransition(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Notification error: {0}")]
    NotificationError(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),
}

pub trait ApproverResolver: Send + Sync {
    fn get_approvers(
        &self,
        scope: &PolicyScope,
        severity: &PolicySeverity,
    ) -> impl std::future::Future<Output = Result<Vec<String>, PolicyToolError>> + Send;

    fn get_required_approvals(
        &self,
        scope: &PolicyScope,
        severity: &PolicySeverity,
    ) -> impl std::future::Future<Output = Result<u32, PolicyToolError>> + Send;

    fn get_approval_timeout_hours(
        &self,
        scope: &PolicyScope,
    ) -> impl std::future::Future<Output = Result<u32, PolicyToolError>> + Send;
}

pub trait NotificationService: Send + Sync {
    fn notify_approvers(
        &self,
        approvers: &[String],
        proposal: &PolicyProposal,
    ) -> impl std::future::Future<Output = Result<(), PolicyToolError>> + Send;

    fn notify_proposer(
        &self,
        proposer: &str,
        proposal: &PolicyProposal,
        status: &str,
        comment: Option<&str>,
    ) -> impl std::future::Future<Output = Result<(), PolicyToolError>> + Send;
}

pub struct PolicyProposeTool<S, R, N>
where
    S: PolicyProposalStorage,
    R: ApproverResolver,
    N: NotificationService,
{
    storage: Arc<S>,
    approver_resolver: Arc<R>,
    notification_service: Arc<N>,
}

impl<S, R, N> PolicyProposeTool<S, R, N>
where
    S: PolicyProposalStorage,
    R: ApproverResolver,
    N: NotificationService,
{
    pub fn new(storage: Arc<S>, approver_resolver: Arc<R>, notification_service: Arc<N>) -> Self {
        Self {
            storage,
            approver_resolver,
            notification_service,
        }
    }

    pub async fn propose(
        &self,
        draft_id: &str,
        justification: Option<String>,
        notify: Vec<String>,
        proposed_by: &str,
    ) -> Result<PolicyProposal, PolicyToolError> {
        let draft = self
            .storage
            .get_draft(draft_id)
            .await?
            .ok_or_else(|| PolicyToolError::DraftNotFound(draft_id.to_string()))?;

        if draft.status == DraftStatus::Submitted {
            return Err(PolicyToolError::DraftAlreadySubmitted(draft_id.to_string()));
        }

        if draft.status == DraftStatus::ValidationFailed {
            return Err(PolicyToolError::DraftNotValidated(draft_id.to_string()));
        }

        if let Some(existing) = self.storage.get_proposal_by_draft(draft_id).await? {
            return Err(PolicyToolError::DraftAlreadySubmitted(existing.proposal_id));
        }

        let scope = self.extract_scope_from_rules(&draft.rules);
        let severity = draft.intent.severity;

        let required_approvals = self
            .approver_resolver
            .get_required_approvals(&scope, &severity)
            .await?;

        let timeout_hours = self
            .approver_resolver
            .get_approval_timeout_hours(&scope)
            .await?;

        let mut approvers = self
            .approver_resolver
            .get_approvers(&scope, &severity)
            .await?;

        for explicit in &notify {
            if !approvers.contains(explicit) {
                approvers.push(explicit.clone());
            }
        }

        let proposal_id = format!("prop-{}", Uuid::new_v4());
        let now = Utc::now();
        let expires_at = now + chrono::Duration::hours(timeout_hours as i64);

        let risk_level = match severity {
            PolicySeverity::Block => RiskLevelKind::High,
            PolicySeverity::Warn => RiskLevelKind::Medium,
            PolicySeverity::Info => RiskLevelKind::Low,
        };

        let approval_mode = match scope {
            PolicyScope::Tenant => ApprovalModeKind::Unanimous,
            PolicyScope::Org => ApprovalModeKind::Quorum,
            _ => ApprovalModeKind::Single,
        };

        let workflow_ctx = ApprovalWorkflowContext {
            request_id: Uuid::new_v4(),
            request_type: "policy_proposal".to_string(),
            required_approvals: required_approvals as i32,
            current_approvals: 0,
            approval_mode,
            timeout_hours: timeout_hours as i32,
            auto_approve_low_risk: false,
            risk_level,
        };

        let mut workflow = ApprovalWorkflow::new(workflow_ctx);
        workflow
            .handle(ApprovalEvent::Submit {
                requestor_id: Uuid::new_v4(),
                submitted_at: now,
            })
            .map_err(|e| PolicyToolError::InvalidStateTransition(e.to_string()))?;

        let proposal = PolicyProposal {
            proposal_id: proposal_id.clone(),
            draft_id: draft_id.to_string(),
            name: draft.name,
            rules: draft.rules,
            scope,
            severity,
            intent: draft.intent,
            justification,
            proposed_by: proposed_by.to_string(),
            proposed_at: now,
            workflow,
            notified_approvers: approvers.clone(),
            expires_at,
        };

        self.storage.store_proposal(proposal.clone()).await?;

        self.notification_service
            .notify_approvers(&approvers, &proposal)
            .await?;

        Ok(proposal)
    }

    fn extract_scope_from_rules(&self, rules: &[PolicyRule]) -> PolicyScope {
        for rule in rules {
            let id = rule.id.to_lowercase();
            if id.contains("tenant") {
                return PolicyScope::Tenant;
            } else if id.contains("org") {
                return PolicyScope::Org;
            } else if id.contains("team") {
                return PolicyScope::Team;
            }
        }
        PolicyScope::Project
    }
}

#[async_trait]
impl<S, R, N> Tool for PolicyProposeTool<S, R, N>
where
    S: PolicyProposalStorage + 'static,
    R: ApproverResolver + 'static,
    N: NotificationService + 'static,
{
    fn name(&self) -> &str {
        "aeterna_policy_propose"
    }

    fn description(&self) -> &str {
        "Submit a validated policy draft for approval. Notifies configured approvers and starts \
         the approval workflow."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "draft_id": {
                    "type": "string",
                    "description": "ID of the validated draft to propose"
                },
                "justification": {
                    "type": "string",
                    "description": "Why this policy is needed"
                },
                "notify": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Additional approvers to notify (emails or user IDs)"
                },
                "proposed_by": {
                    "type": "string",
                    "description": "User ID or email of the proposer"
                }
            },
            "required": ["draft_id", "proposed_by"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        #[derive(Deserialize)]
        struct ProposeParams {
            draft_id: String,
            justification: Option<String>,
            #[serde(default)]
            notify: Vec<String>,
            proposed_by: String,
        }

        let p: ProposeParams = serde_json::from_value(params)?;

        let proposal = self
            .propose(&p.draft_id, p.justification, p.notify, &p.proposed_by)
            .await?;

        Ok(json!({
            "proposal_id": proposal.proposal_id,
            "status": "pending_approval",
            "required_approvers": proposal.workflow.context.required_approvals,
            "current_approvals": 0,
            "approvers_notified": proposal.notified_approvers,
            "expires_at": proposal.expires_at.to_rfc3339(),
        }))
    }
}

pub struct PolicyListPendingTool<S>
where
    S: PolicyProposalStorage,
{
    storage: Arc<S>,
}

impl<S: PolicyProposalStorage> PolicyListPendingTool<S> {
    pub fn new(storage: Arc<S>) -> Self {
        Self { storage }
    }
}

#[async_trait]
impl<S> Tool for PolicyListPendingTool<S>
where
    S: PolicyProposalStorage + 'static,
{
    fn name(&self) -> &str {
        "aeterna_policy_list_pending"
    }

    fn description(&self) -> &str {
        "List pending policy proposals awaiting approval."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "scope": {
                    "type": "string",
                    "enum": ["tenant", "org", "team", "project"],
                    "description": "Filter by scope (optional)"
                }
            }
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        #[derive(Deserialize)]
        struct ListParams {
            scope: Option<String>,
        }

        let p: ListParams = serde_json::from_value(params)?;
        let scope = p.scope.and_then(|s| s.parse::<PolicyScope>().ok());

        let proposals = self.storage.list_pending(scope).await?;

        let results: Vec<Value> = proposals
            .into_iter()
            .map(|p| {
                json!({
                    "proposal_id": p.proposal_id,
                    "name": p.name,
                    "scope": p.scope.to_string(),
                    "severity": p.severity.to_string(),
                    "proposed_by": p.proposed_by,
                    "proposed_at": p.proposed_at.to_rfc3339(),
                    "required_approvals": p.workflow.context.required_approvals,
                    "current_approvals": p.workflow.context.current_approvals,
                    "expires_at": p.expires_at.to_rfc3339(),
                })
            })
            .collect();

        Ok(json!({
            "proposals": results,
            "total": results.len()
        }))
    }
}

pub struct InMemoryPolicyStorage {
    proposals: RwLock<HashMap<String, PolicyProposal>>,
    drafts: RwLock<HashMap<String, PolicyDraft>>,
}

impl InMemoryPolicyStorage {
    pub fn new() -> Self {
        Self {
            proposals: RwLock::new(HashMap::new()),
            drafts: RwLock::new(HashMap::new()),
        }
    }

    pub async fn store_draft(&self, draft: PolicyDraft) {
        let mut drafts = self.drafts.write().await;
        drafts.insert(draft.draft_id.clone(), draft);
    }
}

impl Default for InMemoryPolicyStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicyProposalStorage for InMemoryPolicyStorage {
    async fn store_proposal(&self, proposal: PolicyProposal) -> Result<(), PolicyToolError> {
        let mut proposals = self.proposals.write().await;
        proposals.insert(proposal.proposal_id.clone(), proposal);
        Ok(())
    }

    async fn get_proposal(
        &self,
        proposal_id: &str,
    ) -> Result<Option<PolicyProposal>, PolicyToolError> {
        let proposals = self.proposals.read().await;
        Ok(proposals.get(proposal_id).cloned())
    }

    async fn get_proposal_by_draft(
        &self,
        draft_id: &str,
    ) -> Result<Option<PolicyProposal>, PolicyToolError> {
        let proposals = self.proposals.read().await;
        Ok(proposals.values().find(|p| p.draft_id == draft_id).cloned())
    }

    async fn update_proposal(&self, proposal: PolicyProposal) -> Result<(), PolicyToolError> {
        let mut proposals = self.proposals.write().await;
        proposals.insert(proposal.proposal_id.clone(), proposal);
        Ok(())
    }

    async fn list_pending(
        &self,
        scope: Option<PolicyScope>,
    ) -> Result<Vec<PolicyProposal>, PolicyToolError> {
        let proposals = self.proposals.read().await;
        let pending: Vec<_> = proposals
            .values()
            .filter(|p| {
                matches!(p.workflow.state, WorkflowState::Pending { .. })
                    && scope.as_ref().is_none_or(|s| &p.scope == s)
            })
            .cloned()
            .collect();
        Ok(pending)
    }

    async fn get_draft(&self, draft_id: &str) -> Result<Option<PolicyDraft>, PolicyToolError> {
        let drafts = self.drafts.read().await;
        Ok(drafts.get(draft_id).cloned())
    }
}

pub struct DefaultApproverResolver {
    approvers_by_scope: HashMap<PolicyScope, Vec<String>>,
}

impl DefaultApproverResolver {
    pub fn new() -> Self {
        let mut approvers = HashMap::new();
        approvers.insert(PolicyScope::Tenant, vec!["admin@example.com".to_string()]);
        approvers.insert(PolicyScope::Org, vec!["architect@example.com".to_string()]);
        approvers.insert(PolicyScope::Team, vec!["tech-lead@example.com".to_string()]);
        approvers.insert(
            PolicyScope::Project,
            vec!["tech-lead@example.com".to_string()],
        );

        Self {
            approvers_by_scope: approvers,
        }
    }

    pub fn with_approvers(mut self, scope: PolicyScope, approvers: Vec<String>) -> Self {
        self.approvers_by_scope.insert(scope, approvers);
        self
    }
}

impl Default for DefaultApproverResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl ApproverResolver for DefaultApproverResolver {
    async fn get_approvers(
        &self,
        scope: &PolicyScope,
        _severity: &PolicySeverity,
    ) -> Result<Vec<String>, PolicyToolError> {
        Ok(self
            .approvers_by_scope
            .get(scope)
            .cloned()
            .unwrap_or_default())
    }

    async fn get_required_approvals(
        &self,
        scope: &PolicyScope,
        severity: &PolicySeverity,
    ) -> Result<u32, PolicyToolError> {
        let base = match scope {
            PolicyScope::Tenant => 2,
            PolicyScope::Org => 2,
            PolicyScope::Team => 1,
            PolicyScope::Project => 1,
        };

        let severity_bonus = match severity {
            PolicySeverity::Block => 1,
            _ => 0,
        };

        Ok(base + severity_bonus)
    }

    async fn get_approval_timeout_hours(
        &self,
        scope: &PolicyScope,
    ) -> Result<u32, PolicyToolError> {
        Ok(match scope {
            PolicyScope::Tenant => 72,
            PolicyScope::Org => 48,
            PolicyScope::Team => 24,
            PolicyScope::Project => 24,
        })
    }
}

pub struct NoOpNotificationService;

impl NotificationService for NoOpNotificationService {
    async fn notify_approvers(
        &self,
        _approvers: &[String],
        _proposal: &PolicyProposal,
    ) -> Result<(), PolicyToolError> {
        Ok(())
    }

    async fn notify_proposer(
        &self,
        _proposer: &str,
        _proposal: &PolicyProposal,
        _status: &str,
        _comment: Option<&str>,
    ) -> Result<(), PolicyToolError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy_translator::{PolicyAction, TargetType, ValidationResult};
    use mk_core::types::{ConstraintOperator, ConstraintSeverity, ConstraintTarget, RuleType};

    fn create_test_draft(draft_id: &str, status: DraftStatus) -> PolicyDraft {
        PolicyDraft {
            draft_id: draft_id.to_string(),
            status,
            name: "test-policy".to_string(),
            rules: vec![PolicyRule {
                id: "test-project-rule".to_string(),
                rule_type: RuleType::Deny,
                target: ConstraintTarget::Dependency,
                operator: ConstraintOperator::MustNotUse,
                value: serde_json::Value::String("test".to_string()),
                severity: ConstraintSeverity::Warn,
                message: "Test dependency blocked".to_string(),
            }],
            explanation: "Test policy".to_string(),
            validation: ValidationResult {
                is_valid: true,
                errors: vec![],
                warnings: vec![],
            },
            intent: StructuredIntent {
                original: "Block test".to_string(),
                interpreted: "Block test dependency".to_string(),
                action: PolicyAction::Deny,
                target_type: TargetType::Dependency,
                target_value: "test".to_string(),
                condition: None,
                severity: PolicySeverity::Warn,
                confidence: 0.9,
            },
        }
    }

    #[tokio::test]
    async fn test_propose_creates_proposal() {
        let storage = Arc::new(InMemoryPolicyStorage::new());
        let resolver = Arc::new(DefaultApproverResolver::new());
        let notifier = Arc::new(NoOpNotificationService);

        let draft = create_test_draft("draft-123", DraftStatus::Validated);
        storage.store_draft(draft).await;

        let tool = PolicyProposeTool::new(storage.clone(), resolver, notifier);

        let proposal = tool
            .propose(
                "draft-123",
                Some("Testing".to_string()),
                vec![],
                "user@test.com",
            )
            .await
            .unwrap();

        assert!(proposal.proposal_id.starts_with("prop-"));
        assert_eq!(proposal.draft_id, "draft-123");
        assert_eq!(proposal.proposed_by, "user@test.com");
        assert!(matches!(
            proposal.workflow.state,
            WorkflowState::Pending { .. }
        ));
    }

    #[tokio::test]
    async fn test_propose_fails_for_invalid_draft() {
        let storage = Arc::new(InMemoryPolicyStorage::new());
        let resolver = Arc::new(DefaultApproverResolver::new());
        let notifier = Arc::new(NoOpNotificationService);

        let draft = create_test_draft("draft-invalid", DraftStatus::ValidationFailed);
        storage.store_draft(draft).await;

        let tool = PolicyProposeTool::new(storage, resolver, notifier);

        let result = tool
            .propose("draft-invalid", None, vec![], "user@test.com")
            .await;

        assert!(matches!(result, Err(PolicyToolError::DraftNotValidated(_))));
    }

    #[tokio::test]
    async fn test_propose_fails_for_already_submitted() {
        let storage = Arc::new(InMemoryPolicyStorage::new());
        let resolver = Arc::new(DefaultApproverResolver::new());
        let notifier = Arc::new(NoOpNotificationService);

        let draft = create_test_draft("draft-submitted", DraftStatus::Submitted);
        storage.store_draft(draft).await;

        let tool = PolicyProposeTool::new(storage, resolver, notifier);

        let result = tool
            .propose("draft-submitted", None, vec![], "user@test.com")
            .await;

        assert!(matches!(
            result,
            Err(PolicyToolError::DraftAlreadySubmitted(_))
        ));
    }

    #[tokio::test]
    async fn test_propose_fails_for_missing_draft() {
        let storage = Arc::new(InMemoryPolicyStorage::new());
        let resolver = Arc::new(DefaultApproverResolver::new());
        let notifier = Arc::new(NoOpNotificationService);

        let tool = PolicyProposeTool::new(storage, resolver, notifier);

        let result = tool
            .propose("nonexistent", None, vec![], "user@test.com")
            .await;

        assert!(matches!(result, Err(PolicyToolError::DraftNotFound(_))));
    }

    #[tokio::test]
    async fn test_propose_adds_explicit_approvers() {
        let storage = Arc::new(InMemoryPolicyStorage::new());
        let resolver = Arc::new(DefaultApproverResolver::new());
        let notifier = Arc::new(NoOpNotificationService);

        let draft = create_test_draft("draft-456", DraftStatus::Validated);
        storage.store_draft(draft).await;

        let tool = PolicyProposeTool::new(storage, resolver, notifier);

        let proposal = tool
            .propose(
                "draft-456",
                None,
                vec!["extra@approver.com".to_string()],
                "user@test.com",
            )
            .await
            .unwrap();

        assert!(
            proposal
                .notified_approvers
                .contains(&"extra@approver.com".to_string())
        );
    }

    #[tokio::test]
    async fn test_list_pending_returns_only_pending() {
        let storage = Arc::new(InMemoryPolicyStorage::new());
        let resolver = Arc::new(DefaultApproverResolver::new());
        let notifier = Arc::new(NoOpNotificationService);

        let draft1 = create_test_draft("draft-1", DraftStatus::Validated);
        let draft2 = create_test_draft("draft-2", DraftStatus::Validated);
        storage.store_draft(draft1).await;
        storage.store_draft(draft2).await;

        let tool = PolicyProposeTool::new(storage.clone(), resolver, notifier);

        tool.propose("draft-1", None, vec![], "user@test.com")
            .await
            .unwrap();
        tool.propose("draft-2", None, vec![], "user@test.com")
            .await
            .unwrap();

        let list_tool = PolicyListPendingTool::new(storage);
        let result = list_tool.call(json!({})).await.unwrap();

        assert_eq!(result["total"], 2);
    }

    #[tokio::test]
    async fn test_required_approvals_by_scope() {
        let resolver = DefaultApproverResolver::new();

        let company_approvals = resolver
            .get_required_approvals(&PolicyScope::Tenant, &PolicySeverity::Warn)
            .await
            .unwrap();
        assert_eq!(company_approvals, 2);

        let project_approvals = resolver
            .get_required_approvals(&PolicyScope::Project, &PolicySeverity::Warn)
            .await
            .unwrap();
        assert_eq!(project_approvals, 1);

        let blocking_approvals = resolver
            .get_required_approvals(&PolicyScope::Project, &PolicySeverity::Block)
            .await
            .unwrap();
        assert_eq!(blocking_approvals, 2);
    }

    #[tokio::test]
    async fn test_timeout_hours_by_scope() {
        let resolver = DefaultApproverResolver::new();

        let company_timeout = resolver
            .get_approval_timeout_hours(&PolicyScope::Tenant)
            .await
            .unwrap();
        assert_eq!(company_timeout, 72);

        let project_timeout = resolver
            .get_approval_timeout_hours(&PolicyScope::Project)
            .await
            .unwrap();
        assert_eq!(project_timeout, 24);
    }

    #[tokio::test]
    async fn test_extract_scope_from_rules() {
        let storage = Arc::new(InMemoryPolicyStorage::new());
        let resolver = Arc::new(DefaultApproverResolver::new());
        let notifier = Arc::new(NoOpNotificationService);

        let tool = PolicyProposeTool::new(storage, resolver, notifier);

        let company_rules = vec![PolicyRule {
            id: "tenant-policy".to_string(),
            rule_type: RuleType::Deny,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustNotUse,
            value: serde_json::Value::String("test".to_string()),
            severity: ConstraintSeverity::Block,
            message: "Tenant-wide block".to_string(),
        }];
        assert_eq!(
            tool.extract_scope_from_rules(&company_rules),
            PolicyScope::Tenant
        );

        let org_rules = vec![PolicyRule {
            id: "org-standard".to_string(),
            rule_type: RuleType::Deny,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustNotUse,
            value: serde_json::Value::String("test".to_string()),
            severity: ConstraintSeverity::Warn,
            message: "Org standard".to_string(),
        }];
        assert_eq!(tool.extract_scope_from_rules(&org_rules), PolicyScope::Org);

        let team_rules = vec![PolicyRule {
            id: "team-convention".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::File,
            operator: ConstraintOperator::MustExist,
            value: serde_json::Value::String("README.md".to_string()),
            severity: ConstraintSeverity::Info,
            message: "Team requires README".to_string(),
        }];
        assert_eq!(
            tool.extract_scope_from_rules(&team_rules),
            PolicyScope::Team
        );

        let project_rules = vec![PolicyRule {
            id: "local-rule".to_string(),
            rule_type: RuleType::Deny,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustNotMatch,
            value: serde_json::Value::String("console.log".to_string()),
            severity: ConstraintSeverity::Warn,
            message: "No console.log".to_string(),
        }];
        assert_eq!(
            tool.extract_scope_from_rules(&project_rules),
            PolicyScope::Project
        );
    }

    #[tokio::test]
    async fn test_tool_interface() {
        let storage = Arc::new(InMemoryPolicyStorage::new());
        let resolver = Arc::new(DefaultApproverResolver::new());
        let notifier = Arc::new(NoOpNotificationService);

        let draft = create_test_draft("draft-tool", DraftStatus::Validated);
        storage.store_draft(draft).await;

        let tool = PolicyProposeTool::new(storage, resolver, notifier);

        assert_eq!(tool.name(), "aeterna_policy_propose");
        assert!(tool.description().contains("approval"));

        let result = tool
            .call(json!({
                "draft_id": "draft-tool",
                "proposed_by": "user@test.com"
            }))
            .await
            .unwrap();

        assert_eq!(result["status"], "pending_approval");
        assert!(result["proposal_id"].as_str().unwrap().starts_with("prop-"));
    }
}
