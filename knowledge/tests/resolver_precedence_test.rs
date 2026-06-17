//! Resolver precedence tests (task 12.5).
//!
//! Verifies that `query_with_precedence` and `query_enriched` apply the
//! canonical-vs-residual precedence rules defined in tasks 6.1–6.2:
//!
//! * Tenant entries rank before Project entries (layer authority ordering).
//! * Canonical entries rank before Specialization/Exception at the same layer.
//! * `query_enriched` groups a canonical primary with its residual entries that
//!   are linked via Specializes / ApplicableFrom / ExceptionTo / Clarifies.
//! * Entries with no qualifying relations appear as standalone results with an
//!   empty `local_residuals` vec.

use knowledge::governance::GovernanceEngine;
use knowledge::manager::KnowledgeManager;
use knowledge::repository::GitRepository;
use mk_core::types::{
    KnowledgeEntry, KnowledgeLayer, KnowledgeRelation, KnowledgeRelationType, KnowledgeStatus,
    KnowledgeType, KnowledgeVariantRole, TenantContext, TenantId, UserId,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn test_ctx() -> (TenantContext, TempDir) {
    let dir = TempDir::new().unwrap();
    let tenant_id = TenantId::new("test-tenant".to_string()).unwrap();
    let user_id = UserId::new("test-user".to_string()).unwrap();
    let ctx = TenantContext::new(tenant_id, user_id);
    (ctx, dir)
}

fn make_manager(dir: &TempDir) -> KnowledgeManager {
    let repo = Arc::new(GitRepository::new(dir.path()).unwrap());
    let governance = Arc::new(GovernanceEngine::new());
    KnowledgeManager::new(repo, governance)
}

/// Build a minimal `KnowledgeEntry` with an optional `variant_role` metadata field.
fn make_entry(
    path: &str,
    content: &str,
    layer: KnowledgeLayer,
    variant_role: Option<KnowledgeVariantRole>,
    updated_at: i64,
) -> KnowledgeEntry {
    let mut metadata = HashMap::new();
    if let Some(role) = variant_role {
        // Store as the string variant name so `variant_role()` can parse it back
        metadata.insert(
            "variant_role".to_string(),
            serde_json::json!(format!("{:?}", role)),
        );
    }
    KnowledgeEntry {
        path: path.to_string(),
        content: content.to_string(),
        layer,
        kind: KnowledgeType::Adr,
        status: KnowledgeStatus::Accepted,
        summaries: HashMap::new(),
        metadata,
        commit_hash: None,
        author: Some("test-user".to_string()),
        updated_at,
    }
}

/// Seed an entry into the manager and return its path.
async fn seed(manager: &KnowledgeManager, ctx: TenantContext, entry: KnowledgeEntry) -> String {
    let path = entry.path.clone();
    manager.add(ctx, entry, "seed").await.unwrap();
    path
}

/// Build a minimal `KnowledgeRelation`.
fn make_relation(
    source_id: &str,
    target_id: &str,
    relation_type: KnowledgeRelationType,
    ctx: &TenantContext,
) -> KnowledgeRelation {
    KnowledgeRelation {
        id: uuid::Uuid::new_v4().to_string(),
        source_id: source_id.to_string(),
        target_id: target_id.to_string(),
        relation_type,
        tenant_id: ctx.tenant_id.clone(),
        created_by: ctx.user_id.clone(),
        created_at: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
    }
}

// ── Task 6.1 — Layer authority ordering ───────────────────────────────────────

/// `query_with_precedence` must return Tenant entries before Project entries.
#[tokio::test]
async fn test_layer_authority_company_before_project() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    // Project entry seeded first so repository order is project-then-tenant
    let project_entry = make_entry(
        "resolver/project-entry",
        "project knowledge",
        KnowledgeLayer::Project,
        None, // defaults to Canonical
        1000,
    );
    let company_entry = make_entry(
        "resolver/tenant-entry",
        "tenant knowledge",
        KnowledgeLayer::Tenant,
        None,
        1000,
    );

    seed(&manager, ctx.clone(), project_entry).await;
    seed(&manager, ctx.clone(), company_entry).await;

    let results = manager
        .query_with_precedence(
            ctx,
            "knowledge",
            vec![KnowledgeLayer::Tenant, KnowledgeLayer::Project],
            10,
        )
        .await
        .unwrap();

    assert!(
        results.len() >= 2,
        "expected at least two results, got {}",
        results.len()
    );

    // Tenant entry must be first
    assert_eq!(
        results[0].layer,
        KnowledgeLayer::Tenant,
        "Tenant layer must rank before Project layer"
    );
}

