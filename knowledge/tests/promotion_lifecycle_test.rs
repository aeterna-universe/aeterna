//! Unit tests for the promotion lifecycle (tasks 2.1–2.10, 12.1).
//!
//! These tests use a local GitRepository (no remote/Docker needed) so they
//! run purely in-process.

use knowledge::governance::GovernanceEngine;
use knowledge::manager::{KnowledgeManager, KnowledgeManagerError};
use knowledge::repository::GitRepository;
use mk_core::types::{
    ConstraintOperator, ConstraintSeverity, ConstraintTarget, KnowledgeEntry, KnowledgeLayer,
    KnowledgeRelation, KnowledgeRelationType, KnowledgeStatus, KnowledgeType, KnowledgeVariantRole,
    Policy, PolicyMode, PolicyRule, PromotionDecision, PromotionMode, PromotionRequest,
    PromotionRequestStatus, Role, RuleMergeStrategy, RuleType, TenantContext, TenantId, UserId,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn test_ctx() -> (TenantContext, TempDir) {
    let dir = TempDir::new().unwrap();
    let tenant_id = TenantId::new("test-tenant".to_string()).unwrap();
    let user_id = UserId::new("test-user".to_string()).unwrap();
    let ctx = TenantContext::new(tenant_id, user_id);
    (ctx, dir)
}

/// Returns a context with Admin role for reviewer operations.
fn reviewer_ctx(ctx: &TenantContext) -> TenantContext {
    ctx.clone().with_role(Role::Admin)
}

fn make_manager(dir: &TempDir) -> KnowledgeManager {
    let repo = Arc::new(GitRepository::new(dir.path()).unwrap());
    let governance = Arc::new(GovernanceEngine::new());
    KnowledgeManager::new(repo, governance)
}

fn make_manager_with_governance(dir: &TempDir, governance: GovernanceEngine) -> KnowledgeManager {
    let repo = Arc::new(GitRepository::new(dir.path()).unwrap());
    KnowledgeManager::new(repo, Arc::new(governance))
}

async fn seed_accepted_entry(
    manager: &KnowledgeManager,
    ctx: TenantContext,
    path: &str,
    content: &str,
    layer: KnowledgeLayer,
) -> String {
    let entry = KnowledgeEntry {
        path: path.to_string(),
        content: content.to_string(),
        layer,
        kind: KnowledgeType::Adr,
        status: KnowledgeStatus::Accepted,
        summaries: HashMap::new(),
        metadata: HashMap::new(),
        commit_hash: None,
        author: Some("test-user".to_string()),
        updated_at: 0,
    };
    manager.add(ctx, entry, "seed entry").await.unwrap();
    path.to_string()
}

fn make_promotion_request(
    source_id: &str,
    source_layer: KnowledgeLayer,
    target_layer: KnowledgeLayer,
    mode: PromotionMode,
    ctx: &TenantContext,
) -> PromotionRequest {
    PromotionRequest {
        id: String::new(),
        source_item_id: source_id.to_string(),
        source_layer,
        source_status: KnowledgeStatus::Accepted,
        target_layer,
        promotion_mode: mode,
        shared_content: "shared content".to_string(),
        residual_content: Some("residual content".to_string()),
        residual_role: Some(KnowledgeVariantRole::Specialization),
        justification: Some("unit test".to_string()),
        status: PromotionRequestStatus::PendingReview,
        requested_by: ctx.user_id.clone(),
        tenant_id: ctx.tenant_id.clone(),
        source_version: String::new(),
        latest_decision: None,
        promoted_item_id: None,
        residual_item_id: None,
        created_at: 0,
        updated_at: 0,
    }
}

fn blocking_content_policy(id: &str, layer: KnowledgeLayer, forbidden_value: &str) -> Policy {
    Policy {
        id: id.to_string(),
        name: format!("Policy {id}"),
        description: None,
        layer,
        mode: PolicyMode::Mandatory,
        merge_strategy: RuleMergeStrategy::Override,
        rules: vec![PolicyRule {
            id: format!("{id}-rule"),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustNotMatch,
            value: serde_json::json!(forbidden_value),
            severity: ConstraintSeverity::Block,
            message: format!("content must not include {forbidden_value}"),
        }],
        metadata: HashMap::new(),
    }
}

// ── Task 2.1 — Preview ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_preview_promotion_returns_shared_content() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-001",
        "decision content",
        KnowledgeLayer::Project,
    )
    .await;

    let preview = manager
        .preview_promotion(ctx, &path, KnowledgeLayer::Team, PromotionMode::Full)
        .await
        .unwrap();

    assert_eq!(preview.source_item_id, path);
    assert_eq!(preview.source_layer, KnowledgeLayer::Project);
    assert_eq!(preview.target_layer, KnowledgeLayer::Team);
    assert_eq!(preview.shared_content, "decision content");
    assert!(preview.residual_content.is_none());
}

