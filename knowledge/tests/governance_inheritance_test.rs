use knowledge::governance::GovernanceEngine;
use mk_core::types::{
    ConstraintOperator, ConstraintSeverity, ConstraintTarget, KnowledgeLayer, Policy, PolicyMode,
    PolicyRule, RuleMergeStrategy, RuleType,
};
use std::collections::HashMap;

#[tokio::test]
async fn test_policy_shadowing_and_inheritance() {
    let mut engine = GovernanceEngine::new();

    let company_policy = Policy {
        id: "security".to_string(),
        name: "Security".to_string(),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Mandatory,
        merge_strategy: RuleMergeStrategy::Merge,
        rules: vec![PolicyRule {
            id: "no-secrets".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustNotMatch,
            value: serde_json::json!("SECRET_.*"),
            severity: ConstraintSeverity::Block,
            message: "No secrets allowed".to_string(),
        }],
        metadata: HashMap::new(),
    };

    let org_policy = Policy {
        id: "coding-style".to_string(),
        name: "Style".to_string(),
        description: None,
        layer: KnowledgeLayer::Org,
        mode: PolicyMode::Optional,
        merge_strategy: RuleMergeStrategy::Override,
        rules: vec![PolicyRule {
            id: "indentation".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustMatch,
            value: serde_json::json!(r"^  \S"),
            severity: ConstraintSeverity::Warn,
            message: "Use 2 spaces".to_string(),
        }],
        metadata: HashMap::new(),
    };

    engine.add_policy(company_policy);
    engine.add_policy(org_policy);

    let project_id = "proj-1";
    let mut context = HashMap::new();
    context.insert("projectId".to_string(), serde_json::json!(project_id));
    context.insert("content".to_string(), serde_json::json!("    fn main() {}"));

    let result = engine.validate(KnowledgeLayer::Project, &context);
    assert!(!result.is_valid);

    let project_policy = Policy {
        id: "coding-style".to_string(),
        name: "Project Style".to_string(),
        description: None,
        layer: KnowledgeLayer::Project,
        mode: PolicyMode::Optional,
        merge_strategy: RuleMergeStrategy::Override,
        rules: vec![PolicyRule {
            id: "indentation".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustMatch,
            value: serde_json::json!(r"^    \S"),
            severity: ConstraintSeverity::Warn,
            message: "Use 4 spaces".to_string(),
        }],
        metadata: HashMap::new(),
    };

    engine.add_policy(project_policy);

    let result = engine.validate(KnowledgeLayer::Project, &context);
    assert!(
        result.is_valid,
        "Project policy should have overridden Org policy"
    );

    let project_security_override = Policy {
        id: "security".to_string(),
        name: "Insecure Project".to_string(),
        description: None,
        layer: KnowledgeLayer::Project,
        mode: PolicyMode::Optional,
        merge_strategy: RuleMergeStrategy::Override,
        rules: vec![],
        metadata: HashMap::new(),
    };

    engine.add_policy(project_security_override);

    let mut context_with_secret = HashMap::new();
    context_with_secret.insert("projectId".to_string(), serde_json::json!(project_id));
    context_with_secret.insert("content".to_string(), serde_json::json!("SECRET_KEY=123"));

    let result = engine.validate(KnowledgeLayer::Project, &context_with_secret);
    assert!(
        !result.is_valid,
        "Tenant mandatory policy should NOT be overridable"
    );
}

#[tokio::test]
async fn test_rule_type_deny_precedence() {
    let mut engine = GovernanceEngine::new();

    let company_policy = Policy {
        id: "lib-checks".to_string(),
        name: "Lib Checks".to_string(),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Optional,
        merge_strategy: RuleMergeStrategy::Merge,
        rules: vec![PolicyRule {
            id: "deny-jquery".to_string(),
            rule_type: RuleType::Deny,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustUse,
            value: serde_json::json!("jquery"),
            severity: ConstraintSeverity::Block,
            message: "JQuery is forbidden".to_string(),
        }],
        metadata: HashMap::new(),
    };

    engine.add_policy(company_policy);

    let mut context = HashMap::new();
    context.insert("dependencies".to_string(), serde_json::json!(["jquery"]));

    let result = engine.validate(KnowledgeLayer::Project, &context);
    assert!(!result.is_valid);
    assert_eq!(result.violations[0].rule_id, "deny-jquery");
}