/// Full layer ordering: Tenant > Org > Team > Project.
#[tokio::test]
async fn test_layer_authority_full_ordering() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    // Seed in reverse precedence order to defeat any insertion-order bias
    for (path, layer) in &[
        ("resolver/proj", KnowledgeLayer::Project),
        ("resolver/team", KnowledgeLayer::Team),
        ("resolver/org", KnowledgeLayer::Org),
        ("resolver/co", KnowledgeLayer::Tenant),
    ] {
        let e = make_entry(path, "ordering content", *layer, None, 1000);
        seed(&manager, ctx.clone(), e).await;
    }

    let results = manager
        .query_with_precedence(
            ctx,
            "ordering content",
            vec![
                KnowledgeLayer::Tenant,
                KnowledgeLayer::Org,
                KnowledgeLayer::Team,
                KnowledgeLayer::Project,
            ],
            10,
        )
        .await
        .unwrap();

    assert!(results.len() >= 4, "expected at least 4 results");

    let layers: Vec<KnowledgeLayer> = results.iter().map(|e| e.layer).collect();
    let expected = [
        KnowledgeLayer::Tenant,
        KnowledgeLayer::Org,
        KnowledgeLayer::Team,
        KnowledgeLayer::Project,
    ];
    for (i, expected_layer) in expected.iter().enumerate() {
        assert_eq!(
            layers[i], *expected_layer,
            "position {i}: expected {expected_layer:?}, got {:?}",
            layers[i]
        );
    }
}

// ── Task 6.1 — Variant-role ordering within same layer ─────────────────────

/// At the same layer, Canonical ranks before Specialization.
#[tokio::test]
async fn test_variant_role_canonical_before_specialization_same_layer() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    // Seed Specialization first so insertion order favours it
    let specialization = make_entry(
        "resolver/spec-entry",
        "role ordering",
        KnowledgeLayer::Team,
        Some(KnowledgeVariantRole::Specialization),
        1000,
    );
    let canonical = make_entry(
        "resolver/canon-entry",
        "role ordering",
        KnowledgeLayer::Team,
        Some(KnowledgeVariantRole::Canonical),
        1000,
    );

    seed(&manager, ctx.clone(), specialization).await;
    seed(&manager, ctx.clone(), canonical).await;

    let results = manager
        .query_with_precedence(ctx, "role ordering", vec![KnowledgeLayer::Team], 10)
        .await
        .unwrap();

    assert!(results.len() >= 2);

    // Canonical must precede Specialization
    let canonical_pos = results
        .iter()
        .position(|e| e.path == "resolver/canon-entry")
        .expect("canonical entry not found");
    let spec_pos = results
        .iter()
        .position(|e| e.path == "resolver/spec-entry")
        .expect("specialization entry not found");

    assert!(
        canonical_pos < spec_pos,
        "Canonical (pos {canonical_pos}) must rank before Specialization (pos {spec_pos})"
    );
}

