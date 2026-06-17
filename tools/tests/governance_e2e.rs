use async_trait::async_trait;
use knowledge::governance::GovernanceEngine;
use mk_core::traits::StorageBackend;
use mk_core::types::{
    ConstraintOperator, ConstraintSeverity, ConstraintTarget, KnowledgeLayer, OrganizationalUnit,
    Policy, PolicyMode, PolicyRule, RecordSource, RoleIdentifier, RuleMergeStrategy, RuleType,
    TenantContext, TenantId, UnitType, UserId,
};
use std::collections::HashMap;
use std::sync::Arc;
use storage::postgres::PostgresError;
use tokio::sync::RwLock;

struct MockGovernanceStorage {
    policies: Arc<RwLock<HashMap<String, Vec<Policy>>>>,
    units: Arc<RwLock<HashMap<String, OrganizationalUnit>>>,
    drift_results: Arc<RwLock<Vec<mk_core::types::DriftResult>>>,
}

impl MockGovernanceStorage {
    fn new() -> Self {
        Self {
            policies: Arc::new(RwLock::new(HashMap::new())),
            units: Arc::new(RwLock::new(HashMap::new())),
            drift_results: Arc::new(RwLock::new(Vec::new())),
        }
    }

    async fn add_unit(&self, unit: OrganizationalUnit) {
        self.units.write().await.insert(unit.id.clone(), unit);
    }

    async fn add_policy_for_unit(&self, unit_id: &str, policy: Policy) {
        let mut policies = self.policies.write().await;
        policies
            .entry(unit_id.to_string())
            .or_insert_with(Vec::new)
            .push(policy);
    }
}

#[async_trait]
impl StorageBackend for MockGovernanceStorage {
    type Error = PostgresError;