#[tokio::test]
async fn test_layer_hierarchy_order() {
    let mut engine = GovernanceEngine::new();

    let company_policy = Policy {
        id: "layer-test".to_string(),
        name: "Tenant Layer Test".to_string(),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Optional,
        merge_strategy: RuleMergeStrategy::Override,
        rules: vec![PolicyRule {
            id: "layer-rule".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustMatch,
            value: serde_json::json!("tenant"),
            severity: ConstraintSeverity::Warn,
            message: "Must contain tenant".to_string(),
        }],
        metadata: HashMap::new(),
    };

    let org_policy = Policy {
        id: "layer-test".to_string(),
        name: "Org Layer Test".to_string(),
        description: None,
        layer: KnowledgeLayer::Org,
        mode: PolicyMode::Optional,
        merge_strategy: RuleMergeStrategy::Override,
        rules: vec![PolicyRule {
            id: "layer-rule".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustMatch,
            value: serde_json::json!("org"),
            severity: ConstraintSeverity::Warn,
            message: "Must contain org".to_string(),
        }],
        metadata: HashMap::new(),
    };

    engine.add_policy(company_policy);
    engine.add_policy(org_policy);

    let mut context = HashMap::new();
    context.insert("content".to_string(), serde_json::json!("org content here"));

    let result = engine.validate(KnowledgeLayer::Org, &context);
    assert!(
        result.is_valid,
        "Org policy should override tenant for Org layer validation"
    );

    let mut context_company = HashMap::new();
    context_company.insert(
        "content".to_string(),
        serde_json::json!("tenant content here"),
    );
    let result_company = engine.validate(KnowledgeLayer::Tenant, &context_company);
    assert!(
        result_company.is_valid,
        "Tenant layer validation should only use tenant policies"
    );
}

#[tokio::test]
async fn test_team_layer_inheritance() {
    let mut engine = GovernanceEngine::new();

    let company_policy = Policy {
        id: "tenant-rule".to_string(),
        name: "Tenant Rule".to_string(),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Mandatory,
        merge_strategy: RuleMergeStrategy::Merge,
        rules: vec![PolicyRule {
            id: "tenant-check".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustNotMatch,
            value: serde_json::json!("FORBIDDEN"),
            severity: ConstraintSeverity::Block,
            message: "FORBIDDEN not allowed".to_string(),
        }],
        metadata: HashMap::new(),
    };

    let team_policy = Policy {
        id: "team-rule".to_string(),
        name: "Team Rule".to_string(),
        description: None,
        layer: KnowledgeLayer::Team,
        mode: PolicyMode::Optional,
        merge_strategy: RuleMergeStrategy::Merge,
        rules: vec![PolicyRule {
            id: "team-check".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustMatch,
            value: serde_json::json!("team_approved"),
            severity: ConstraintSeverity::Warn,
            message: "Team approval marker needed".to_string(),
        }],
        metadata: HashMap::new(),
    };

    engine.add_policy(company_policy);
    engine.add_policy(team_policy);

    let mut valid_context = HashMap::new();
    valid_context.insert(
        "content".to_string(),
        serde_json::json!("team_approved content"),
    );
    let result = engine.validate(KnowledgeLayer::Team, &valid_context);
    assert!(
        result.is_valid,
        "Content meeting both tenant and team requirements should pass"
    );

    let mut forbidden_context = HashMap::new();
    forbidden_context.insert(
        "content".to_string(),
        serde_json::json!("team_approved FORBIDDEN content"),
    );
    let result = engine.validate(KnowledgeLayer::Team, &forbidden_context);
    assert!(
        !result.is_valid,
        "Tenant mandatory policy should still apply at team layer"
    );

    let mut missing_marker_context = HashMap::new();
    missing_marker_context.insert("content".to_string(), serde_json::json!("no marker here"));
    let result = engine.validate(KnowledgeLayer::Team, &missing_marker_context);
    assert!(
        !result.is_valid,
        "Team policy should be enforced at team layer"
    );
}

