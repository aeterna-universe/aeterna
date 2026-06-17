//! Tests for drift detection accuracy in the GovernanceEngine.
//!
//! These tests verify:
//! - Drift score calculation based on violation severity
//! - Missing mandatory policies detection
//! - Stale policy version detection
//! - Semantic contradiction detection
//! - Edge cases and boundary conditions

use knowledge::governance::GovernanceEngine;
use mk_core::types::{
    ConstraintOperator, ConstraintSeverity, ConstraintTarget, KnowledgeLayer, Policy, PolicyMode,
    PolicyRule, RuleMergeStrategy, RuleType, TenantContext, TenantId, UserId,
};
use std::collections::HashMap;
use std::sync::Arc;
use storage::postgres::PostgresBackend;
use testing::{postgres, unique_id};
use tokio::sync::OnceCell;

static SCHEMA_INITIALIZED: OnceCell<bool> = OnceCell::const_new();

async fn create_test_backend() -> Option<PostgresBackend> {
    let fixture = postgres().await?;
    let backend = PostgresBackend::new(fixture.url()).await.ok()?;

    SCHEMA_INITIALIZED
        .get_or_init(|| async {
            backend.initialize_schema().await.ok();
            true
        })
        .await;

    Some(backend)
}

fn create_test_context() -> TenantContext {
    TenantContext::new(
        TenantId::new("test-tenant".to_string()).unwrap(),
        UserId::new("test-user".to_string()).unwrap(),
    )
}

fn create_mandatory_policy(id: &str, rules: Vec<PolicyRule>) -> Policy {
    Policy {
        id: id.to_string(),
        name: format!("Policy {}", id),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Mandatory,
        merge_strategy: RuleMergeStrategy::Override,
        rules,
        metadata: HashMap::new(),
    }
}

fn create_advisory_policy(id: &str, rules: Vec<PolicyRule>) -> Policy {
    Policy {
        id: id.to_string(),
        name: format!("Policy {}", id),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Optional,
        merge_strategy: RuleMergeStrategy::Override,
        rules,
        metadata: HashMap::new(),
    }
}

#[tokio::test]
async fn test_drift_score_zero_when_no_violations() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let policy = create_mandatory_policy(
        "valid-policy",
        vec![PolicyRule {
            id: "rule-1".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::File,
            operator: ConstraintOperator::MustExist,
            value: serde_json::json!(null),
            severity: ConstraintSeverity::Block,
            message: "File must exist".to_string(),
        }],
    );
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("path".to_string(), serde_json::json!("/src/main.rs"));

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert_eq!(
        drift_score, 0.0,
        "No violations should yield drift score of 0.0"
    );
}

#[tokio::test]
async fn test_drift_score_block_severity_yields_one() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let policy = create_mandatory_policy(
        "blocking-policy",
        vec![PolicyRule {
            id: "block-rule".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustUse,
            value: serde_json::json!("required-lib"),
            severity: ConstraintSeverity::Block,
            message: "Required library missing".to_string(),
        }],
    );
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("dependencies".to_string(), serde_json::json!(["other-lib"]));

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert_eq!(
        drift_score, 1.0,
        "Block severity violation should yield drift score of 1.0"
    );
}

#[tokio::test]
async fn test_drift_score_warn_severity_yields_half() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let policy = create_mandatory_policy(
        "warning-policy",
        vec![PolicyRule {
            id: "warn-rule".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustUse,
            value: serde_json::json!("recommended-lib"),
            severity: ConstraintSeverity::Warn,
            message: "Recommended library missing".to_string(),
        }],
    );
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("dependencies".to_string(), serde_json::json!(["other-lib"]));

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert_eq!(
        drift_score, 0.5,
        "Warn severity violation should yield drift score of 0.5"
    );
}