#[tokio::test]
async fn test_preview_partial_includes_residual_slot() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-002",
        "full content",
        KnowledgeLayer::Project,
    )
    .await;

    let preview = manager
        .preview_promotion(ctx, &path, KnowledgeLayer::Team, PromotionMode::Partial)
        .await
        .unwrap();

    assert!(preview.residual_content.is_some());
}

#[tokio::test]
async fn test_preview_rejects_downward_promotion() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-003",
        "content",
        KnowledgeLayer::Team,
    )
    .await;

    let result = manager
        .preview_promotion(ctx, &path, KnowledgeLayer::Project, PromotionMode::Full)
        .await;

    assert!(matches!(result, Err(KnowledgeManagerError::Validation(_))));
}

#[tokio::test]
async fn test_preview_missing_source_returns_error() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let result = manager
        .preview_promotion(
            ctx,
            "nonexistent/item",
            KnowledgeLayer::Team,
            PromotionMode::Full,
        )
        .await;

    assert!(matches!(
        result,
        Err(KnowledgeManagerError::SourceNotFound(_))
    ));
}

// ── Task 2.2 — Create ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_create_promotion_request_assigns_id_and_pending_status() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-010",
        "content",
        KnowledgeLayer::Project,
    )
    .await;

    let req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );

    let stored = manager.create_promotion_request(ctx, req).await.unwrap();

    assert!(!stored.id.is_empty());
    assert_eq!(stored.status, PromotionRequestStatus::PendingReview);
    assert!(stored.created_at > 0);
}

#[tokio::test]
async fn test_create_promotion_request_rejects_company_source() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let req = make_promotion_request(
        "some/item",
        KnowledgeLayer::Tenant,
        KnowledgeLayer::Org,
        PromotionMode::Full,
        &ctx,
    );

    let result = manager.create_promotion_request(ctx, req).await;
    assert!(matches!(result, Err(KnowledgeManagerError::Validation(_))));
}

#[tokio::test]
async fn test_create_promotion_request_rejects_cross_tenant_request() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-tenant-mismatch",
        "content",
        KnowledgeLayer::Project,
    )
    .await;

    let mut req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );
    req.tenant_id = TenantId::new("other-tenant".to_string()).unwrap();

    let result = manager.create_promotion_request(ctx, req).await;
    assert!(matches!(
        result,
        Err(KnowledgeManagerError::TenantMismatch(_))
    ));
}

#[tokio::test]
async fn test_create_promotion_request_rejects_cross_tenant_target_context() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-cross-target",
        "content",
        KnowledgeLayer::Project,
    )
    .await;

    let req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );

    let cross_tenant_ctx = ctx.clone();
    let cross_tenant_ctx = TenantContext {
        target_tenant_id: Some(TenantId::new("other-tenant".to_string()).unwrap()),
        ..cross_tenant_ctx
    };

    let result = manager
        .create_promotion_request(cross_tenant_ctx, req)
        .await;
    assert!(matches!(
        result,
        Err(KnowledgeManagerError::ForbiddenCrossTenant)
    ));
}

#[tokio::test]
async fn test_create_promotion_request_rejects_sensitive_content() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-sensitive",
        "content",
        KnowledgeLayer::Project,
    )
    .await;

    let mut req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );
    req.shared_content = "api_key=abc123".to_string();

    let result = manager.create_promotion_request(ctx, req).await;
    assert!(matches!(
        result,
        Err(KnowledgeManagerError::ConfidentialContent)
    ));
}