/// Full variant-role ordering within the same layer:
/// Canonical > Clarification > Specialization > Applicability > Exception.
#[tokio::test]
async fn test_variant_role_full_ordering_same_layer() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    // Seed in reverse expected order
    let roles = [
        ("resolver/exception", KnowledgeVariantRole::Exception),
        (
            "resolver/applicability",
            KnowledgeVariantRole::Applicability,
        ),
        (
            "resolver/specialization",
            KnowledgeVariantRole::Specialization,
        ),
        (
            "resolver/clarification",
            KnowledgeVariantRole::Clarification,
        ),
        ("resolver/canonical", KnowledgeVariantRole::Canonical),
    ];

    for (path, role) in &roles {
        let e = make_entry(
            path,
            "role full ordering",
            KnowledgeLayer::Org,
            Some(*role),
            1000,
        );
        seed(&manager, ctx.clone(), e).await;
    }

    let results = manager
        .query_with_precedence(ctx, "role full ordering", vec![KnowledgeLayer::Org], 10)
        .await
        .unwrap();

    assert!(results.len() >= 5);

    let path_order: Vec<&str> = results.iter().map(|e| e.path.as_str()).collect();

    let pos = |p: &str| path_order.iter().position(|&x| x == p).unwrap();

    assert!(
        pos("resolver/canonical") < pos("resolver/clarification"),
        "Canonical before Clarification"
    );
    assert!(
        pos("resolver/clarification") < pos("resolver/specialization"),
        "Clarification before Specialization"
    );
    assert!(
        pos("resolver/specialization") < pos("resolver/applicability"),
        "Specialization before Applicability"
    );
    assert!(
        pos("resolver/applicability") < pos("resolver/exception"),
        "Applicability before Exception"
    );
}

/// Within the same layer AND same variant role, the most recently updated entry
/// must appear first (updated_at descending).
#[tokio::test]
async fn test_updated_at_tiebreak_within_same_role_and_layer() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    // Older canonical seeded first
    let older = make_entry(
        "resolver/older-canon",
        "tiebreak content",
        KnowledgeLayer::Team,
        Some(KnowledgeVariantRole::Canonical),
        500,
    );
    let newer = make_entry(
        "resolver/newer-canon",
        "tiebreak content",
        KnowledgeLayer::Team,
        Some(KnowledgeVariantRole::Canonical),
        1500,
    );

    seed(&manager, ctx.clone(), older).await;
    seed(&manager, ctx.clone(), newer).await;

    let results = manager
        .query_with_precedence(ctx, "tiebreak content", vec![KnowledgeLayer::Team], 10)
        .await
        .unwrap();

    let newer_pos = results
        .iter()
        .position(|e| e.path == "resolver/newer-canon")
        .expect("newer entry not found");
    let older_pos = results
        .iter()
        .position(|e| e.path == "resolver/older-canon")
        .expect("older entry not found");

    assert!(
        newer_pos < older_pos,
        "newer updated_at (pos {newer_pos}) must rank before older (pos {older_pos})"
    );
}

// ── Task 6.2 — query_enriched grouping ────────────────────────────────────────

/// `query_enriched` groups a Canonical primary with a Specialization residual
/// linked via a `Specializes` relation.
#[tokio::test]
async fn test_enriched_groups_canonical_with_specialization() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    // Canonical entry at Team layer
    let canonical = make_entry(
        "resolver/enriched-canon",
        "grouping test",
        KnowledgeLayer::Team,
        Some(KnowledgeVariantRole::Canonical),
        1000,
    );
    // Specialization residual at the same layer
    let residual = make_entry(
        "resolver/enriched-spec",
        "grouping test",
        KnowledgeLayer::Team,
        Some(KnowledgeVariantRole::Specialization),
        900,
    );

    seed(&manager, ctx.clone(), canonical).await;
    seed(&manager, ctx.clone(), residual).await;

    // Link: residual Specializes canonical
    let rel = make_relation(
        "resolver/enriched-spec",
        "resolver/enriched-canon",
        KnowledgeRelationType::Specializes,
        &ctx,
    );
    manager.create_relation(ctx.clone(), rel).await.unwrap();

    let results = manager
        .query_enriched(ctx, "grouping test", vec![KnowledgeLayer::Team], 10)
        .await
        .unwrap();

    // We expect one grouped result, not two flat results
    assert_eq!(
        results.len(),
        1,
        "expected 1 grouped result, got {}",
        results.len()
    );

    let group = &results[0];
    assert_eq!(
        group.primary.entry.path, "resolver/enriched-canon",
        "primary must be the canonical entry"
    );
    assert_eq!(group.local_residuals.len(), 1, "expected 1 local residual");
    assert_eq!(
        group.local_residuals[0].0,
        KnowledgeRelationType::Specializes,
        "residual relation type must be Specializes"
    );
    assert_eq!(
        group.local_residuals[0].1.entry.path, "resolver/enriched-spec",
        "residual entry must be the specialization"
    );
}