#[tokio::test]
async fn test_drift_score_info_severity_yields_point_one() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let policy = create_mandatory_policy(
        "info-policy",
        vec![PolicyRule {
            id: "info-rule".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustUse,
            value: serde_json::json!("optional-lib"),
            severity: ConstraintSeverity::Info,
            message: "Optional library missing".to_string(),
        }],
    );
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("dependencies".to_string(), serde_json::json!(["other-lib"]));

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert!(
        (drift_score - 0.1).abs() < 0.01,
        "Info severity violation should yield drift score of ~0.1"
    );
}

#[tokio::test]
async fn test_drift_score_capped_at_one() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let policy = create_mandatory_policy(
        "multi-violation-policy",
        vec![
            PolicyRule {
                id: "block-1".to_string(),
                rule_type: RuleType::Allow,
                target: ConstraintTarget::Dependency,
                operator: ConstraintOperator::MustUse,
                value: serde_json::json!("lib-1"),
                severity: ConstraintSeverity::Block,
                message: "Missing lib-1".to_string(),
            },
            PolicyRule {
                id: "block-2".to_string(),
                rule_type: RuleType::Allow,
                target: ConstraintTarget::Dependency,
                operator: ConstraintOperator::MustUse,
                value: serde_json::json!("lib-2"),
                severity: ConstraintSeverity::Block,
                message: "Missing lib-2".to_string(),
            },
            PolicyRule {
                id: "block-3".to_string(),
                rule_type: RuleType::Allow,
                target: ConstraintTarget::Dependency,
                operator: ConstraintOperator::MustUse,
                value: serde_json::json!("lib-3"),
                severity: ConstraintSeverity::Block,
                message: "Missing lib-3".to_string(),
            },
        ],
    );
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("dependencies".to_string(), serde_json::json!([]));

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert_eq!(
        drift_score, 1.0,
        "Drift score should be capped at 1.0 even with multiple Block violations"
    );
}

#[tokio::test]
async fn test_drift_score_mixed_severities() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let policy = create_mandatory_policy(
        "mixed-policy",
        vec![
            PolicyRule {
                id: "warn-rule".to_string(),
                rule_type: RuleType::Allow,
                target: ConstraintTarget::Dependency,
                operator: ConstraintOperator::MustUse,
                value: serde_json::json!("warn-lib"),
                severity: ConstraintSeverity::Warn,
                message: "Missing warn-lib".to_string(),
            },
            PolicyRule {
                id: "info-rule".to_string(),
                rule_type: RuleType::Allow,
                target: ConstraintTarget::Dependency,
                operator: ConstraintOperator::MustUse,
                value: serde_json::json!("info-lib"),
                severity: ConstraintSeverity::Info,
                message: "Missing info-lib".to_string(),
            },
        ],
    );
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("dependencies".to_string(), serde_json::json!([]));

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert!(
        (drift_score - 0.6).abs() < 0.01,
        "Warn (0.5) + Info (0.1) should yield ~0.6"
    );
}

#[tokio::test]
async fn test_missing_mandatory_policies_detection() {
    let engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let context: HashMap<String, serde_json::Value> = HashMap::new();
    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();

    assert!(
        drift_score > 0.0,
        "Missing mandatory policies should trigger drift"
    );
    assert!(
        drift_score <= 0.5,
        "Missing mandatory policies is a Warn severity (0.5)"
    );
}

#[tokio::test]
async fn test_stale_policy_version_detection() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let mut metadata = HashMap::new();
    metadata.insert("version_hash".to_string(), serde_json::json!("v2.0.0"));

    let policy = Policy {
        id: "versioned-policy".to_string(),
        name: "Versioned Policy".to_string(),
        description: None,
        layer: KnowledgeLayer::Project,
        mode: PolicyMode::Mandatory,
        merge_strategy: RuleMergeStrategy::Override,
        rules: vec![],
        metadata,
    };
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("version_hash".to_string(), serde_json::json!("v1.0.0"));

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert!(
        drift_score > 0.0,
        "Stale policy version should trigger drift"
    );
}