#[tokio::test]
async fn test_create_promotion_request_blocks_on_governance_policy() {
    let (ctx, dir) = test_ctx();
    let mut governance = GovernanceEngine::new();
    governance.add_policy(blocking_content_policy(
        "promotion-submit-policy",
        KnowledgeLayer::Team,
        "forbidden-pattern",
    ));
    let manager = make_manager_with_governance(&dir, governance);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-governance",
        "content",
        KnowledgeLayer::Project,
    )
    .await;

    let mut req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );
    req.shared_content = "contains forbidden-pattern".to_string();

    let result = manager.create_promotion_request(ctx, req).await;
    assert!(matches!(result, Err(KnowledgeManagerError::Governance(_))));
}

// ── Task 2.3 — Approve ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_approve_transitions_to_approved() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-020",
        "content",
        KnowledgeLayer::Project,
    )
    .await;

    let req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );
    let created = manager
        .create_promotion_request(ctx.clone(), req)
        .await
        .unwrap();

    let approved = manager
        .approve_promotion(
            reviewer_ctx(&ctx),
            &created.id,
            PromotionDecision::ApproveAsReplacement,
            None,
        )
        .await
        .unwrap();

    assert_eq!(approved.status, PromotionRequestStatus::Approved);
    assert_eq!(
        approved.latest_decision,
        Some(PromotionDecision::ApproveAsReplacement)
    );
}

#[tokio::test]
async fn test_approve_already_approved_fails() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-021",
        "content",
        KnowledgeLayer::Project,
    )
    .await;

    let req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );
    let created = manager
        .create_promotion_request(ctx.clone(), req)
        .await
        .unwrap();
    let approved = manager
        .approve_promotion(
            reviewer_ctx(&ctx),
            &created.id,
            PromotionDecision::ApproveAsReplacement,
            None,
        )
        .await
        .unwrap();

    // Task 8.2 — idempotency: re-approving an already-approved promotion must
    // return the existing record (Ok), not an error.
    let result = manager
        .approve_promotion(
            reviewer_ctx(&ctx),
            &approved.id,
            PromotionDecision::ApproveAsReplacement,
            None,
        )
        .await;
    assert!(
        result.is_ok(),
        "re-approving an approved promotion must be idempotent (returns Ok)"
    );
    assert_eq!(result.unwrap().status, PromotionRequestStatus::Approved);
}

#[tokio::test]
async fn test_approve_rejects_cross_tenant_context() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-approve-cross-tenant",
        "content",
        KnowledgeLayer::Project,
    )
    .await;

    let req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );
    let created = manager
        .create_promotion_request(ctx.clone(), req)
        .await
        .unwrap();

    let other_ctx = TenantContext::new(
        TenantId::new("other-tenant".to_string()).unwrap(),
        UserId::new("reviewer".to_string()).unwrap(),
    )
    .with_role(Role::TechLead);

    let result = manager
        .approve_promotion(
            other_ctx,
            &created.id,
            PromotionDecision::ApproveAsReplacement,
            None,
        )
        .await;

    // Cross-tenant access is rejected: either PromotionNotFound (tenant-scoped lookup)
    // or TenantMismatch (explicit check).
    assert!(
        result.is_err(),
        "Cross-tenant approve must fail, got: {result:?}"
    );
}

#[tokio::test]
async fn test_approve_requires_reviewer_authority_for_target_layer() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-org-review",
        "content",
        KnowledgeLayer::Project,
    )
    .await;

    let req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Org,
        PromotionMode::Full,
        &ctx,
    );
    let created = manager
        .create_promotion_request(ctx.clone(), req)
        .await
        .unwrap();

    let reviewer_ctx = ctx.clone().with_role(Role::TechLead);
    let result = manager
        .approve_promotion(
            reviewer_ctx,
            &created.id,
            PromotionDecision::ApproveAsReplacement,
            None,
        )
        .await;

    assert!(matches!(
        result,
        Err(KnowledgeManagerError::Authorization(_))
    ));
}

#[tokio::test]
async fn test_approve_blocks_when_governance_policy_fails() {
    let (ctx, dir) = test_ctx();
    let mut governance = GovernanceEngine::new();
    governance.add_policy(blocking_content_policy(
        "promotion-approve-policy",
        KnowledgeLayer::Team,
        "blocked-at-approve",
    ));
    let manager = make_manager_with_governance(&dir, governance);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-approve-policy",
        "content",
        KnowledgeLayer::Project,
    )
    .await;

    let mut req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );
    req.shared_content = "blocked-at-approve".to_string();
    let created = manager.create_promotion_request(ctx.clone(), req).await;
    assert!(matches!(created, Err(KnowledgeManagerError::Governance(_))));
}