#[tokio::test]
async fn test_merge_strategy_merge_accumulates_rules() {
    let mut engine = GovernanceEngine::new();

    let company_policy = Policy {
        id: "multi-rule".to_string(),
        name: "Multi Rule".to_string(),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Optional,
        merge_strategy: RuleMergeStrategy::Merge,
        rules: vec![PolicyRule {
            id: "rule-1".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustNotMatch,
            value: serde_json::json!("RULE1_VIOLATION"),
            severity: ConstraintSeverity::Block,
            message: "Rule 1 violated".to_string(),
        }],
        metadata: HashMap::new(),
    };

    let org_policy = Policy {
        id: "multi-rule".to_string(),
        name: "Multi Rule Org".to_string(),
        description: None,
        layer: KnowledgeLayer::Org,
        mode: PolicyMode::Optional,
        merge_strategy: RuleMergeStrategy::Merge,
        rules: vec![PolicyRule {
            id: "rule-2".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustNotMatch,
            value: serde_json::json!("RULE2_VIOLATION"),
            severity: ConstraintSeverity::Block,
            message: "Rule 2 violated".to_string(),
        }],
        metadata: HashMap::new(),
    };

    engine.add_policy(company_policy);
    engine.add_policy(org_policy);

    let mut context_rule1 = HashMap::new();
    context_rule1.insert("content".to_string(), serde_json::json!("RULE1_VIOLATION"));
    let result = engine.validate(KnowledgeLayer::Org, &context_rule1);
    assert!(
        !result.is_valid,
        "Rule 1 from tenant should still apply after merge"
    );

    let mut context_rule2 = HashMap::new();
    context_rule2.insert("content".to_string(), serde_json::json!("RULE2_VIOLATION"));
    let result = engine.validate(KnowledgeLayer::Org, &context_rule2);
    assert!(
        !result.is_valid,
        "Rule 2 from org should be added via merge"
    );

    let mut context_both = HashMap::new();
    context_both.insert(
        "content".to_string(),
        serde_json::json!("RULE1_VIOLATION RULE2_VIOLATION"),
    );
    let result = engine.validate(KnowledgeLayer::Org, &context_both);
    assert!(!result.is_valid);
    assert!(
        !result.violations.is_empty(),
        "Both rules should be evaluated"
    );
}

#[tokio::test]
async fn test_severity_levels_in_hierarchy() {
    let mut engine = GovernanceEngine::new();

    let block_policy = Policy {
        id: "severity-test".to_string(),
        name: "Severity Test".to_string(),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Optional,
        merge_strategy: RuleMergeStrategy::Merge,
        rules: vec![
            PolicyRule {
                id: "block-rule".to_string(),
                rule_type: RuleType::Allow,
                target: ConstraintTarget::Code,
                operator: ConstraintOperator::MustNotMatch,
                value: serde_json::json!("CRITICAL_ERROR"),
                severity: ConstraintSeverity::Block,
                message: "Critical error found".to_string(),
            },
            PolicyRule {
                id: "warn-rule".to_string(),
                rule_type: RuleType::Allow,
                target: ConstraintTarget::Code,
                operator: ConstraintOperator::MustNotMatch,
                value: serde_json::json!("MINOR_ISSUE"),
                severity: ConstraintSeverity::Warn,
                message: "Minor issue found".to_string(),
            },
            PolicyRule {
                id: "info-rule".to_string(),
                rule_type: RuleType::Allow,
                target: ConstraintTarget::Code,
                operator: ConstraintOperator::MustNotMatch,
                value: serde_json::json!("NOTE_THIS"),
                severity: ConstraintSeverity::Info,
                message: "Note for review".to_string(),
            },
        ],
        metadata: HashMap::new(),
    };

    engine.add_policy(block_policy);

    let mut context_block = HashMap::new();
    context_block.insert(
        "content".to_string(),
        serde_json::json!("CRITICAL_ERROR here"),
    );
    let result = engine.validate(KnowledgeLayer::Project, &context_block);
    assert!(!result.is_valid);
    assert_eq!(result.violations[0].severity, ConstraintSeverity::Block);

    let mut context_warn = HashMap::new();
    context_warn.insert("content".to_string(), serde_json::json!("MINOR_ISSUE here"));
    let result = engine.validate(KnowledgeLayer::Project, &context_warn);
    assert!(!result.is_valid);
    assert_eq!(result.violations[0].severity, ConstraintSeverity::Warn);

    let mut context_info = HashMap::new();
    context_info.insert("content".to_string(), serde_json::json!("NOTE_THIS here"));
    let result = engine.validate(KnowledgeLayer::Project, &context_info);
    assert!(!result.is_valid);
    assert_eq!(result.violations[0].severity, ConstraintSeverity::Info);
}