#[tokio::test]
async fn test_no_stale_policy_when_versions_match() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let mut metadata = HashMap::new();
    metadata.insert("version_hash".to_string(), serde_json::json!("v2.0.0"));

    let policy = Policy {
        id: "versioned-policy".to_string(),
        name: "Versioned Policy".to_string(),
        description: None,
        layer: KnowledgeLayer::Project,
        mode: PolicyMode::Mandatory,
        merge_strategy: RuleMergeStrategy::Override,
        rules: vec![],
        metadata,
    };
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("version_hash".to_string(), serde_json::json!("v2.0.0"));

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert_eq!(
        drift_score, 0.0,
        "Matching versions should not trigger drift"
    );
}

#[tokio::test]
async fn test_advisory_policies_dont_count_as_mandatory() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let policy = create_advisory_policy(
        "advisory-only",
        vec![PolicyRule {
            id: "advisory-rule".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::File,
            operator: ConstraintOperator::MustExist,
            value: serde_json::json!(null),
            severity: ConstraintSeverity::Info,
            message: "Advisory check".to_string(),
        }],
    );
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("path".to_string(), serde_json::json!("/src/main.rs"));

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert!(
        drift_score > 0.0,
        "Only advisory policies should still trigger missing mandatory drift"
    );
}

#[tokio::test]
async fn test_multiple_policies_accumulate_violations() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let policy1 = create_mandatory_policy(
        "policy-1",
        vec![PolicyRule {
            id: "rule-1".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustUse,
            value: serde_json::json!("lib-a"),
            severity: ConstraintSeverity::Info,
            message: "Missing lib-a".to_string(),
        }],
    );

    let policy2 = create_mandatory_policy(
        "policy-2",
        vec![PolicyRule {
            id: "rule-2".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustUse,
            value: serde_json::json!("lib-b"),
            severity: ConstraintSeverity::Info,
            message: "Missing lib-b".to_string(),
        }],
    );

    engine.add_policy(policy1);
    engine.add_policy(policy2);

    let mut context = HashMap::new();
    context.insert("dependencies".to_string(), serde_json::json!([]));

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert!(
        (drift_score - 0.2).abs() < 0.01,
        "Two Info violations should yield ~0.2"
    );
}

#[tokio::test]
async fn test_deny_rule_violation_detection() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let policy = create_mandatory_policy(
        "deny-policy",
        vec![PolicyRule {
            id: "deny-rule".to_string(),
            rule_type: RuleType::Deny,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustUse,
            value: serde_json::json!("forbidden-lib"),
            severity: ConstraintSeverity::Block,
            message: "Forbidden library detected".to_string(),
        }],
    );
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert(
        "dependencies".to_string(),
        serde_json::json!(["forbidden-lib", "good-lib"]),
    );

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert_eq!(
        drift_score, 1.0,
        "Using forbidden dependency should yield max drift"
    );
}

#[tokio::test]
async fn test_empty_context_with_mandatory_policies() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let policy = create_mandatory_policy(
        "requires-deps",
        vec![PolicyRule {
            id: "check-deps".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustExist,
            value: serde_json::json!(null),
            severity: ConstraintSeverity::Block,
            message: "Dependencies must be declared".to_string(),
        }],
    );
    engine.add_policy(policy);

    let context: HashMap<String, serde_json::Value> = HashMap::new();

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert_eq!(
        drift_score, 1.0,
        "Missing required dependency key should yield max drift"
    );
}

#[tokio::test]
async fn test_drift_with_must_not_use_satisfied() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let policy = create_mandatory_policy(
        "no-bad-libs",
        vec![PolicyRule {
            id: "no-bad".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustNotUse,
            value: serde_json::json!("bad-lib"),
            severity: ConstraintSeverity::Block,
            message: "Bad library forbidden".to_string(),
        }],
    );
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("dependencies".to_string(), serde_json::json!(["good-lib"]));

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert_eq!(
        drift_score, 0.0,
        "Not using forbidden lib should yield zero drift"
    );
}