    async fn store(
        &self,
        _ctx: TenantContext,
        _key: &str,
        _value: &[u8],
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn retrieve(
        &self,
        _ctx: TenantContext,
        _key: &str,
    ) -> Result<Option<Vec<u8>>, Self::Error> {
        Ok(None)
    }

    async fn delete(&self, _ctx: TenantContext, _key: &str) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn exists(&self, _ctx: TenantContext, _key: &str) -> Result<bool, Self::Error> {
        Ok(false)
    }

    async fn get_ancestors(
        &self,
        _ctx: TenantContext,
        unit_id: &str,
    ) -> Result<Vec<OrganizationalUnit>, Self::Error> {
        let units = self.units.read().await;
        let mut ancestors = Vec::new();

        if let Some(unit) = units.get(unit_id) {
            let mut current_parent = unit.parent_id.clone();
            while let Some(parent_id) = current_parent {
                if let Some(parent) = units.get(&parent_id) {
                    ancestors.push(parent.clone());
                    current_parent = parent.parent_id.clone();
                } else {
                    break;
                }
            }
        }

        Ok(ancestors)
    }

    async fn get_descendants(
        &self,
        _ctx: TenantContext,
        unit_id: &str,
    ) -> Result<Vec<OrganizationalUnit>, Self::Error> {
        let units = self.units.read().await;
        let descendants: Vec<OrganizationalUnit> = units
            .values()
            .filter(|u| u.parent_id.as_deref() == Some(unit_id))
            .cloned()
            .collect();
        Ok(descendants)
    }

    async fn get_unit_policies(
        &self,
        _ctx: TenantContext,
        unit_id: &str,
    ) -> Result<Vec<Policy>, Self::Error> {
        let policies = self.policies.read().await;
        Ok(policies.get(unit_id).cloned().unwrap_or_default())
    }

    async fn add_unit_policy(
        &self,
        _ctx: &TenantContext,
        unit_id: &str,
        policy: &Policy,
    ) -> Result<(), Self::Error> {
        let mut policies = self.policies.write().await;
        policies
            .entry(unit_id.to_string())
            .or_insert_with(Vec::new)
            .push(policy.clone());
        Ok(())
    }

    async fn assign_role(
        &self,
        _user_id: &UserId,
        _tenant_id: &TenantId,
        _unit_id: &str,
        _role: RoleIdentifier,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn remove_role(
        &self,
        _user_id: &UserId,
        _tenant_id: &TenantId,
        _unit_id: &str,
        _role: RoleIdentifier,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn store_drift_result(
        &self,
        result: mk_core::types::DriftResult,
    ) -> Result<(), Self::Error> {
        self.drift_results.write().await.push(result);
        Ok(())
    }

    async fn get_latest_drift_result(
        &self,
        _ctx: TenantContext,
        project_id: &str,
    ) -> Result<Option<mk_core::types::DriftResult>, Self::Error> {
        let results = self.drift_results.read().await;
        Ok(results
            .iter()
            .rfind(|r| r.project_id == project_id)
            .cloned())
    }

    async fn list_all_units(&self) -> Result<Vec<OrganizationalUnit>, Self::Error> {
        Ok(self.units.read().await.values().cloned().collect())
    }

    async fn record_job_status(
        &self,
        _job_type: &str,
        _tenant_id: &str,
        _status: &str,
        _error: Option<&str>,
        _started_at: i64,
        _completed_at: Option<i64>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn get_governance_events(
        &self,
        _ctx: TenantContext,
        _since_timestamp: i64,
        _limit: usize,
    ) -> Result<Vec<mk_core::types::GovernanceEvent>, Self::Error> {
        Ok(Vec::new())
    }

    async fn create_suppression(
        &self,
        _suppression: mk_core::types::DriftSuppression,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn list_suppressions(
        &self,
        _ctx: TenantContext,
        _project_id: &str,
    ) -> Result<Vec<mk_core::types::DriftSuppression>, Self::Error> {
        Ok(Vec::new())
    }

    async fn delete_suppression(
        &self,
        _ctx: TenantContext,
        _suppression_id: &str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn get_drift_config(
        &self,
        _ctx: TenantContext,
        _project_id: &str,
    ) -> Result<Option<mk_core::types::DriftConfig>, Self::Error> {
        Ok(None)
    }

    async fn save_drift_config(
        &self,
        _config: mk_core::types::DriftConfig,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn persist_event(
        &self,
        _event: mk_core::types::PersistentEvent,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn get_pending_events(
        &self,
        _ctx: TenantContext,
        _limit: usize,
    ) -> Result<Vec<mk_core::types::PersistentEvent>, Self::Error> {
        Ok(Vec::new())
    }

    async fn update_event_status(
        &self,
        _event_id: &str,
        _status: mk_core::types::EventStatus,
        _error: Option<String>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn get_dead_letter_events(
        &self,
        _ctx: TenantContext,
        _limit: usize,
    ) -> Result<Vec<mk_core::types::PersistentEvent>, Self::Error> {
        Ok(Vec::new())
    }

    async fn check_idempotency(
        &self,
        _consumer_group: &str,
        _idempotency_key: &str,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    async fn record_consumer_state(
        &self,
        _state: mk_core::types::ConsumerState,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn get_event_metrics(
        &self,
        _ctx: TenantContext,
        _period_start: i64,
        _period_end: i64,
    ) -> Result<Vec<mk_core::types::EventDeliveryMetrics>, Self::Error> {
        Ok(Vec::new())
    }

    async fn record_event_metrics(
        &self,
        _metrics: mk_core::types::EventDeliveryMetrics,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn create_unit(&self, unit: &OrganizationalUnit) -> Result<(), Self::Error> {
        self.units
            .write()
            .await
            .insert(unit.id.clone(), unit.clone());
        Ok(())
    }

    async fn get_unit_by_id(
        &self,
        _unit_id: &str,
        _tenant_id: &str,
    ) -> Result<Option<OrganizationalUnit>, Self::Error> {
        Ok(None)
    }

    async fn update_unit(&self, _unit: &OrganizationalUnit) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn delete_unit(&self, _unit_id: &str, _tenant_id: &str) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn list_unit_members(
        &self,
        _unit_id: &str,
        _tenant_id: &str,
    ) -> Result<Vec<(UserId, RoleIdentifier)>, Self::Error> {
        Ok(Vec::new())
    }

    async fn assign_team_to_project(
        &self,
        _project_id: &str,
        _team_id: &str,
        _tenant_id: &str,
        _assignment_type: &str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn remove_team_from_project(
        &self,
        _project_id: &str,
        _team_id: &str,
        _tenant_id: &str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn list_project_team_assignments(
        &self,
        _project_id: &str,
        _tenant_id: &str,
    ) -> Result<Vec<(String, String)>, Self::Error> {
        Ok(Vec::new())
    }

    async fn get_effective_roles_at_scope(
        &self,
        _user_id: &UserId,
        _tenant_id: &TenantId,
        _unit_id: &str,
    ) -> Result<Vec<RoleIdentifier>, Self::Error> {
        Ok(Vec::new())
    }
}

fn create_tenant_context(tenant: &str, user: &str) -> TenantContext {
    TenantContext::new(
        TenantId::new(tenant.to_string()).unwrap(),
        UserId::new(user.to_string()).unwrap(),
    )
}

fn create_unit(
    id: &str,
    name: &str,
    unit_type: UnitType,
    parent: Option<&str>,
    tenant: &str,
) -> OrganizationalUnit {
    OrganizationalUnit {
        id: id.to_string(),
        name: name.to_string(),
        unit_type,
        parent_id: parent.map(String::from),
        tenant_id: TenantId::new(tenant.to_string()).unwrap(),
        metadata: HashMap::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: RecordSource::Admin,
    }
}

#[tokio::test]
async fn test_e2e_complete_governance_workflow() {
    let storage = Arc::new(MockGovernanceStorage::new());

    let tenant = create_unit(
        "acme-corp",
        "Acme Tenant",
        UnitType::Organization,
        None,
        "tenant-1",
    );
    let org = create_unit(
        "engineering",
        "Engineering Org",
        UnitType::Organization,
        Some("acme-corp"),
        "tenant-1",
    );
    let team = create_unit(
        "platform-team",
        "Platform Team",
        UnitType::Team,
        Some("engineering"),
        "tenant-1",
    );
    let project = create_unit(
        "api-gateway",
        "API Gateway",
        UnitType::Project,
        Some("platform-team"),
        "tenant-1",
    );

    storage.add_unit(tenant).await;
    storage.add_unit(org).await;
    storage.add_unit(team).await;
    storage.add_unit(project).await;

    let tenant_policy = Policy {
        id: "security-baseline".to_string(),
        name: "Tenant Security Baseline".to_string(),
        description: Some("Mandatory security requirements".to_string()),
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Mandatory,
        merge_strategy: RuleMergeStrategy::Merge,
        rules: vec![PolicyRule {
            id: "no-eval".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustNotMatch,
            value: serde_json::json!(r"eval\s*\("),
            severity: ConstraintSeverity::Block,
            message: "eval() is forbidden for security reasons".to_string(),
        }],
        metadata: HashMap::new(),
    };

    let team_policy = Policy {
        id: "team-standards".to_string(),
        name: "Platform Team Standards".to_string(),
        description: Some("Team coding standards".to_string()),
        layer: KnowledgeLayer::Team,
        mode: PolicyMode::Mandatory,
        merge_strategy: RuleMergeStrategy::Merge,
        rules: vec![PolicyRule {
            id: "require-logging".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustUse,
            value: serde_json::json!("tracing"),
            severity: ConstraintSeverity::Warn,
            message: "Projects should use tracing for observability".to_string(),
        }],
        metadata: HashMap::new(),
    };

    storage
        .add_policy_for_unit("acme-corp", tenant_policy)
        .await;
    storage
        .add_policy_for_unit("platform-team", team_policy)
        .await;

    let engine = GovernanceEngine::new().with_storage(storage.clone());

    let ctx = create_tenant_context("tenant-1", "developer-1");

    let mut context = HashMap::new();
    context.insert("unitId".to_string(), serde_json::json!("api-gateway"));
    context.insert(
        "content".to_string(),
        serde_json::json!("function safe() { return 1; }"),
    );
    context.insert(
        "dependencies".to_string(),
        serde_json::json!(["tracing", "tokio"]),
    );

    let drift_score = engine
        .check_drift(&ctx, "api-gateway", &context)
        .await
        .unwrap();
    assert_eq!(drift_score, 0.0, "Compliant code should have zero drift");

    let mut bad_context = HashMap::new();
    bad_context.insert("unitId".to_string(), serde_json::json!("api-gateway"));
    bad_context.insert(
        "content".to_string(),
        serde_json::json!("let result = eval('malicious');"),
    );
    bad_context.insert("dependencies".to_string(), serde_json::json!(["tokio"]));

    let drift_score = engine
        .check_drift(&ctx, "api-gateway", &bad_context)
        .await
        .unwrap();
    assert!(
        drift_score > 0.0,
        "Violating code should have non-zero drift"
    );
}

#[tokio::test]
async fn test_e2e_multi_tenant_isolation() {
    let storage = Arc::new(MockGovernanceStorage::new());

    let tenant1_company = create_unit(
        "tenant-a",
        "Tenant A",
        UnitType::Organization,
        None,
        "tenant-1",
    );
    let tenant2_company = create_unit(
        "tenant-b",
        "Tenant B",
        UnitType::Organization,
        None,
        "tenant-2",
    );

    storage.add_unit(tenant1_company).await;
    storage.add_unit(tenant2_company).await;

    let tenant1_policy = Policy {
        id: "t1-policy".to_string(),
        name: "Tenant 1 Policy".to_string(),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Mandatory,
        merge_strategy: RuleMergeStrategy::Override,
        rules: vec![PolicyRule {
            id: "t1-rule".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustUse,
            value: serde_json::json!("tenant1-lib"),
            severity: ConstraintSeverity::Block,
            message: "Tenant 1 requires tenant1-lib".to_string(),
        }],
        metadata: HashMap::new(),
    };

    let tenant2_policy = Policy {
        id: "t2-policy".to_string(),
        name: "Tenant 2 Policy".to_string(),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Mandatory,
        merge_strategy: RuleMergeStrategy::Override,
        rules: vec![PolicyRule {
            id: "t2-rule".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustUse,
            value: serde_json::json!("tenant2-lib"),
            severity: ConstraintSeverity::Block,
            message: "Tenant 2 requires tenant2-lib".to_string(),
        }],
        metadata: HashMap::new(),
    };

    storage
        .add_policy_for_unit("tenant-a", tenant1_policy)
        .await;
    storage
        .add_policy_for_unit("tenant-b", tenant2_policy)
        .await;

    let engine = GovernanceEngine::new().with_storage(storage);

    let ctx1 = create_tenant_context("tenant-1", "user-1");
    let ctx2 = create_tenant_context("tenant-2", "user-2");

    let mut context1 = HashMap::new();
    context1.insert("unitId".to_string(), serde_json::json!("tenant-a"));
    context1.insert(
        "dependencies".to_string(),
        serde_json::json!(["tenant1-lib"]),
    );

    let mut context2 = HashMap::new();
    context2.insert("unitId".to_string(), serde_json::json!("tenant-b"));
    context2.insert(
        "dependencies".to_string(),
        serde_json::json!(["tenant2-lib"]),
    );

    let score1 = engine
        .check_drift(&ctx1, "tenant-a", &context1)
        .await
        .unwrap();
    let score2 = engine
        .check_drift(&ctx2, "tenant-b", &context2)
        .await
        .unwrap();

    assert_eq!(score1, 0.0, "Tenant 1 compliant with tenant1-lib");
    assert_eq!(score2, 0.0, "Tenant 2 compliant with tenant2-lib");

    let mut cross_context = HashMap::new();
    cross_context.insert("unitId".to_string(), serde_json::json!("tenant-a"));
    cross_context.insert(
        "dependencies".to_string(),
        serde_json::json!(["tenant2-lib"]),
    );

    let cross_score = engine
        .check_drift(&ctx1, "tenant-a", &cross_context)
        .await
        .unwrap();
    assert!(
        cross_score > 0.0,
        "Tenant 1 should fail without tenant1-lib"
    );
}

#[tokio::test]
async fn test_e2e_policy_inheritance_chain() {
    let storage = Arc::new(MockGovernanceStorage::new());

    let tenant = create_unit("corp", "Corporation", UnitType::Organization, None, "t1");
    let org = create_unit(
        "org",
        "Organization",
        UnitType::Organization,
        Some("corp"),
        "t1",
    );
    let team = create_unit("team", "Team", UnitType::Team, Some("org"), "t1");
    let project = create_unit("proj", "Project", UnitType::Project, Some("team"), "t1");

    storage.add_unit(tenant).await;
    storage.add_unit(org).await;
    storage.add_unit(team).await;
    storage.add_unit(project).await;

    storage
        .add_policy_for_unit(
            "corp",
            Policy {
                id: "tenant-rule".to_string(),
                name: "Tenant Rule".to_string(),
                description: None,
                layer: KnowledgeLayer::Tenant,
                mode: PolicyMode::Mandatory,
                merge_strategy: RuleMergeStrategy::Merge,
                rules: vec![PolicyRule {
                    id: "req-a".to_string(),
                    rule_type: RuleType::Allow,
                    target: ConstraintTarget::Dependency,
                    operator: ConstraintOperator::MustUse,
                    value: serde_json::json!("lib-a"),
                    severity: ConstraintSeverity::Info,
                    message: "Tenant requires lib-a".to_string(),
                }],
                metadata: HashMap::new(),
            },
        )
        .await;

    storage
        .add_policy_for_unit(
            "org",
            Policy {
                id: "org-rule".to_string(),
                name: "Org Rule".to_string(),
                description: None,
                layer: KnowledgeLayer::Org,
                mode: PolicyMode::Mandatory,
                merge_strategy: RuleMergeStrategy::Merge,
                rules: vec![PolicyRule {
                    id: "req-b".to_string(),
                    rule_type: RuleType::Allow,
                    target: ConstraintTarget::Dependency,
                    operator: ConstraintOperator::MustUse,
                    value: serde_json::json!("lib-b"),
                    severity: ConstraintSeverity::Info,
                    message: "Org requires lib-b".to_string(),
                }],
                metadata: HashMap::new(),
            },
        )
        .await;

    storage
        .add_policy_for_unit(
            "team",
            Policy {
                id: "team-rule".to_string(),
                name: "Team Rule".to_string(),
                description: None,
                layer: KnowledgeLayer::Team,
                mode: PolicyMode::Mandatory,
                merge_strategy: RuleMergeStrategy::Merge,
                rules: vec![PolicyRule {
                    id: "req-c".to_string(),
                    rule_type: RuleType::Allow,
                    target: ConstraintTarget::Dependency,
                    operator: ConstraintOperator::MustUse,
                    value: serde_json::json!("lib-c"),
                    severity: ConstraintSeverity::Info,
                    message: "Team requires lib-c".to_string(),
                }],
                metadata: HashMap::new(),
            },
        )
        .await;

    let engine = GovernanceEngine::new().with_storage(storage);
    let ctx = create_tenant_context("t1", "user");

    let mut all_libs = HashMap::new();
    all_libs.insert("unitId".to_string(), serde_json::json!("proj"));
    all_libs.insert(
        "dependencies".to_string(),
        serde_json::json!(["lib-a", "lib-b", "lib-c"]),
    );

    let score = engine.check_drift(&ctx, "proj", &all_libs).await.unwrap();
    assert_eq!(score, 0.0, "All inherited requirements satisfied");

    let mut missing_one = HashMap::new();
    missing_one.insert("unitId".to_string(), serde_json::json!("proj"));
    missing_one.insert(
        "dependencies".to_string(),
        serde_json::json!(["lib-a", "lib-b"]),
    );

    let score = engine
        .check_drift(&ctx, "proj", &missing_one)
        .await
        .unwrap();
    assert!(score > 0.0, "Missing lib-c should trigger drift");
}

#[tokio::test]
async fn test_e2e_drift_result_persistence() {
    let storage = Arc::new(MockGovernanceStorage::new());

    let project = create_unit("test-proj", "Test Project", UnitType::Project, None, "t1");
    storage.add_unit(project).await;

    storage
        .add_policy_for_unit(
            "test-proj",
            Policy {
                id: "test-policy".to_string(),
                name: "Test Policy".to_string(),
                description: None,
                layer: KnowledgeLayer::Project,
                mode: PolicyMode::Mandatory,
                merge_strategy: RuleMergeStrategy::Override,
                rules: vec![PolicyRule {
                    id: "test-rule".to_string(),
                    rule_type: RuleType::Allow,
                    target: ConstraintTarget::Dependency,
                    operator: ConstraintOperator::MustUse,
                    value: serde_json::json!("required-lib"),
                    severity: ConstraintSeverity::Warn,
                    message: "Required lib missing".to_string(),
                }],
                metadata: HashMap::new(),
            },
        )
        .await;

    let engine = GovernanceEngine::new().with_storage(storage.clone());
    let ctx = create_tenant_context("t1", "user");

    let mut context = HashMap::new();
    context.insert("unitId".to_string(), serde_json::json!("test-proj"));
    context.insert("dependencies".to_string(), serde_json::json!(["other-lib"]));

    let _score = engine
        .check_drift(&ctx, "test-proj", &context)
        .await
        .unwrap();

    let stored_result = storage
        .get_latest_drift_result(ctx.clone(), "test-proj")
        .await
        .unwrap();
    assert!(stored_result.is_some(), "Drift result should be persisted");

    let result = stored_result.unwrap();
    assert_eq!(result.project_id, "test-proj");
    assert!(
        !result.violations.is_empty(),
        "Violations should be recorded"
    );
}

#[tokio::test]
async fn test_e2e_policy_override_at_lower_layer() {
    let storage = Arc::new(MockGovernanceStorage::new());

    let tenant = create_unit("corp", "Corp", UnitType::Organization, None, "t1");
    let project = create_unit("proj", "Project", UnitType::Project, Some("corp"), "t1");

    storage.add_unit(tenant).await;
    storage.add_unit(project).await;

    storage
        .add_policy_for_unit(
            "corp",
            Policy {
                id: "shared-policy".to_string(),
                name: "Tenant Policy".to_string(),
                description: None,
                layer: KnowledgeLayer::Tenant,
                mode: PolicyMode::Mandatory,
                merge_strategy: RuleMergeStrategy::Override,
                rules: vec![PolicyRule {
                    id: "tenant-rule".to_string(),
                    rule_type: RuleType::Allow,
                    target: ConstraintTarget::Dependency,
                    operator: ConstraintOperator::MustUse,
                    value: serde_json::json!("old-lib"),
                    severity: ConstraintSeverity::Block,
                    message: "Tenant requires old-lib".to_string(),
                }],
                metadata: HashMap::new(),
            },
        )
        .await;

    storage
        .add_policy_for_unit(
            "proj",
            Policy {
                id: "shared-policy".to_string(),
                name: "Project Override".to_string(),
                description: None,
                layer: KnowledgeLayer::Project,
                mode: PolicyMode::Mandatory,
                merge_strategy: RuleMergeStrategy::Override,
                rules: vec![PolicyRule {
                    id: "project-rule".to_string(),
                    rule_type: RuleType::Allow,
                    target: ConstraintTarget::Dependency,
                    operator: ConstraintOperator::MustUse,
                    value: serde_json::json!("new-lib"),
                    severity: ConstraintSeverity::Block,
                    message: "Project requires new-lib".to_string(),
                }],
                metadata: HashMap::new(),
            },
        )
        .await;

    let engine = GovernanceEngine::new().with_storage(storage);
    let ctx = create_tenant_context("t1", "user");

    let mut context = HashMap::new();
    context.insert("unitId".to_string(), serde_json::json!("proj"));
    context.insert("dependencies".to_string(), serde_json::json!(["new-lib"]));

    let score = engine.check_drift(&ctx, "proj", &context).await.unwrap();
    assert_eq!(score, 0.0, "Project override should take precedence");
}

#[tokio::test]
async fn test_e2e_validation_result_structure() {
    let mut engine = GovernanceEngine::new();

    engine.add_policy(Policy {
        id: "test-policy".to_string(),
        name: "Test Policy".to_string(),
        description: Some("Test description".to_string()),
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Mandatory,
        merge_strategy: RuleMergeStrategy::Override,
        rules: vec![
            PolicyRule {
                id: "block-rule".to_string(),
                rule_type: RuleType::Allow,
                target: ConstraintTarget::Dependency,
                operator: ConstraintOperator::MustUse,
                value: serde_json::json!("critical-lib"),
                severity: ConstraintSeverity::Block,
                message: "Critical lib missing".to_string(),
            },
            PolicyRule {
                id: "warn-rule".to_string(),
                rule_type: RuleType::Allow,
                target: ConstraintTarget::Dependency,
                operator: ConstraintOperator::MustUse,
                value: serde_json::json!("recommended-lib"),
                severity: ConstraintSeverity::Warn,
                message: "Recommended lib missing".to_string(),
            },
        ],
        metadata: HashMap::new(),
    });

    let mut context = HashMap::new();
    context.insert("dependencies".to_string(), serde_json::json!([]));

    let result = engine.validate(KnowledgeLayer::Tenant, &context);

    assert!(!result.is_valid, "Should be invalid with violations");
    assert_eq!(result.violations.len(), 2, "Should have 2 violations");

    let block_violation = result.violations.iter().find(|v| v.rule_id == "block-rule");
    let warn_violation = result.violations.iter().find(|v| v.rule_id == "warn-rule");

    assert!(block_violation.is_some(), "Should have block violation");
    assert!(warn_violation.is_some(), "Should have warn violation");

    assert_eq!(block_violation.unwrap().severity, ConstraintSeverity::Block);
    assert_eq!(warn_violation.unwrap().severity, ConstraintSeverity::Warn);
}

#[tokio::test]
async fn test_e2e_empty_policy_graceful_handling() {
    let storage = Arc::new(MockGovernanceStorage::new());

    let project = create_unit("empty-proj", "Empty Project", UnitType::Project, None, "t1");
    storage.add_unit(project).await;

    let engine = GovernanceEngine::new().with_storage(storage);
    let ctx = create_tenant_context("t1", "user");

    let mut context = HashMap::new();
    context.insert("unitId".to_string(), serde_json::json!("empty-proj"));
    context.insert("dependencies".to_string(), serde_json::json!(["any-lib"]));

    let result = engine.check_drift(&ctx, "empty-proj", &context).await;
    assert!(result.is_ok(), "Should handle empty policies gracefully");

    let score = result.unwrap();
    assert!(
        score <= 0.5,
        "Should only have missing mandatory policies warning"
    );
}