#[tokio::test]
async fn test_empty_policy_layers_are_skipped() {
    let mut engine = GovernanceEngine::new();

    let project_policy = Policy {
        id: "project-only".to_string(),
        name: "Project Only".to_string(),
        description: None,
        layer: KnowledgeLayer::Project,
        mode: PolicyMode::Optional,
        merge_strategy: RuleMergeStrategy::Merge,
        rules: vec![PolicyRule {
            id: "project-rule".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustMatch,
            value: serde_json::json!("project_marker"),
            severity: ConstraintSeverity::Warn,
            message: "Project marker needed".to_string(),
        }],
        metadata: HashMap::new(),
    };

    engine.add_policy(project_policy);

    let mut context = HashMap::new();
    context.insert("content".to_string(), serde_json::json!("no marker"));
    let result = engine.validate(KnowledgeLayer::Org, &context);
    assert!(
        result.is_valid,
        "Project policy should not apply at Org layer"
    );

    let result = engine.validate(KnowledgeLayer::Project, &context);
    assert!(
        !result.is_valid,
        "Project policy should apply at Project layer"
    );
}

#[tokio::test]
async fn test_multiple_policies_same_layer() {
    let mut engine = GovernanceEngine::new();

    let policy_a = Policy {
        id: "policy-a".to_string(),
        name: "Policy A".to_string(),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Optional,
        merge_strategy: RuleMergeStrategy::Merge,
        rules: vec![PolicyRule {
            id: "rule-a".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustNotMatch,
            value: serde_json::json!("VIOLATION_A"),
            severity: ConstraintSeverity::Block,
            message: "Violation A".to_string(),
        }],
        metadata: HashMap::new(),
    };

    let policy_b = Policy {
        id: "policy-b".to_string(),
        name: "Policy B".to_string(),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Optional,
        merge_strategy: RuleMergeStrategy::Merge,
        rules: vec![PolicyRule {
            id: "rule-b".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustNotMatch,
            value: serde_json::json!("VIOLATION_B"),
            severity: ConstraintSeverity::Block,
            message: "Violation B".to_string(),
        }],
        metadata: HashMap::new(),
    };

    engine.add_policy(policy_a);
    engine.add_policy(policy_b);

    let mut context = HashMap::new();
    context.insert(
        "content".to_string(),
        serde_json::json!("VIOLATION_A VIOLATION_B"),
    );
    let result = engine.validate(KnowledgeLayer::Project, &context);
    assert!(!result.is_valid);
    assert_eq!(
        result.violations.len(),
        2,
        "Both policies should generate violations"
    );
}