#[tokio::test]
async fn test_drift_with_must_not_use_violated() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let policy = create_mandatory_policy(
        "no-bad-libs",
        vec![PolicyRule {
            id: "no-bad".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustNotUse,
            value: serde_json::json!("bad-lib"),
            severity: ConstraintSeverity::Block,
            message: "Bad library forbidden".to_string(),
        }],
    );
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert(
        "dependencies".to_string(),
        serde_json::json!(["good-lib", "bad-lib"]),
    );

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert_eq!(
        drift_score, 1.0,
        "Using forbidden lib should yield max drift"
    );
}

#[tokio::test]
async fn test_drift_with_regex_match_satisfied() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let policy = create_mandatory_policy(
        "naming-policy",
        vec![PolicyRule {
            id: "name-pattern".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::File,
            operator: ConstraintOperator::MustMatch,
            value: serde_json::json!(r"^[a-z_]+\.rs$"),
            severity: ConstraintSeverity::Block,
            message: "File must match naming convention".to_string(),
        }],
    );
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("path".to_string(), serde_json::json!("my_module.rs"));

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert_eq!(drift_score, 0.0, "Matching pattern should yield zero drift");
}

#[tokio::test]
async fn test_drift_with_regex_match_violated() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let policy = create_mandatory_policy(
        "naming-policy",
        vec![PolicyRule {
            id: "name-pattern".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::File,
            operator: ConstraintOperator::MustMatch,
            value: serde_json::json!(r"^[a-z_]+\.rs$"),
            severity: ConstraintSeverity::Block,
            message: "File must match naming convention".to_string(),
        }],
    );
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("path".to_string(), serde_json::json!("MyModule.rs"));

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert_eq!(
        drift_score, 1.0,
        "Non-matching pattern should yield max drift"
    );
}

#[tokio::test]
async fn test_drift_with_must_not_match_satisfied() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let policy = create_mandatory_policy(
        "no-test-files",
        vec![PolicyRule {
            id: "no-test".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::File,
            operator: ConstraintOperator::MustNotMatch,
            value: serde_json::json!(r"_test\.rs$"),
            severity: ConstraintSeverity::Block,
            message: "Test files not allowed in src".to_string(),
        }],
    );
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("path".to_string(), serde_json::json!("main.rs"));

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert_eq!(drift_score, 0.0, "Non-test file should yield zero drift");
}

#[tokio::test]
async fn test_drift_with_must_not_match_violated() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let policy = create_mandatory_policy(
        "no-test-files",
        vec![PolicyRule {
            id: "no-test".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::File,
            operator: ConstraintOperator::MustNotMatch,
            value: serde_json::json!(r"_test\.rs$"),
            severity: ConstraintSeverity::Block,
            message: "Test files not allowed in src".to_string(),
        }],
    );
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("path".to_string(), serde_json::json!("main_test.rs"));

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert_eq!(drift_score, 1.0, "Test file in src should yield max drift");
}

#[tokio::test]
async fn test_drift_layer_filtering() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let company_policy = Policy {
        id: "tenant-policy".to_string(),
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
            value: serde_json::json!("tenant-lib"),
            severity: ConstraintSeverity::Block,
            message: "Tenant lib required".to_string(),
        }],
        metadata: HashMap::new(),
    };

    let project_policy = Policy {
        id: "project-policy".to_string(),
        name: "Project Policy".to_string(),
        description: None,
        layer: KnowledgeLayer::Project,
        mode: PolicyMode::Mandatory,
        merge_strategy: RuleMergeStrategy::Override,
        rules: vec![PolicyRule {
            id: "project-rule".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustUse,
            value: serde_json::json!("project-lib"),
            severity: ConstraintSeverity::Warn,
            message: "Project lib required".to_string(),
        }],
        metadata: HashMap::new(),
    };

    engine.add_policy(company_policy);
    engine.add_policy(project_policy);

    let mut context = HashMap::new();
    context.insert(
        "dependencies".to_string(),
        serde_json::json!(["tenant-lib"]),
    );

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert!(
        drift_score > 0.0,
        "Missing project-lib should trigger drift"
    );
    assert!(
        drift_score <= 0.5,
        "Only project-lib missing (Warn) should yield <= 0.5"
    );
}