// ── Task 2.4 — Reject ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_reject_transitions_to_rejected() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-030",
        "content",
        KnowledgeLayer::Project,
    )
    .await;

    let req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );
    let created = manager
        .create_promotion_request(ctx.clone(), req)
        .await
        .unwrap();

    let rejected = manager
        .reject_promotion(reviewer_ctx(&ctx), &created.id, "not ready", None)
        .await
        .unwrap();

    assert_eq!(rejected.status, PromotionRequestStatus::Rejected);
    assert_eq!(rejected.latest_decision, Some(PromotionDecision::Reject));
}

#[tokio::test]
async fn test_reject_does_not_modify_source_item() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-031",
        "original content",
        KnowledgeLayer::Project,
    )
    .await;

    let req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );
    let created = manager
        .create_promotion_request(ctx.clone(), req)
        .await
        .unwrap();
    manager
        .reject_promotion(reviewer_ctx(&ctx), &created.id, "not ready", None)
        .await
        .unwrap();

    // Source entry still accessible and status unchanged
    let entry = manager
        .get(ctx, KnowledgeLayer::Project, &path)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(entry.status, KnowledgeStatus::Accepted);
    assert_eq!(entry.content, "original content");
}

#[tokio::test]
async fn test_reject_requires_reviewer_authority() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-reject-authz",
        "content",
        KnowledgeLayer::Project,
    )
    .await;

    let req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );
    let created = manager
        .create_promotion_request(ctx.clone(), req)
        .await
        .unwrap();

    let reviewer_ctx = ctx.clone().with_role(Role::Developer);
    let result = manager
        .reject_promotion(reviewer_ctx, &created.id, "not allowed", None)
        .await;
    assert!(matches!(
        result,
        Err(KnowledgeManagerError::Authorization(_))
    ));
}

// ── Task 2.5 — Retarget ──────────────────────────────────────────────────────

#[tokio::test]
async fn test_retarget_updates_target_layer() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-040",
        "content",
        KnowledgeLayer::Project,
    )
    .await;

    let req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );
    let created = manager
        .create_promotion_request(ctx.clone(), req)
        .await
        .unwrap();

    let retargeted = manager
        .retarget_promotion(reviewer_ctx(&ctx), &created.id, KnowledgeLayer::Org, None)
        .await
        .unwrap();

    assert_eq!(retargeted.target_layer, KnowledgeLayer::Org);
    assert_eq!(retargeted.status, PromotionRequestStatus::PendingReview);
    assert_eq!(
        retargeted.latest_decision,
        Some(PromotionDecision::RetargetLayer)
    );
}

#[tokio::test]
async fn test_retarget_rejects_downward_target() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-041",
        "content",
        KnowledgeLayer::Team,
    )
    .await;

    // Source at Team, initial target Org — try retarget to Project (downward)
    let mut req = make_promotion_request(
        &path,
        KnowledgeLayer::Team,
        KnowledgeLayer::Org,
        PromotionMode::Full,
        &ctx,
    );
    req.source_layer = KnowledgeLayer::Team;
    let created = manager
        .create_promotion_request(ctx.clone(), req)
        .await
        .unwrap();

    let result = manager
        .retarget_promotion(
            reviewer_ctx(&ctx),
            &created.id,
            KnowledgeLayer::Project,
            None,
        )
        .await;
    assert!(matches!(result, Err(KnowledgeManagerError::Validation(_))));
}

#[tokio::test]
async fn test_retarget_requires_authority_for_new_target_layer() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-retarget-authz",
        "content",
        KnowledgeLayer::Project,
    )
    .await;

    let req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );
    let created = manager
        .create_promotion_request(ctx.clone(), req)
        .await
        .unwrap();

    let reviewer_ctx = ctx.clone().with_role(Role::TechLead);
    let result = manager
        .retarget_promotion(reviewer_ctx, &created.id, KnowledgeLayer::Org, None)
        .await;
    assert!(matches!(
        result,
        Err(KnowledgeManagerError::Authorization(_))
    ));
}

// ── Task 2.6 / 2.7 — Apply (non-destructive) ─────────────────────────────────

