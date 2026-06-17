//! # Knowledge Resolver (task 6.1–6.4)
//!
//! Implements deterministic precedence rules for canonical vs residual
//! knowledge retrieval.  The resolver:
//!
//! 1. **Filters superseded items** from query results (task 6.1 / spec
//!    "Deterministic Resolution Precedence").
//! 2. **Groups residuals under their canonical parent** using `Specializes`,
//!    `ApplicableFrom`, `ExceptionTo`, and `Clarifies` relations (task 6.2).
//! 3. **Attaches relation metadata** to every returned entry (task 6.3).
//! 4. **Sorts results** by layer precedence (Tenant > Org > Team > Project)
//!    and then by variant role rank (Canonical < Specialization < …) so that
//!    the most authoritative answer always comes first (task 6.4 / CCA prep).
//!
//! The resolver is intentionally stateless and dependency-free so it can be
//! unit-tested without a live repository.

use mk_core::types::{
    KnowledgeEntry, KnowledgeEntryWithRelations, KnowledgeQueryResult, KnowledgeRelation,
    KnowledgeRelationType, KnowledgeStatus,
};
use std::collections::HashMap;

// ── Residual relation types ───────────────────────────────────────────────────

/// Relation types that connect a residual (lower-layer) item back to its
/// canonical (higher-layer) parent.
pub(crate) const RESIDUAL_RELATION_TYPES: &[KnowledgeRelationType] = &[
    KnowledgeRelationType::Specializes,
    KnowledgeRelationType::ApplicableFrom,
    KnowledgeRelationType::ExceptionTo,
    KnowledgeRelationType::Clarifies,
];

// ── Public API ────────────────────────────────────────────────────────────────