/// All four qualifying residual relation types are grouped correctly:
/// Specializes, ApplicableFrom, ExceptionTo, Clarifies.
#[tokio::test]
async fn test_enriched_groups_all_residual_relation_types() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let canonical = make_entry(
        "resolver/multi-canon",
        "multi residual",
        KnowledgeLayer::Org,
        Some(KnowledgeVariantRole::Canonical),
        2000,
    );
    seed(&manager, ctx.clone(), canonical).await;

    let residuals: &[(&str, KnowledgeVariantRole, KnowledgeRelationType)] = &[
        (
            "resolver/multi-spec",
            KnowledgeVariantRole::Specialization,
            KnowledgeRelationType::Specializes,
        ),
        (
            "resolver/multi-applicable",
            KnowledgeVariantRole::Applicability,
            KnowledgeRelationType::ApplicableFrom,
        ),
        (
            "resolver/multi-exception",
            KnowledgeVariantRole::Exception,
            KnowledgeRelationType::ExceptionTo,
        ),
        (
            "resolver/multi-clarification",
            KnowledgeVariantRole::Clarification,
            KnowledgeRelationType::Clarifies,
        ),
    ];

    for (path, role, rel_type) in residuals {
        let e = make_entry(
            path,
            "multi residual",
            KnowledgeLayer::Org,
            Some(*role),
            1000,
        );
        seed(&manager, ctx.clone(), e).await;

        let rel = make_relation(path, "resolver/multi-canon", *rel_type, &ctx);
        manager.create_relation(ctx.clone(), rel).await.unwrap();
    }

    let results = manager
        .query_enriched(ctx, "multi residual", vec![KnowledgeLayer::Org], 10)
        .await
        .unwrap();

    assert_eq!(
        results.len(),
        1,
        "all residuals should collapse into one group"
    );

    let group = &results[0];
    assert_eq!(
        group.primary.entry.path, "resolver/multi-canon",
        "primary must be the canonical entry"
    );
    assert_eq!(
        group.local_residuals.len(),
        4,
        "expected all 4 residuals to be grouped"
    );

    let residual_relation_types: Vec<KnowledgeRelationType> =
        group.local_residuals.iter().map(|(t, _)| *t).collect();

    for (_, _, expected_rel_type) in residuals {
        assert!(
            residual_relation_types.contains(expected_rel_type),
            "expected relation type {:?} to be present in local_residuals",
            expected_rel_type
        );
    }
}

/// An entry with no qualifying relations is returned as a standalone result
/// with an empty `local_residuals` vec.
#[tokio::test]
async fn test_enriched_standalone_entry_has_empty_local_residuals() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let entry = make_entry(
        "resolver/standalone",
        "standalone content",
        KnowledgeLayer::Project,
        None,
        1000,
    );
    seed(&manager, ctx.clone(), entry).await;

    let results = manager
        .query_enriched(ctx, "standalone content", vec![KnowledgeLayer::Project], 10)
        .await
        .unwrap();

    assert_eq!(results.len(), 1, "expected exactly one result");
    assert_eq!(
        results[0].primary.entry.path, "resolver/standalone",
        "result path must match seeded entry"
    );
    assert!(
        results[0].local_residuals.is_empty(),
        "standalone entry must have no local_residuals"
    );
}