#[tokio::test]
async fn test_apply_full_promotion_creates_promoted_entry_and_supersedes_source() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-050",
        "canonical content",
        KnowledgeLayer::Project,
    )
    .await;

    let req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );
    let created = manager
        .create_promotion_request(ctx.clone(), req)
        .await
        .unwrap();
    let approved = manager
        .approve_promotion(
            reviewer_ctx(&ctx),
            &created.id,
            PromotionDecision::ApproveAsReplacement,
            None,
        )
        .await
        .unwrap();
    let applied = manager
        .apply_promotion(reviewer_ctx(&ctx), &approved.id)
        .await
        .unwrap();

    assert_eq!(applied.status, PromotionRequestStatus::Applied);
    assert!(applied.promoted_item_id.is_some());

    // task 2.7: source still exists
    let source = manager
        .get(ctx.clone(), KnowledgeLayer::Project, &path)
        .await
        .unwrap();
    assert!(
        source.is_some(),
        "source entry must still exist after promotion"
    );

    // task 2.8: source is now Superseded (Full mode)
    assert_eq!(source.unwrap().status, KnowledgeStatus::Superseded);
}

#[tokio::test]
async fn test_apply_partial_promotion_preserves_residual_entry() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-060",
        "base content",
        KnowledgeLayer::Project,
    )
    .await;

    let mut req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Partial,
        &ctx,
    );
    req.shared_content = "shared part".to_string();
    req.residual_content = Some("residual part".to_string());
    req.residual_role = Some(KnowledgeVariantRole::Specialization);

    let created = manager
        .create_promotion_request(ctx.clone(), req)
        .await
        .unwrap();
    let approved = manager
        .approve_promotion(
            reviewer_ctx(&ctx),
            &created.id,
            PromotionDecision::ApproveAsSpecialization,
            None,
        )
        .await
        .unwrap();
    let applied = manager
        .apply_promotion(reviewer_ctx(&ctx), &approved.id)
        .await
        .unwrap();

    // task 2.9: residual item stored
    assert!(applied.residual_item_id.is_some());

    // task 2.7 + task 2.9: source NOT superseded in partial mode
    let source = manager
        .get(ctx, KnowledgeLayer::Project, &path)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        source.status,
        KnowledgeStatus::Accepted,
        "source must remain Accepted after partial promotion"
    );
}

// ── Task 2.10 — Relations ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_apply_creates_promoted_to_and_promoted_from_relations() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-070",
        "content",
        KnowledgeLayer::Project,
    )
    .await;

    let req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );
    let created = manager
        .create_promotion_request(ctx.clone(), req)
        .await
        .unwrap();
    let approved = manager
        .approve_promotion(
            reviewer_ctx(&ctx),
            &created.id,
            PromotionDecision::ApproveAsReplacement,
            None,
        )
        .await
        .unwrap();
    let applied = manager
        .apply_promotion(reviewer_ctx(&ctx), &approved.id)
        .await
        .unwrap();

    let promoted_id = applied.promoted_item_id.clone().unwrap();

    // Check relations from source
    let rels = manager
        .repository_get_relations_for_item(ctx.clone(), &path)
        .await
        .unwrap();

    let has_promoted_to = rels.iter().any(|r| {
        r.relation_type == KnowledgeRelationType::PromotedTo && r.target_id == promoted_id
    });
    assert!(has_promoted_to, "expected PromotedTo relation from source");

    // Check relations from promoted item
    let prels = manager
        .repository_get_relations_for_item(ctx, &promoted_id)
        .await
        .unwrap();
    let has_promoted_from = prels
        .iter()
        .any(|r| r.relation_type == KnowledgeRelationType::PromotedFrom && r.target_id == path);
    assert!(
        has_promoted_from,
        "expected PromotedFrom relation on promoted item"
    );
    let has_supersedes = prels
        .iter()
        .any(|r| r.relation_type == KnowledgeRelationType::Supersedes);
    assert!(
        has_supersedes,
        "expected Supersedes relation for ApproveAsReplacement"
    );
}