#[tokio::test]
async fn test_drift_idempotent_check() {
    let mut engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let policy = create_mandatory_policy(
        "idempotent-test",
        vec![PolicyRule {
            id: "rule-1".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustUse,
            value: serde_json::json!("required-lib"),
            severity: ConstraintSeverity::Warn,
            message: "Required lib missing".to_string(),
        }],
    );
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("dependencies".to_string(), serde_json::json!(["other-lib"]));

    let score1 = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    let score2 = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    let score3 = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();

    assert_eq!(score1, score2, "Drift check should be idempotent");
    assert_eq!(score2, score3, "Drift check should be idempotent");
}

#[tokio::test]
async fn test_llm_enhanced_drift_detects_semantic_violations() {
    let mock_llm = Arc::new(memory::llm::mock::MockLlmService::new());

    let engine = GovernanceEngine::new().with_llm_service(mock_llm);
    let ctx = create_test_context();

    let mut context = HashMap::new();
    context.insert(
        "content".to_string(),
        serde_json::json!("This code violate:security-rule violates security practices"),
    );

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();

    assert!(
        drift_score > 0.0,
        "LLM should detect semantic violations when rule marker present"
    );
}

#[tokio::test]
async fn test_llm_enhanced_drift_no_false_positives() {
    let mock_llm = Arc::new(memory::llm::mock::MockLlmService::new());

    let engine = GovernanceEngine::new().with_llm_service(mock_llm);
    let ctx = create_test_context();

    let mut context = HashMap::new();
    context.insert(
        "content".to_string(),
        serde_json::json!("This is perfectly compliant code with no issues"),
    );

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();

    assert!(
        drift_score <= 0.5,
        "LLM should not create false positives for compliant content"
    );
}

#[tokio::test]
async fn test_llm_violations_prefixed_with_llm_marker() {
    let mock_llm = Arc::new(memory::llm::mock::MockLlmService::new());

    let mut engine = GovernanceEngine::new().with_llm_service(mock_llm);
    let ctx = create_test_context();

    let policy = create_mandatory_policy(
        "semantic-policy",
        vec![PolicyRule {
            id: "semantic-check".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustMatch,
            value: serde_json::json!(".*"),
            severity: ConstraintSeverity::Warn,
            message: "Semantic policy".to_string(),
        }],
    );
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert(
        "content".to_string(),
        serde_json::json!("Code that violate:semantic-check triggers LLM analysis"),
    );

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert!(drift_score > 0.0, "Should detect LLM violation");
}

#[tokio::test]
async fn test_llm_graceful_degradation_without_service() {
    let engine = GovernanceEngine::new();
    let ctx = create_test_context();

    let mut context = HashMap::new();
    context.insert(
        "content".to_string(),
        serde_json::json!("Content that would trigger LLM analysis if available"),
    );

    let result = engine.check_drift(&ctx, "project-1", &context).await;
    assert!(result.is_ok(), "Should work without LLM service configured");
}

#[tokio::test]
async fn test_llm_combined_with_rule_based_violations() {
    let mock_llm = Arc::new(memory::llm::mock::MockLlmService::new());

    let mut engine = GovernanceEngine::new().with_llm_service(mock_llm);
    let ctx = create_test_context();

    let policy = create_mandatory_policy(
        "combined-policy",
        vec![
            PolicyRule {
                id: "rule-based".to_string(),
                rule_type: RuleType::Allow,
                target: ConstraintTarget::Dependency,
                operator: ConstraintOperator::MustUse,
                value: serde_json::json!("required-lib"),
                severity: ConstraintSeverity::Warn,
                message: "Required lib missing".to_string(),
            },
            PolicyRule {
                id: "llm-checked".to_string(),
                rule_type: RuleType::Allow,
                target: ConstraintTarget::Code,
                operator: ConstraintOperator::MustMatch,
                value: serde_json::json!(".*"),
                severity: ConstraintSeverity::Info,
                message: "LLM semantic check".to_string(),
            },
        ],
    );
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("dependencies".to_string(), serde_json::json!(["other-lib"]));
    context.insert(
        "content".to_string(),
        serde_json::json!("Code that violate:llm-checked"),
    );

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert!(
        drift_score >= 0.6,
        "Should combine rule-based (0.5) + LLM (0.1) violations"
    );
}