/// Build a sorted, deduplicated, relation-enriched result set from raw search
/// results and their pre-fetched relations.
///
/// # Arguments
///
/// * `entries`        – Raw entries returned by `repository.search()`.
/// * `all_relations`  – All relations fetched for entries in `entries`,
///                      keyed by the item ID they were fetched for.
///                      May include relations for items not in `entries`
///                      (e.g. the canonical parent of a residual).
/// * `include_superseded` – When `false` (default), entries whose
///                      `status == Superseded` are silently dropped.
///
/// # Returns
///
/// A `Vec<KnowledgeQueryResult>` sorted by descending authority:
///   1. Layer precedence (Tenant=4 > Org=3 > Team=2 > Project=1)
///   2. Variant-role rank (Canonical=0 < Specialization=1 < …)
///
/// Each `KnowledgeQueryResult` groups a primary entry with any residual
/// items that point to it via the residual relation types listed above.
pub fn resolve(
    entries: Vec<KnowledgeEntry>,
    all_relations: HashMap<String, Vec<KnowledgeRelation>>,
    include_superseded: bool,
) -> Vec<KnowledgeQueryResult> {
    // Step 1 – filter superseded unless caller explicitly wants them.
    let active: Vec<KnowledgeEntry> = if include_superseded {
        entries
    } else {
        entries
            .into_iter()
            .filter(|e| e.status != KnowledgeStatus::Superseded)
            .collect()
    };

    if active.is_empty() {
        return vec![];
    }

    // Build a quick-lookup map: path -> entry
    let entry_map: HashMap<String, KnowledgeEntry> = active
        .iter()
        .cloned()
        .map(|e| (e.path.clone(), e))
        .collect();

    // Step 2 – for each entry, collect its relations (source *or* target side).
    let enrich = |entry: &KnowledgeEntry| -> KnowledgeEntryWithRelations {
        let mut rels: Vec<KnowledgeRelation> = vec![];
        // relations keyed by this entry's path
        if let Some(r) = all_relations.get(&entry.path) {
            rels.extend(r.iter().cloned());
        }
        // also include relations where this item is the target
        for rel_list in all_relations.values() {
            for rel in rel_list {
                if rel.target_id == entry.path
                    && rel.source_id != entry.path
                    && !rels.iter().any(|r| r.id == rel.id)
                {
                    rels.push(rel.clone());
                }
            }
        }
        rels.sort_by_key(|r| r.created_at);
        KnowledgeEntryWithRelations::new(entry.clone(), rels)
    };

    // Step 3 – identify which entries are "residuals" (they have a residual
    // relation pointing to a canonical parent that is also in our result set).
    // Those residuals will be grouped under their primary rather than shown
    // as standalone results.
    let mut residual_paths: HashMap<String, (String, KnowledgeRelationType)> = HashMap::new();

    for entry in &active {
        if let Some(rels) = all_relations.get(&entry.path) {
            for rel in rels {
                if RESIDUAL_RELATION_TYPES.contains(&rel.relation_type) {
                    // This entry Specializes / ApplicableFrom / … its target.
                    // If the target also appears in our result set, treat this
                    // entry as a residual of the target.
                    if entry_map.contains_key(&rel.target_id) {
                        residual_paths.insert(
                            entry.path.clone(),
                            (rel.target_id.clone(), rel.relation_type),
                        );
                    }
                }
            }
        }
    }

    // Step 4 – build KnowledgeQueryResult groups.
    let mut results: Vec<KnowledgeQueryResult> = vec![];
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Primary entries: all entries that are NOT residuals of something in our set.
    let mut primaries: Vec<&KnowledgeEntry> = active
        .iter()
        .filter(|e| !residual_paths.contains_key(&e.path))
        .collect();

    // Sort primaries: highest layer first, then lowest role rank (Canonical first).
    primaries.sort_by(|a, b| {
        let layer_cmp = b.layer.precedence().cmp(&a.layer.precedence());
        if layer_cmp != std::cmp::Ordering::Equal {
            return layer_cmp;
        }
        a.variant_role().rank().cmp(&b.variant_role().rank())
    });

    for primary in primaries {
        if seen.contains(&primary.path) {
            continue;
        }
        seen.insert(primary.path.clone());

        let enriched_primary = enrich(primary);

        // Collect residuals that point to this primary.
        let mut local_residuals: Vec<(KnowledgeRelationType, KnowledgeEntryWithRelations)> = active
            .iter()
            .filter_map(|e| {
                residual_paths
                    .get(&e.path)
                    .and_then(|(parent_id, rel_type)| {
                        if parent_id == &primary.path {
                            seen.insert(e.path.clone());
                            Some((*rel_type, enrich(e)))
                        } else {
                            None
                        }
                    })
            })
            .collect();

        // Sort residuals by role rank so Specialization comes before Exception.
        local_residuals.sort_by_key(|(_, e)| e.entry.variant_role().rank());

        results.push(KnowledgeQueryResult {
            primary: enriched_primary,
            local_residuals,
        });
    }

    // Any entries that were marked as residuals but whose parent is NOT in the
    // result set (parent was filtered out or not in the search results) become
    // standalone primaries at the end.
    for entry in &active {
        if seen.contains(&entry.path) {
            continue;
        }
        seen.insert(entry.path.clone());
        results.push(KnowledgeQueryResult {
            primary: enrich(entry),
            local_residuals: vec![],
        });
    }

    results
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mk_core::types::{
        KnowledgeLayer, KnowledgeStatus, KnowledgeType, KnowledgeVariantRole, TenantId, UserId,
    };
    use std::collections::HashMap as HM;

    fn entry(
        path: &str,
        layer: KnowledgeLayer,
        status: KnowledgeStatus,
        role: Option<KnowledgeVariantRole>,
    ) -> KnowledgeEntry {
        let mut metadata = HM::new();
        if let Some(r) = role {
            metadata.insert("variant_role".to_string(), serde_json::json!(r.to_string()));
        }
        KnowledgeEntry {
            path: path.to_string(),
            content: format!("content of {path}"),
            layer,
            kind: KnowledgeType::Adr,
            status,
            summaries: HM::new(),
            metadata,
            commit_hash: None,
            author: None,
            updated_at: 0,
        }
    }

    fn rel(
        id: &str,
        source_id: &str,
        target_id: &str,
        rel_type: KnowledgeRelationType,
    ) -> KnowledgeRelation {
        KnowledgeRelation {
            id: id.to_string(),
            source_id: source_id.to_string(),
            target_id: target_id.to_string(),
            relation_type: rel_type,
            tenant_id: TenantId::new("t".to_string()).unwrap(),
            created_by: UserId::new("u".to_string()).unwrap(),
            created_at: 0,
            metadata: HM::new(),
        }
    }

    // ── 6.1 – Superseded items are filtered by default ────────────────────────

    #[test]
    fn test_superseded_items_excluded_by_default() {
        let entries = vec![
            entry(
                "a/active",
                KnowledgeLayer::Team,
                KnowledgeStatus::Accepted,
                None,
            ),
            entry(
                "a/superseded",
                KnowledgeLayer::Team,
                KnowledgeStatus::Superseded,
                None,
            ),
        ];
        let results = resolve(entries, HM::new(), false);
        assert_eq!(results.len(), 1, "superseded item should be filtered");
        assert_eq!(results[0].primary.entry.path, "a/active");
    }

    #[test]
    fn test_superseded_items_included_when_requested() {
        let entries = vec![
            entry(
                "a/active",
                KnowledgeLayer::Team,
                KnowledgeStatus::Accepted,
                None,
            ),
            entry(
                "a/superseded",
                KnowledgeLayer::Team,
                KnowledgeStatus::Superseded,
                None,
            ),
        ];
        let results = resolve(entries, HM::new(), true);
        assert_eq!(results.len(), 2);
    }

    // ── 6.1 – Layer precedence: Tenant ranks above Project ───────────────────

    #[test]
    fn test_higher_layer_ranks_first() {
        let entries = vec![
            entry(
                "p/proj",
                KnowledgeLayer::Project,
                KnowledgeStatus::Accepted,
                None,
            ),
            entry(
                "t/tenant",
                KnowledgeLayer::Tenant,
                KnowledgeStatus::Accepted,
                None,
            ),
            entry(
                "t/team",
                KnowledgeLayer::Team,
                KnowledgeStatus::Accepted,
                None,
            ),
        ];
        let results = resolve(entries, HM::new(), false);
        assert_eq!(results[0].primary.entry.layer, KnowledgeLayer::Tenant);
        assert_eq!(results[1].primary.entry.layer, KnowledgeLayer::Team);
        assert_eq!(results[2].primary.entry.layer, KnowledgeLayer::Project);
    }

    // ── 6.1 – Canonical ranks above Specialization at same layer ─────────────

    #[test]
    fn test_canonical_ranks_before_specialization_same_layer() {
        let entries = vec![
            entry(
                "t/spec",
                KnowledgeLayer::Team,
                KnowledgeStatus::Accepted,
                Some(KnowledgeVariantRole::Specialization),
            ),
            entry(
                "t/canon",
                KnowledgeLayer::Team,
                KnowledgeStatus::Accepted,
                Some(KnowledgeVariantRole::Canonical),
            ),
        ];
        let results = resolve(entries, HM::new(), false);
        assert_eq!(results[0].primary.entry.path, "t/canon");
        assert_eq!(results[1].primary.entry.path, "t/spec");
    }

    // ── 6.2 – Residual grouped under canonical ────────────────────────────────

    #[test]
    fn test_residual_grouped_under_canonical() {
        let canon = entry(
            "org/canon",
            KnowledgeLayer::Org,
            KnowledgeStatus::Accepted,
            Some(KnowledgeVariantRole::Canonical),
        );
        let residual = entry(
            "proj/residual",
            KnowledgeLayer::Project,
            KnowledgeStatus::Accepted,
            Some(KnowledgeVariantRole::Specialization),
        );

        // "proj/residual" Specializes "org/canon"
        let specializes_rel = rel(
            "r1",
            "proj/residual",
            "org/canon",
            KnowledgeRelationType::Specializes,
        );

        let mut all_rels: HM<String, Vec<KnowledgeRelation>> = HM::new();
        all_rels.insert("proj/residual".to_string(), vec![specializes_rel]);

        let results = resolve(vec![canon, residual], all_rels, false);

        assert_eq!(
            results.len(),
            1,
            "residual should be grouped under canonical"
        );
        assert_eq!(results[0].primary.entry.path, "org/canon");
        assert_eq!(results[0].local_residuals.len(), 1);
        assert_eq!(results[0].local_residuals[0].1.entry.path, "proj/residual");
        assert_eq!(
            results[0].local_residuals[0].0,
            KnowledgeRelationType::Specializes
        );
    }

    // ── 6.3 – Relations attached to entries ──────────────────────────────────

    #[test]
    fn test_relations_attached_to_returned_entries() {
        let e = entry(
            "p/item",
            KnowledgeLayer::Project,
            KnowledgeStatus::Accepted,
            None,
        );
        let r = rel(
            "r1",
            "p/item",
            "o/other",
            KnowledgeRelationType::DerivedFrom,
        );
        let mut all_rels: HM<String, Vec<KnowledgeRelation>> = HM::new();
        all_rels.insert("p/item".to_string(), vec![r.clone()]);

        let results = resolve(vec![e], all_rels, false);
        assert_eq!(results[0].primary.relations.len(), 1);
        assert_eq!(results[0].primary.relations[0].id, "r1");
    }

    // ── 6.2 – Residual without canonical parent becomes standalone ────────────

    #[test]
    fn test_orphan_residual_becomes_standalone() {
        let residual = entry(
            "proj/orphan",
            KnowledgeLayer::Project,
            KnowledgeStatus::Accepted,
            Some(KnowledgeVariantRole::Specialization),
        );
        // Points to a canonical that is NOT in the result set
        let r = rel(
            "r1",
            "proj/orphan",
            "org/missing",
            KnowledgeRelationType::Specializes,
        );
        let mut all_rels: HM<String, Vec<KnowledgeRelation>> = HM::new();
        all_rels.insert("proj/orphan".to_string(), vec![r]);

        let results = resolve(vec![residual], all_rels, false);
        assert_eq!(results.len(), 1, "orphan residual should appear standalone");
        assert_eq!(results[0].local_residuals.len(), 0);
    }

    // ── 6.1 – Empty input returns empty output ────────────────────────────────

    #[test]
    fn test_empty_input_returns_empty() {
        let results = resolve(vec![], HM::new(), false);
        assert!(results.is_empty());
    }
}