#[tokio::test]
async fn test_dependency_constraint_inheritance() {
    let mut engine = GovernanceEngine::new();

    let company_policy = Policy {
        id: "dependency-policy".to_string(),
        name: "Dependency Policy".to_string(),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Mandatory,
        merge_strategy: RuleMergeStrategy::Merge,
        rules: vec![PolicyRule {
            id: "require-security-lib".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustUse,
            value: serde_json::json!("security-lib"),
            severity: ConstraintSeverity::Block,
            message: "Security library required".to_string(),
        }],
        metadata: HashMap::new(),
    };

    let org_policy = Policy {
        id: "dependency-policy".to_string(),
        name: "Org Dependency Policy".to_string(),
        description: None,
        layer: KnowledgeLayer::Org,
        mode: PolicyMode::Optional,
        merge_strategy: RuleMergeStrategy::Merge,
        rules: vec![PolicyRule {
            id: "require-logging-lib".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustUse,
            value: serde_json::json!("logging-lib"),
            severity: ConstraintSeverity::Warn,
            message: "Logging library recommended".to_string(),
        }],
        metadata: HashMap::new(),
    };

    engine.add_policy(company_policy);
    engine.add_policy(org_policy);

    let mut context_missing_security = HashMap::new();
    context_missing_security.insert(
        "dependencies".to_string(),
        serde_json::json!(["logging-lib", "other-lib"]),
    );
    let result = engine.validate(KnowledgeLayer::Project, &context_missing_security);
    assert!(
        !result.is_valid,
        "Missing mandatory security-lib should fail"
    );

    let mut context_has_both = HashMap::new();
    context_has_both.insert(
        "dependencies".to_string(),
        serde_json::json!(["security-lib", "logging-lib"]),
    );
    let result = engine.validate(KnowledgeLayer::Project, &context_has_both);
    assert!(result.is_valid, "Having both libs should pass");
}

#[tokio::test]
async fn test_config_constraint_target() {
    let mut engine = GovernanceEngine::new();

    // Test MustExist: config key must exist in context
    let config_policy = Policy {
        id: "config-policy".to_string(),
        name: "Config Policy".to_string(),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Mandatory,
        merge_strategy: RuleMergeStrategy::Merge,
        rules: vec![PolicyRule {
            id: "require-config".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Config,
            operator: ConstraintOperator::MustExist,
            value: serde_json::json!(null), // MustExist checks key presence, value not used
            severity: ConstraintSeverity::Block,
            message: "Config section required".to_string(),
        }],
        metadata: HashMap::new(),
    };

    engine.add_policy(config_policy);

    // Context without "config" key should fail
    let context_without_config: HashMap<String, serde_json::Value> = HashMap::new();
    let result = engine.validate(KnowledgeLayer::Project, &context_without_config);
    assert!(!result.is_valid, "Missing config key should fail");

    // Context with "config" key should pass
    let mut context_with_config = HashMap::new();
    context_with_config.insert("config".to_string(), serde_json::json!({"any": "value"}));
    let result = engine.validate(KnowledgeLayer::Project, &context_with_config);
    assert!(result.is_valid, "Config key present should pass");

    // Test MustNotExist: config key must NOT exist
    let mut engine2 = GovernanceEngine::new();
    let no_config_policy = Policy {
        id: "no-config-policy".to_string(),
        name: "No Config Policy".to_string(),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Mandatory,
        merge_strategy: RuleMergeStrategy::Merge,
        rules: vec![PolicyRule {
            id: "forbid-config".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Config,
            operator: ConstraintOperator::MustNotExist,
            value: serde_json::json!(null),
            severity: ConstraintSeverity::Block,
            message: "Config section forbidden".to_string(),
        }],
        metadata: HashMap::new(),
    };

    engine2.add_policy(no_config_policy);

    // Context with "config" key should fail for MustNotExist
    let result = engine2.validate(KnowledgeLayer::Project, &context_with_config);
    assert!(
        !result.is_valid,
        "Config key present should fail with MustNotExist"
    );

    // Context without "config" key should pass for MustNotExist
    let result = engine2.validate(KnowledgeLayer::Project, &context_without_config);
    assert!(
        result.is_valid,
        "Missing config key should pass with MustNotExist"
    );
}