#[tokio::test]
async fn test_llm_does_not_duplicate_existing_violations() {
    let mock_llm = Arc::new(memory::llm::mock::MockLlmService::new());

    let mut engine = GovernanceEngine::new().with_llm_service(mock_llm);
    let ctx = create_test_context();

    let policy = create_mandatory_policy(
        "dup-test-policy",
        vec![PolicyRule {
            id: "shared-rule".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustUse,
            value: serde_json::json!("must-have-lib"),
            severity: ConstraintSeverity::Block,
            message: "Must have lib".to_string(),
        }],
    );
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("dependencies".to_string(), serde_json::json!([]));
    context.insert(
        "content".to_string(),
        serde_json::json!("Content that triggers violate:shared-rule"),
    );

    let drift_score = engine
        .check_drift(&ctx, "project-1", &context)
        .await
        .unwrap();
    assert_eq!(
        drift_score, 1.0,
        "Score should be capped at 1.0, not doubled"
    );
}

#[tokio::test]
async fn test_drift_auto_suppress_info_filters_info_violations() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let test_id = unique_id("drift");
    let tenant_id = TenantId::new(format!("tenant-auto-suppress-{}", test_id)).unwrap();
    let ctx = TenantContext::new(
        tenant_id.clone(),
        UserId::new("user-1".to_string()).unwrap(),
    );

    let drift_config = mk_core::types::DriftConfig {
        project_id: format!("proj-auto-suppress-{}", test_id),
        tenant_id: tenant_id.clone(),
        threshold: 0.2,
        low_confidence_threshold: 0.7,
        auto_suppress_info: true,
        updated_at: chrono::Utc::now().timestamp(),
    };

    use mk_core::traits::StorageBackend;
    backend.save_drift_config(drift_config).await.unwrap();

    let mut engine = GovernanceEngine::new().with_storage(Arc::new(backend));

    let policy = Policy {
        id: format!("auto-suppress-policy-{}", test_id),
        name: "Auto Suppress Policy".to_string(),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Mandatory,
        merge_strategy: RuleMergeStrategy::Override,
        rules: vec![
            PolicyRule {
                id: "info-rule".to_string(),
                rule_type: RuleType::Allow,
                target: ConstraintTarget::Dependency,
                operator: ConstraintOperator::MustUse,
                value: serde_json::json!("optional-lib"),
                severity: ConstraintSeverity::Info,
                message: "Optional library missing".to_string(),
            },
            PolicyRule {
                id: "warn-rule".to_string(),
                rule_type: RuleType::Allow,
                target: ConstraintTarget::Dependency,
                operator: ConstraintOperator::MustUse,
                value: serde_json::json!("recommended-lib"),
                severity: ConstraintSeverity::Warn,
                message: "Recommended library missing".to_string(),
            },
        ],
        metadata: HashMap::new(),
    };
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("dependencies".to_string(), serde_json::json!([]));

    let drift_score = engine
        .check_drift(&ctx, &format!("proj-auto-suppress-{}", test_id), &context)
        .await
        .unwrap();

    assert!(
        (drift_score - 0.5).abs() < 0.01,
        "With auto_suppress_info=true, Info violations should be filtered; only Warn (0.5) should \
         count. Got: {}",
        drift_score
    );
}