#[tokio::test]
async fn test_create_relation_prevents_duplicates() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let tenant_id = ctx.tenant_id.clone();
    let user_id = ctx.user_id.clone();

    let rel = KnowledgeRelation {
        id: uuid::Uuid::new_v4().to_string(),
        source_id: "item-a".to_string(),
        target_id: "item-b".to_string(),
        relation_type: KnowledgeRelationType::DerivedFrom,
        tenant_id: tenant_id.clone(),
        created_by: user_id.clone(),
        created_at: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
    };

    manager
        .create_relation(ctx.clone(), rel.clone())
        .await
        .unwrap();

    // Second identical relation should fail
    let rel2 = KnowledgeRelation {
        id: uuid::Uuid::new_v4().to_string(),
        ..rel
    };
    let result = manager.create_relation(ctx, rel2).await;
    assert!(
        matches!(result, Err(KnowledgeManagerError::DuplicateRelation(_, _))),
        "duplicate relation should be rejected"
    );
}

#[tokio::test]
async fn test_create_relation_rejects_tenant_mismatch() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let relation = KnowledgeRelation {
        id: uuid::Uuid::new_v4().to_string(),
        source_id: "item-a".to_string(),
        target_id: "item-b".to_string(),
        relation_type: KnowledgeRelationType::DerivedFrom,
        tenant_id: TenantId::new("other-tenant".to_string()).unwrap(),
        created_by: ctx.user_id.clone(),
        created_at: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
    };

    let result = manager.create_relation(ctx, relation).await;
    assert!(matches!(
        result,
        Err(KnowledgeManagerError::TenantMismatch(_))
    ));
}

#[tokio::test]
async fn test_apply_nonexistent_promotion_returns_not_found() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let result = manager.apply_promotion(ctx, "no-such-id").await;
    assert!(matches!(
        result,
        Err(KnowledgeManagerError::PromotionNotFound(_))
    ));
}

#[tokio::test]
async fn test_apply_unapproved_promotion_returns_invalid_transition() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "test/adr-080",
        "content",
        KnowledgeLayer::Project,
    )
    .await;

    let req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );
    let created = manager
        .create_promotion_request(ctx.clone(), req)
        .await
        .unwrap();

    // Attempt to apply without approving first
    let result = manager.apply_promotion(ctx, &created.id).await;
    assert!(matches!(
        result,
        Err(KnowledgeManagerError::InvalidPromotionTransition(_))
    ));
}

// ── Task 12.10 — End-to-end promotion lifecycle tests ────────────────────────

/// 12.10.1 — Full promotion: source marked Superseded, promoted item exists at target layer.
#[tokio::test]
async fn e2e_full_promotion_supersedes_source() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "e2e/adr-full",
        "Full promotion candidate content",
        KnowledgeLayer::Project,
    )
    .await;

    let req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );
    let created = manager
        .create_promotion_request(ctx.clone(), req)
        .await
        .unwrap();

    let approved = manager
        .approve_promotion(
            reviewer_ctx(&ctx),
            &created.id,
            PromotionDecision::ApproveAsReplacement,
            None,
        )
        .await
        .unwrap();
    assert_eq!(approved.status, PromotionRequestStatus::Approved);

    let applied = manager
        .apply_promotion(reviewer_ctx(&ctx), &approved.id)
        .await
        .unwrap();
    assert_eq!(applied.status, PromotionRequestStatus::Applied);

    // Source must be Superseded
    let source_item = manager
        .get(ctx.clone(), KnowledgeLayer::Project, &path)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        source_item.status,
        KnowledgeStatus::Superseded,
        "12.10.1: source must be Superseded after full promotion"
    );

    // Promoted item must exist at target layer
    let promoted_id = applied.promoted_item_id.clone().unwrap();
    let promoted_item = manager
        .get(ctx.clone(), KnowledgeLayer::Team, &promoted_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(promoted_item.layer, KnowledgeLayer::Team);
    assert_eq!(
        promoted_item.status,
        KnowledgeStatus::Accepted,
        "12.10.1: promoted item must be Accepted at target layer"
    );
}