/// `query_enriched` de-duplicates: a residual that has already been consumed by
/// a group does not appear as a second standalone result.
#[tokio::test]
async fn test_enriched_residual_not_duplicated_as_standalone() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let canonical = make_entry(
        "resolver/dedup-canon",
        "dedup content",
        KnowledgeLayer::Team,
        Some(KnowledgeVariantRole::Canonical),
        1000,
    );
    let residual = make_entry(
        "resolver/dedup-spec",
        "dedup content",
        KnowledgeLayer::Team,
        Some(KnowledgeVariantRole::Specialization),
        900,
    );

    seed(&manager, ctx.clone(), canonical).await;
    seed(&manager, ctx.clone(), residual).await;

    let rel = make_relation(
        "resolver/dedup-spec",
        "resolver/dedup-canon",
        KnowledgeRelationType::Specializes,
        &ctx,
    );
    manager.create_relation(ctx.clone(), rel).await.unwrap();

    let results = manager
        .query_enriched(ctx, "dedup content", vec![KnowledgeLayer::Team], 10)
        .await
        .unwrap();

    // Residual must NOT appear again as a top-level result
    assert_eq!(
        results.len(),
        1,
        "residual must not appear as a second standalone result"
    );
    assert!(
        results[0]
            .local_residuals
            .iter()
            .any(|(_, e)| e.entry.path == "resolver/dedup-spec"),
        "residual must be inside local_residuals of the canonical group"
    );
}

/// Non-qualifying relation types (PromotedTo, Supersedes, DerivedFrom) do NOT
/// cause grouping — the linked entry remains standalone.
#[tokio::test]
async fn test_enriched_non_qualifying_relations_do_not_group() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let primary = make_entry(
        "resolver/non-qual-primary",
        "non qualifying",
        KnowledgeLayer::Team,
        None,
        1000,
    );
    let linked = make_entry(
        "resolver/non-qual-linked",
        "non qualifying",
        KnowledgeLayer::Team,
        None,
        900,
    );

    seed(&manager, ctx.clone(), primary).await;
    seed(&manager, ctx.clone(), linked).await;

    // PromotedTo is NOT a grouping relation
    let rel = make_relation(
        "resolver/non-qual-primary",
        "resolver/non-qual-linked",
        KnowledgeRelationType::PromotedTo,
        &ctx,
    );
    manager.create_relation(ctx.clone(), rel).await.unwrap();

    let results = manager
        .query_enriched(ctx, "non qualifying", vec![KnowledgeLayer::Team], 10)
        .await
        .unwrap();

    // Both entries should appear as separate standalone results
    assert_eq!(
        results.len(),
        2,
        "non-qualifying relation must not cause grouping; expected 2 standalone results"
    );
    for r in &results {
        assert!(
            r.local_residuals.is_empty(),
            "entry {} must have no local_residuals when relation is non-qualifying",
            r.primary.entry.path
        );
    }
}

/// `query_enriched` primary entries carry their own relations in
/// `primary.relations`, not just those that were used for grouping.
#[tokio::test]
async fn test_enriched_primary_relations_field_is_populated() {
    let (ctx, dir) = test_ctx();
    let manager = make_manager(&dir);

    let entry = make_entry(
        "resolver/rels-primary",
        "relations field",
        KnowledgeLayer::Org,
        None,
        1000,
    );
    let other = make_entry(
        "resolver/rels-other",
        "relations field",
        KnowledgeLayer::Org,
        None,
        900,
    );

    seed(&manager, ctx.clone(), entry).await;
    seed(&manager, ctx.clone(), other).await;

    let rel = make_relation(
        "resolver/rels-primary",
        "resolver/rels-other",
        KnowledgeRelationType::DerivedFrom,
        &ctx,
    );
    manager.create_relation(ctx.clone(), rel).await.unwrap();

    let results = manager
        .query_enriched(ctx, "relations field", vec![KnowledgeLayer::Org], 10)
        .await
        .unwrap();

    // Both results are standalone (DerivedFrom is not a grouping relation)
    assert_eq!(results.len(), 2);

    let primary_result = results
        .iter()
        .find(|r| r.primary.entry.path == "resolver/rels-primary")
        .expect("primary entry not found");

    // The DerivedFrom relation must appear in primary.relations
    assert!(
        primary_result
            .primary
            .relations
            .iter()
            .any(|r| r.relation_type == KnowledgeRelationType::DerivedFrom),
        "primary.relations must include the DerivedFrom relation"
    );
}