#[tokio::test]
async fn test_drift_without_auto_suppress_includes_info_violations() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let test_id = unique_id("drift");
    let tenant_id = TenantId::new(format!("tenant-no-suppress-{}", test_id)).unwrap();
    let ctx = TenantContext::new(
        tenant_id.clone(),
        UserId::new("user-1".to_string()).unwrap(),
    );

    let drift_config = mk_core::types::DriftConfig {
        project_id: format!("proj-no-suppress-{}", test_id),
        tenant_id: tenant_id.clone(),
        threshold: 0.2,
        low_confidence_threshold: 0.7,
        auto_suppress_info: false,
        updated_at: chrono::Utc::now().timestamp(),
    };

    use mk_core::traits::StorageBackend;
    backend.save_drift_config(drift_config).await.unwrap();

    let mut engine = GovernanceEngine::new().with_storage(Arc::new(backend));

    let policy = Policy {
        id: format!("no-suppress-policy-{}", test_id),
        name: "No Suppress Policy".to_string(),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Mandatory,
        merge_strategy: RuleMergeStrategy::Override,
        rules: vec![
            PolicyRule {
                id: "info-rule".to_string(),
                rule_type: RuleType::Allow,
                target: ConstraintTarget::Dependency,
                operator: ConstraintOperator::MustUse,
                value: serde_json::json!("optional-lib"),
                severity: ConstraintSeverity::Info,
                message: "Optional library missing".to_string(),
            },
            PolicyRule {
                id: "warn-rule".to_string(),
                rule_type: RuleType::Allow,
                target: ConstraintTarget::Dependency,
                operator: ConstraintOperator::MustUse,
                value: serde_json::json!("recommended-lib"),
                severity: ConstraintSeverity::Warn,
                message: "Recommended library missing".to_string(),
            },
        ],
        metadata: HashMap::new(),
    };
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("dependencies".to_string(), serde_json::json!([]));

    let drift_score = engine
        .check_drift(&ctx, &format!("proj-no-suppress-{}", test_id), &context)
        .await
        .unwrap();

    assert!(
        (drift_score - 0.6).abs() < 0.01,
        "With auto_suppress_info=false, both Info (0.1) and Warn (0.5) should count. Got: {}",
        drift_score
    );
}

#[tokio::test]
async fn test_drift_stores_result_with_suppressions() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let test_id = unique_id("drift");
    let tenant_id = TenantId::new(format!("tenant-store-result-{}", test_id)).unwrap();
    let ctx = TenantContext::new(
        tenant_id.clone(),
        UserId::new("user-1".to_string()).unwrap(),
    );
    let project_id = format!("proj-store-result-{}", test_id);

    let drift_config = mk_core::types::DriftConfig {
        project_id: project_id.clone(),
        tenant_id: tenant_id.clone(),
        threshold: 0.2,
        low_confidence_threshold: 0.7,
        auto_suppress_info: true,
        updated_at: chrono::Utc::now().timestamp(),
    };

    use mk_core::traits::StorageBackend;
    backend.save_drift_config(drift_config).await.unwrap();

    let backend_arc = Arc::new(backend);
    let mut engine = GovernanceEngine::new().with_storage(backend_arc.clone());

    let policy = Policy {
        id: "store-result-policy".to_string(),
        name: "Store Result Policy".to_string(),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: PolicyMode::Mandatory,
        merge_strategy: RuleMergeStrategy::Override,
        rules: vec![PolicyRule {
            id: "info-rule".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustUse,
            value: serde_json::json!("optional-lib"),
            severity: ConstraintSeverity::Info,
            message: "Optional library missing".to_string(),
        }],
        metadata: HashMap::new(),
    };
    engine.add_policy(policy);

    let mut context = HashMap::new();
    context.insert("dependencies".to_string(), serde_json::json!([]));

    let _ = engine
        .check_drift(&ctx, &project_id, &context)
        .await
        .unwrap();

    let stored_result = backend_arc
        .get_latest_drift_result(ctx, &project_id)
        .await
        .unwrap();

    assert!(stored_result.is_some(), "Drift result should be stored");
    let result = stored_result.unwrap();
    assert_eq!(
        result.drift_score, 0.0,
        "With Info auto-suppressed, drift should be 0"
    );
    assert!(
        !result.suppressed_violations.is_empty(),
        "Info violation should be in suppressed list"
    );
}