/// 12.10.2 — Partial promotion: residual survives at source layer with Specialization role.
#[tokio::test]
async fn e2e_partial_promotion_preserves_residual_specialization() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "e2e/adr-partial",
        "Shared guidance with local specialization",
        KnowledgeLayer::Project,
    )
    .await;

    let req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Partial,
        &ctx,
    );
    let created = manager
        .create_promotion_request(ctx.clone(), req)
        .await
        .unwrap();

    let approved = manager
        .approve_promotion(
            reviewer_ctx(&ctx),
            &created.id,
            PromotionDecision::ApproveAsSpecialization,
            None,
        )
        .await
        .unwrap();

    let applied = manager
        .apply_promotion(reviewer_ctx(&ctx), &approved.id)
        .await
        .unwrap();
    assert_eq!(applied.status, PromotionRequestStatus::Applied);

    // Source must NOT be Superseded — residual must remain
    let source_item = manager
        .get(ctx.clone(), KnowledgeLayer::Project, &path)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        source_item.status,
        KnowledgeStatus::Superseded,
        "12.10.2: source must not be Superseded in partial promotion"
    );

    // Residual item must exist
    let residual_id = applied
        .residual_item_id
        .clone()
        .expect("12.10.2: partial promotion must produce a residual_item_id");
    let residual_item = manager
        .get(ctx.clone(), KnowledgeLayer::Project, &residual_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        residual_item.layer,
        KnowledgeLayer::Project,
        "12.10.2: residual must stay at source layer"
    );
    // Variant role must be Specialization
    let role = residual_item
        .metadata
        .get("variant_role")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert_eq!(
        role, "specialization",
        "12.10.2: residual item must carry variant_role=specialization"
    );
}

/// 12.10.3 — Promotion rejected by reviewer: source unchanged, status = Rejected.
#[tokio::test]
async fn e2e_promotion_rejected_source_unchanged() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "e2e/adr-reject",
        "Content to be rejected",
        KnowledgeLayer::Project,
    )
    .await;

    let req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Team,
        PromotionMode::Full,
        &ctx,
    );
    let created = manager
        .create_promotion_request(ctx.clone(), req)
        .await
        .unwrap();

    let rejected = manager
        .reject_promotion(
            reviewer_ctx(&ctx),
            &created.id,
            "Not appropriate for team-wide adoption",
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        rejected.status,
        PromotionRequestStatus::Rejected,
        "12.10.3: promotion must be Rejected"
    );
    assert!(rejected.latest_decision.is_some());

    // Source must remain Accepted
    let source_item = manager
        .get(ctx.clone(), KnowledgeLayer::Project, &path)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        source_item.status,
        KnowledgeStatus::Accepted,
        "12.10.3: source must remain Accepted after rejection"
    );

    // No promoted item should exist
    assert!(
        rejected.promoted_item_id.is_none(),
        "12.10.3: no promoted item should exist after rejection"
    );
}

/// 12.10.4 — Promotion retargeted from org to team, then approved at team layer.
#[tokio::test]
async fn e2e_promotion_retargeted_then_approved() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let path = seed_accepted_entry(
        &manager,
        ctx.clone(),
        "e2e/adr-retarget",
        "Content originally targeting org, retargeted to team",
        KnowledgeLayer::Project,
    )
    .await;

    // Start targeting org
    let req = make_promotion_request(
        &path,
        KnowledgeLayer::Project,
        KnowledgeLayer::Org,
        PromotionMode::Full,
        &ctx,
    );
    let created = manager
        .create_promotion_request(ctx.clone(), req)
        .await
        .unwrap();
    assert_eq!(created.target_layer, KnowledgeLayer::Org);

    // Retarget to team
    let retargeted = manager
        .retarget_promotion(reviewer_ctx(&ctx), &created.id, KnowledgeLayer::Team, None)
        .await
        .unwrap();
    assert_eq!(
        retargeted.target_layer,
        KnowledgeLayer::Team,
        "12.10.4: target layer must be Team after retarget"
    );
    assert_eq!(retargeted.status, PromotionRequestStatus::PendingReview);

    // Approve at new target
    let approved = manager
        .approve_promotion(
            reviewer_ctx(&ctx),
            &retargeted.id,
            PromotionDecision::ApproveAsReplacement,
            None,
        )
        .await
        .unwrap();
    assert_eq!(approved.status, PromotionRequestStatus::Approved);

    // Apply
    let applied = manager
        .apply_promotion(reviewer_ctx(&ctx), &approved.id)
        .await
        .unwrap();
    assert_eq!(applied.status, PromotionRequestStatus::Applied);

    let promoted_id = applied.promoted_item_id.clone().unwrap();
    let promoted_item = manager
        .get(ctx.clone(), KnowledgeLayer::Team, &promoted_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        promoted_item.layer,
        KnowledgeLayer::Team,
        "12.10.4: promoted item must land at Team layer (retargeted destination)"
    );
}
