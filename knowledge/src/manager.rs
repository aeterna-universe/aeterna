use crate::governance::GovernanceEngine;
use crate::repository::{GitRepository, RepositoryError};
use crate::telemetry::KnowledgeTelemetry;
use mk_core::traits::KnowledgeRepository;
use mk_core::types::{
    GovernanceEvent, KnowledgeEntry, KnowledgeEntryWithRelations, KnowledgeLayer,
    KnowledgeQueryResult, KnowledgeRelation, KnowledgeRelationType, KnowledgeStatus,
    KnowledgeVariantRole, PromotionDecision, PromotionMode, PromotionRequest,
    PromotionRequestStatus, TenantContext, ValidationResult,
};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Error, Debug)]
pub enum KnowledgeManagerError {
    #[error("Repository error: {0}")]
    Repository(#[from] RepositoryError),
    #[error("Governance violation: {0}")]
    Governance(String),
    #[error("Governance error: {0}")]
    GovernanceInternal(#[from] crate::governance::GovernanceError),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("Promotion not found: {0}")]
    PromotionNotFound(String),
    #[error("Invalid promotion state transition: {0}")]
    InvalidPromotionTransition(String),
    #[error("Stale promotion request: source item has changed since version {0}")]
    StalePromotion(String),
    #[error("Duplicate relation between {0} and {1}")]
    DuplicateRelation(String, String),
    #[error("Source item not found: {0}")]
    SourceNotFound(String),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Tenant mismatch: promotion belongs to tenant {0}")]
    TenantMismatch(String),
    #[error("Cross-tenant promotion is not permitted")]
    ForbiddenCrossTenant,
    #[error("Authorization failed: {0}")]
    Authorization(String),
    #[error("Confidential content cannot be promoted to broader layers")]
    ConfidentialContent,
    /// Task 8.1 — optimistic concurrency: caller's version token is out of date
    #[error(
        "Optimistic concurrency conflict: promotion was modified since version {0}; reload and retry"
    )]
    OptimisticConflict(i64),
    /// Task 8.5 — parallel promotion conflict: another promotion for the same source is already approved or applied
    #[error("Conflicting promotion: a promotion for source item {0} is already {1}")]
    ConflictingPromotion(String, String),
}

// ── Preview DTO ───────────────────────────────────────────────────────────────

/// Returned by `preview_promotion` to show what would be promoted and what
/// would remain in the lower layer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromotionPreview {
    pub source_item_id: String,
    pub source_layer: KnowledgeLayer,
    pub target_layer: KnowledgeLayer,
    pub mode: PromotionMode,
    /// Content that will be written to the target layer
    pub shared_content: String,
    /// Content that will remain / be stored as a residual entry (Partial only)
    pub residual_content: Option<String>,
    /// Governance policy violations found for this candidate
    pub governance_violations: Vec<String>,
}

// ── Precedence helpers ────────────────────────────────────────────────────────

/// Maps a KnowledgeLayer to its canonical authority rank.
/// Lower return value = higher authority (Tenant is most authoritative).
/// Used as the primary sort key in `query_with_precedence`.
fn layer_authority(layer: KnowledgeLayer) -> u8 {
    match layer {
        KnowledgeLayer::Tenant => 0,
        KnowledgeLayer::Org => 1,
        KnowledgeLayer::Team => 2,
        KnowledgeLayer::Project => 3,
    }
}

// ── Manager ───────────────────────────────────────────────────────────────────

pub struct KnowledgeManager {
    repository: Arc<GitRepository>,
    governance: Arc<GovernanceEngine>,
    /// Task 11.1 / 11.3 — promotion lifecycle metrics
    telemetry: KnowledgeTelemetry,
}

impl KnowledgeManager {
    pub fn new(repository: Arc<GitRepository>, governance: Arc<GovernanceEngine>) -> Self {
        Self {
            repository,
            governance,
            telemetry: KnowledgeTelemetry,
        }
    }

    fn contains_sensitive_content(content: &str) -> bool {
        const SENSITIVE_PATTERNS: &[&str] = &[
            "password",
            "secret",
            "api_key",
            "apikey",
            "private_key",
            "privatekey",
            "token",
            "credential",
            "access_key",
            "auth_key",
        ];
        let lower = content.to_lowercase();
        SENSITIVE_PATTERNS
            .iter()
            .any(|pattern| lower.contains(pattern))
    }

    fn enforce_cross_tenant_context(ctx: &TenantContext) -> Result<(), KnowledgeManagerError> {
        if let Some(target_tenant_id) = &ctx.target_tenant_id
            && target_tenant_id != &ctx.tenant_id
        {
            return Err(KnowledgeManagerError::ForbiddenCrossTenant);
        }
        Ok(())
    }

    fn enforce_tenant_match(
        ctx: &TenantContext,
        tenant_id: &mk_core::types::TenantId,
    ) -> Result<(), KnowledgeManagerError> {
        Self::enforce_cross_tenant_context(ctx)?;
        if &ctx.tenant_id != tenant_id {
            return Err(KnowledgeManagerError::TenantMismatch(tenant_id.to_string()));
        }
        Ok(())
    }

    fn enforce_reviewer_layer_authority(
        ctx: &TenantContext,
        target_layer: KnowledgeLayer,
    ) -> Result<(), KnowledgeManagerError> {
        use mk_core::types::Role;

        let highest_role = ctx
            .highest_precedence_role()
            .and_then(|role| match role {
                mk_core::types::RoleIdentifier::Known(role) => Some(role),
                mk_core::types::RoleIdentifier::Custom(_) => None,
            })
            .ok_or_else(|| {
                KnowledgeManagerError::Authorization(
                    "No reviewer role available for promotion lifecycle action".to_string(),
                )
            })?;

        let allowed = match highest_role {
            Role::PlatformAdmin | Role::TenantAdmin | Role::Admin => true,
            Role::Architect => matches!(target_layer, KnowledgeLayer::Team | KnowledgeLayer::Org),
            Role::TechLead => matches!(target_layer, KnowledgeLayer::Team),
            _ => false,
        };

        if !allowed {
            return Err(KnowledgeManagerError::Authorization(format!(
                "Role {:?} is not authorized to review promotions targeting {:?}",
                highest_role, target_layer
            )));
        }

        Ok(())
    }

    async fn enforce_promotion_policy(
        &self,
        ctx: &TenantContext,
        target_layer: KnowledgeLayer,
        shared_content: &str,
        residual_content: Option<&str>,
        action: &str,
    ) -> Result<(), KnowledgeManagerError> {
        if Self::contains_sensitive_content(shared_content)
            || residual_content.is_some_and(Self::contains_sensitive_content)
        {
            return Err(KnowledgeManagerError::ConfidentialContent);
        }

        let mut gov_ctx = HashMap::new();
        gov_ctx.insert("action".to_string(), serde_json::json!(action));
        gov_ctx.insert("content".to_string(), serde_json::json!(shared_content));
        gov_ctx.insert("layer".to_string(), serde_json::json!(target_layer));
        if let Some(residual_content) = residual_content {
            gov_ctx.insert(
                "residual_content".to_string(),
                serde_json::json!(residual_content),
            );
        }

        let validation = self
            .governance
            .validate_with_context(target_layer, &gov_ctx, Some(ctx))
            .await?;

        let blocking_messages: Vec<String> = validation
            .violations
            .iter()
            .filter(|violation| violation.severity == mk_core::types::ConstraintSeverity::Block)
            .map(|violation| violation.message.clone())
            .collect();

        if !blocking_messages.is_empty() {
            return Err(KnowledgeManagerError::Governance(
                blocking_messages.join("; "),
            ));
        }

        Ok(())
    }

    // ── Existing methods (unchanged) ─────────────────────────────────────────

    #[tracing::instrument(skip_all, fields(layer = ?entry.layer, path = %entry.path))]
    pub async fn add(
        &self,
        ctx: TenantContext,
        entry: KnowledgeEntry,
        message: &str,
    ) -> Result<String, KnowledgeManagerError> {
        let mut context = HashMap::new();
        context.insert("path".to_string(), serde_json::json!(entry.path));
        context.insert("content".to_string(), serde_json::json!(entry.content));
        context.insert("layer".to_string(), serde_json::json!(entry.layer));

        let validation = self
            .governance
            .validate_with_context(entry.layer, &context, Some(&ctx))
            .await?;

        if !validation.is_valid {
            let errors: Vec<String> = validation
                .violations
                .iter()
                .map(|v| v.message.clone())
                .collect();
            return Err(KnowledgeManagerError::Governance(errors.join(", ")));
        }

        let commit_hash = self.repository.store(ctx, entry, message).await?;
        Ok(commit_hash)
    }

    #[tracing::instrument(skip_all, fields(limit))]
    pub async fn query(
        &self,
        ctx: TenantContext,
        query: &str,
        layers: Vec<KnowledgeLayer>,
        limit: usize,
    ) -> Result<Vec<KnowledgeEntry>, KnowledgeManagerError> {
        let results = self.repository.search(ctx, query, layers, limit).await?;
        Ok(results)
    }

    // ── Task 6.1: canonical-vs-residual precedence ────────────────────────────

    /// Returns all entries matching `query` across `layers`, sorted by
    /// canonical-vs-residual precedence rules:
    ///
    /// 1. Within a layer, Canonical entries rank highest (precedence 5),
    ///    then Clarification (4), Specialization (3), Applicability (2),
    ///    Exception (1).
    /// 2. Across layers the canonical authority order is
    ///    Tenant > Org > Team > Project (higher layer = more authoritative).
    /// 3. Within the same variant-role and layer, entries are ordered by
    ///    `updated_at` descending (most recent first).
    ///
    /// This is the low-level ordering function.  Most callers should prefer
    /// `query_enriched` which also loads relations.
    #[tracing::instrument(skip_all, fields(limit))]
    pub async fn query_with_precedence(
        &self,
        ctx: TenantContext,
        query: &str,
        layers: Vec<KnowledgeLayer>,
        limit: usize,
    ) -> Result<Vec<KnowledgeEntry>, KnowledgeManagerError> {
        let mut results = self.repository.search(ctx, query, layers, limit).await?;

        results.sort_by(|a, b| {
            // Primary key: layer authority (Tenant=0 is highest)
            let layer_cmp = layer_authority(a.layer).cmp(&layer_authority(b.layer));
            if layer_cmp != std::cmp::Ordering::Equal {
                return layer_cmp;
            }
            // Secondary key: variant role precedence (Canonical=5 is highest → reverse)
            let role_cmp = b.variant_precedence().cmp(&a.variant_precedence());
            if role_cmp != std::cmp::Ordering::Equal {
                return role_cmp;
            }
            // Tertiary key: most recently updated first
            b.updated_at.cmp(&a.updated_at)
        });

        Ok(results)
    }

    // ── Task 6.2: enriched query with canonical + local residual context ───────

    /// Returns search results grouped as `KnowledgeQueryResult` items.
    ///
    /// Each result contains a `primary` entry (the most authoritative match —
    /// typically a Canonical item at the highest available layer) together
    /// with any directly related local residuals (Specialization, Applicability,
    /// Exception, Clarification) found via stored `KnowledgeRelation` records.
    ///
    /// Grouping algorithm:
    /// 1. Fetch raw results via `query_with_precedence` (canonical-first ordering).
    /// 2. For each result, load its relations.
    /// 3. Identify residual entries whose path appears in the raw result set
    ///    and that are linked to the primary via Specializes / ApplicableFrom /
    ///    ExceptionTo / Clarifies.
    /// 4. De-duplicate: once an entry is attached as a residual it is removed
    ///    from the primary candidate set.
    #[tracing::instrument(skip_all, fields(limit))]
    pub async fn query_enriched(
        &self,
        ctx: TenantContext,
        query: &str,
        layers: Vec<KnowledgeLayer>,
        limit: usize,
    ) -> Result<Vec<KnowledgeQueryResult>, KnowledgeManagerError> {
        // Step 1 – precedence-sorted flat list
        let sorted = self
            .query_with_precedence(ctx.clone(), query, layers, limit)
            .await?;

        // Step 2 – load relations for every entry; build path → entry map
        let mut path_to_entry: HashMap<String, KnowledgeEntry> =
            sorted.iter().map(|e| (e.path.clone(), e.clone())).collect();

        // Relation load: best-effort; errors are logged but do not fail the query
        let mut path_to_relations: HashMap<String, Vec<KnowledgeRelation>> = HashMap::new();
        for entry in &sorted {
            match self
                .repository
                .get_relations_for_item(ctx.clone(), &entry.path)
                .await
            {
                Ok(rels) => {
                    path_to_relations.insert(entry.path.clone(), rels);
                }
                Err(e) => {
                    tracing::warn!(path = %entry.path, error = %e, "failed to load relations for enriched query");
                }
            }
        }

        // Step 3 – group into KnowledgeQueryResult
        // Relation types that qualify an entry as a "local residual" of another
        const RESIDUAL_RELATION_TYPES: &[KnowledgeRelationType] = &[
            KnowledgeRelationType::Specializes,
            KnowledgeRelationType::ApplicableFrom,
            KnowledgeRelationType::ExceptionTo,
            KnowledgeRelationType::Clarifies,
        ];

        let mut consumed: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut results: Vec<KnowledgeQueryResult> = Vec::new();

        for primary_entry in &sorted {
            if consumed.contains(&primary_entry.path) {
                continue;
            }

            let primary_relations = path_to_relations
                .get(&primary_entry.path)
                .cloned()
                .unwrap_or_default();

            // Identify relations that point to entries in our result set
            let mut local_residuals: Vec<(KnowledgeRelationType, KnowledgeEntryWithRelations)> =
                Vec::new();

            for rel in &primary_relations {
                if !RESIDUAL_RELATION_TYPES.contains(&rel.relation_type) {
                    continue;
                }
                // The residual is the "other" end of the relation
                let residual_path = if rel.source_id == primary_entry.path {
                    &rel.target_id
                } else {
                    &rel.source_id
                };

                if consumed.contains(residual_path) {
                    continue;
                }
                if let Some(residual_entry) = path_to_entry.remove(residual_path) {
                    consumed.insert(residual_path.clone());
                    let residual_rels = path_to_relations
                        .get(residual_path)
                        .cloned()
                        .unwrap_or_default();
                    local_residuals.push((
                        rel.relation_type,
                        KnowledgeEntryWithRelations::new(residual_entry, residual_rels),
                    ));
                }
            }

            consumed.insert(primary_entry.path.clone());

            results.push(KnowledgeQueryResult {
                primary: KnowledgeEntryWithRelations::new(primary_entry.clone(), primary_relations),
                local_residuals,
            });
        }

        Ok(results)
    }

    #[tracing::instrument(skip_all, fields(layer = ?layer))]
    pub async fn check_constraints(
        &self,
        ctx: TenantContext,
        layer: KnowledgeLayer,
        context: HashMap<String, serde_json::Value>,
    ) -> Result<ValidationResult, KnowledgeManagerError> {
        let result = self
            .governance
            .validate_with_context(layer, &context, Some(&ctx))
            .await?;
        Ok(result)
    }

    #[tracing::instrument(skip_all, fields(layer = ?layer, prefix))]
    pub async fn list(
        &self,
        ctx: TenantContext,
        layer: KnowledgeLayer,
        prefix: &str,
    ) -> Result<Vec<KnowledgeEntry>, KnowledgeManagerError> {
        let entries = self.repository.list(ctx, layer, prefix).await?;
        Ok(entries)
    }

    #[tracing::instrument(skip_all, fields(layer = ?layer, path))]
    pub async fn get(
        &self,
        ctx: TenantContext,
        layer: KnowledgeLayer,
        path: &str,
    ) -> Result<Option<KnowledgeEntry>, KnowledgeManagerError> {
        let entry = self.repository.get(ctx, layer, path).await?;
        Ok(entry)
    }

    #[tracing::instrument(skip_all)]
    pub async fn get_head_commit(
        &self,
        ctx: TenantContext,
    ) -> Result<Option<String>, KnowledgeManagerError> {
        Ok(self.repository.get_head_commit(ctx).await?)
    }

    #[tracing::instrument(skip_all, fields(since_commit))]
    pub async fn get_affected_items(
        &self,
        ctx: TenantContext,
        since_commit: &str,
    ) -> Result<Vec<(KnowledgeLayer, String)>, KnowledgeManagerError> {
        Ok(self
            .repository
            .get_affected_items(ctx, since_commit)
            .await?)
    }

    pub fn root_path(&self) -> Option<std::path::PathBuf> {
        Some(self.repository.root_path().to_path_buf())
    }

    /// Proxy for test access to relations stored in the repository.
    pub async fn repository_get_relations_for_item(
        &self,
        ctx: TenantContext,
        item_id: &str,
    ) -> Result<Vec<KnowledgeRelation>, KnowledgeManagerError> {
        Ok(self.repository.get_relations_for_item(ctx, item_id).await?)
    }

    pub async fn repository_get(
        &self,
        ctx: TenantContext,
        item_id: &str,
    ) -> Result<Option<KnowledgeEntry>, KnowledgeManagerError> {
        for layer in [
            KnowledgeLayer::Project,
            KnowledgeLayer::Team,
            KnowledgeLayer::Org,
            KnowledgeLayer::Tenant,
        ] {
            if let Some(entry) = self.repository.get(ctx.clone(), layer, item_id).await? {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }

    pub async fn get_promotion_request(
        &self,
        ctx: TenantContext,
        promotion_id: &str,
    ) -> Result<Option<PromotionRequest>, KnowledgeManagerError> {
        Ok(self
            .repository
            .get_promotion_request(ctx, promotion_id)
            .await?)
    }

    pub async fn list_promotion_requests(
        &self,
        ctx: TenantContext,
        status: Option<PromotionRequestStatus>,
    ) -> Result<Vec<PromotionRequest>, KnowledgeManagerError> {
        Ok(self.repository.list_promotion_requests(ctx, status).await?)
    }

    // ── Task 2.1 — Promotion preview ─────────────────────────────────────────

    /// Returns what would be promoted and what would remain without persisting
    /// anything. Validates layer direction and governance policies.
    #[tracing::instrument(skip_all, fields(source_item_id, ?target_layer, ?mode, lifecycle_stage = "preview"))]
    pub async fn preview_promotion(
        &self,
        ctx: TenantContext,
        source_item_id: &str,
        target_layer: KnowledgeLayer,
        mode: PromotionMode,
    ) -> Result<PromotionPreview, KnowledgeManagerError> {
        Self::enforce_cross_tenant_context(&ctx)?;

        // Locate source across layers
        let entry = self
            .find_entry_by_id(ctx.clone(), source_item_id)
            .await?
            .ok_or_else(|| KnowledgeManagerError::SourceNotFound(source_item_id.to_string()))?;

        // Validate direction (reuse PromotionRequest's validator via a temp struct)
        let probe = PromotionRequest {
            id: String::new(),
            source_item_id: source_item_id.to_string(),
            source_layer: entry.layer,
            source_status: entry.status,
            target_layer,
            promotion_mode: mode,
            shared_content: entry.content.clone(),
            residual_content: None,
            residual_role: None,
            justification: None,
            status: PromotionRequestStatus::Draft,
            requested_by: ctx.user_id.clone(),
            tenant_id: ctx.tenant_id.clone(),
            source_version: entry.commit_hash.clone().unwrap_or_default(),
            latest_decision: None,
            promoted_item_id: None,
            residual_item_id: None,
            created_at: 0,
            updated_at: 0,
        };
        probe
            .validate_layer_direction()
            .map_err(KnowledgeManagerError::Validation)?;

        self.enforce_promotion_policy(
            &ctx,
            target_layer,
            &entry.content,
            None,
            "promotion_preview",
        )
        .await?;

        // Check governance constraints against target layer
        let mut gov_ctx = HashMap::new();
        gov_ctx.insert("content".to_string(), serde_json::json!(entry.content));
        gov_ctx.insert("layer".to_string(), serde_json::json!(target_layer));
        let validation = self
            .governance
            .validate_with_context(target_layer, &gov_ctx, Some(&ctx))
            .await?;
        let violations: Vec<String> = validation
            .violations
            .iter()
            .map(|v| v.message.clone())
            .collect();

        let (shared_content, residual_content) = match mode {
            PromotionMode::Full => (entry.content.clone(), None),
            PromotionMode::Partial => {
                // By default preview: full content is shared, residual is empty (caller
                // can specify actual split when calling create_promotion_request)
                (entry.content.clone(), Some(String::new()))
            }
        };

        Ok(PromotionPreview {
            source_item_id: source_item_id.to_string(),
            source_layer: entry.layer,
            target_layer,
            mode,
            shared_content,
            residual_content,
            governance_violations: violations,
        })
    }

    // ── Task 2.2 — Create promotion request ──────────────────────────────────

    #[tracing::instrument(skip_all, fields(source_item_id = %request.source_item_id, ?request.source_layer, ?request.target_layer, ?request.promotion_mode))]
    pub async fn create_promotion_request(
        &self,
        ctx: TenantContext,
        mut request: PromotionRequest,
    ) -> Result<PromotionRequest, KnowledgeManagerError> {
        Self::enforce_tenant_match(&ctx, &request.tenant_id)?;

        // Structural validation
        request
            .validate_layer_direction()
            .map_err(KnowledgeManagerError::Validation)?;

        // Verify source still exists and version matches
        let entry = self
            .find_entry_by_id(ctx.clone(), &request.source_item_id)
            .await?
            .ok_or_else(|| KnowledgeManagerError::SourceNotFound(request.source_item_id.clone()))?;

        let current_version = entry.commit_hash.clone().unwrap_or_default();
        if !request.source_version.is_empty() && current_version != request.source_version {
            return Err(KnowledgeManagerError::StalePromotion(
                request.source_version.clone(),
            ));
        }
        request.source_version = current_version;

        self.enforce_promotion_policy(
            &ctx,
            request.target_layer,
            &request.shared_content,
            request.residual_content.as_deref(),
            "promotion_submit",
        )
        .await?;

        // Assign id and timestamps if not already set
        if request.id.is_empty() {
            request.id = uuid::Uuid::new_v4().to_string();
        }
        let now = chrono::Utc::now().timestamp();
        request.created_at = now;
        request.updated_at = now;
        request.status = PromotionRequestStatus::PendingReview;

        let stored = self
            .repository
            .store_promotion_request(ctx.clone(), request)
            .await?;

        // Task 9.1 / 9.7 — emit event with split mode + justification audit metadata
        let _ = self
            .governance
            .publish_event(GovernanceEvent::KnowledgePromotionRequested {
                promotion_id: stored.id.clone(),
                source_item_id: stored.source_item_id.clone(),
                source_layer: stored.source_layer,
                target_layer: stored.target_layer,
                promotion_mode: stored.promotion_mode,
                justification: stored.justification.clone(),
                requested_by: stored.requested_by.clone(),
                tenant_id: stored.tenant_id.clone(),
                timestamp: stored.created_at,
            })
            .await;

        // Task 11.1 — metric: new promotion request submitted
        self.telemetry.record_promotion_requested(
            &format!("{:?}", stored.source_layer),
            &format!("{:?}", stored.target_layer),
        );

        Ok(stored)
    }

    // ── Task 2.3 — Approve ────────────────────────────────────────────────────

    #[tracing::instrument(skip_all, fields(promotion_id, ?decision, lifecycle_stage = "approve"))]
    pub async fn approve_promotion(
        &self,
        ctx: TenantContext,
        promotion_id: &str,
        decision: PromotionDecision,
        client_version: Option<i64>,
    ) -> Result<PromotionRequest, KnowledgeManagerError> {
        let mut req = self.load_promotion(ctx.clone(), promotion_id).await?;

        Self::enforce_tenant_match(&ctx, &req.tenant_id)?;
        Self::enforce_reviewer_layer_authority(&ctx, req.target_layer)?;

        self.enforce_promotion_policy(
            &ctx,
            req.target_layer,
            &req.shared_content,
            req.residual_content.as_deref(),
            "promotion_approve",
        )
        .await?;

        // Task 8.1 — optimistic concurrency: reject if caller's version is stale
        if let Some(v) = client_version
            && v != req.updated_at
        {
            return Err(KnowledgeManagerError::OptimisticConflict(req.updated_at));
        }

        // Task 8.2 — idempotency: already approved → return existing result
        if req.status == PromotionRequestStatus::Approved {
            return Ok(req);
        }

        // Only PendingReview → Approved is valid
        if req.status != PromotionRequestStatus::PendingReview {
            return Err(KnowledgeManagerError::InvalidPromotionTransition(format!(
                "cannot approve a request in {:?} state",
                req.status
            )));
        }

        // Reject decisions are not approvals
        if matches!(
            decision,
            PromotionDecision::Reject | PromotionDecision::NeedsRefinement
        ) {
            return Err(KnowledgeManagerError::InvalidPromotionTransition(
                "use reject_promotion for Reject/NeedsRefinement decisions".to_string(),
            ));
        }

        // Task 8.4 — stale source check: re-verify source version at approval time
        if !req.source_version.is_empty() {
            let source = self
                .find_entry_by_id(ctx.clone(), &req.source_item_id)
                .await?
                .ok_or_else(|| KnowledgeManagerError::SourceNotFound(req.source_item_id.clone()))?;
            let current_version = source.commit_hash.clone().unwrap_or_default();
            if current_version != req.source_version {
                return Err(KnowledgeManagerError::StalePromotion(
                    req.source_version.clone(),
                ));
            }
        }

        // Task 8.5 — parallel conflict: reject if another promotion for same source is already approved/applied
        let all_promotions = self
            .repository
            .list_promotion_requests(ctx.clone(), None)
            .await?;
        for other in &all_promotions {
            if other.id != req.id
                && other.source_item_id == req.source_item_id
                && matches!(
                    other.status,
                    PromotionRequestStatus::Approved | PromotionRequestStatus::Applied
                )
            {
                let conflict_type =
                    format!("parallel_{}", format!("{:?}", other.status).to_lowercase());
                self.telemetry.record_promotion_conflict(&conflict_type);
                return Err(KnowledgeManagerError::ConflictingPromotion(
                    req.source_item_id.clone(),
                    format!("{:?}", other.status),
                ));
            }
        }

        req.status = PromotionRequestStatus::Approved;
        req.latest_decision = Some(decision);
        req.updated_at = chrono::Utc::now().timestamp();

        let stored = self
            .repository
            .update_promotion_request(ctx.clone(), req)
            .await?;

        // Task 9.2
        let approver = ctx.user_id.clone();
        let _ = self
            .governance
            .publish_event(GovernanceEvent::KnowledgePromotionApproved {
                promotion_id: stored.id.clone(),
                decision,
                approved_by: approver,
                tenant_id: stored.tenant_id.clone(),
                timestamp: stored.updated_at,
            })
            .await;

        // Task 11.1 — metric: promotion approved, latency from creation to approval
        let approval_latency_ms = ((stored.updated_at - stored.created_at) as f64) * 1000.0;
        self.telemetry
            .record_promotion_approved(&format!("{:?}", stored.target_layer), approval_latency_ms);
        Ok(stored)
    }

    // ── Task 2.4 — Reject ─────────────────────────────────────────────────────

    #[tracing::instrument(skip_all, fields(promotion_id, lifecycle_stage = "reject"))]
    pub async fn reject_promotion(
        &self,
        ctx: TenantContext,
        promotion_id: &str,
        reason: &str,
        client_version: Option<i64>,
    ) -> Result<PromotionRequest, KnowledgeManagerError> {
        let mut req = self.load_promotion(ctx.clone(), promotion_id).await?;

        Self::enforce_tenant_match(&ctx, &req.tenant_id)?;
        Self::enforce_reviewer_layer_authority(&ctx, req.target_layer)?;

        // Task 8.1 — optimistic concurrency
        if let Some(v) = client_version
            && v != req.updated_at
        {
            return Err(KnowledgeManagerError::OptimisticConflict(req.updated_at));
        }

        // Task 8.2 — idempotency: already rejected → return existing result
        if req.status == PromotionRequestStatus::Rejected {
            return Ok(req);
        }

        if req.status != PromotionRequestStatus::PendingReview {
            return Err(KnowledgeManagerError::InvalidPromotionTransition(format!(
                "cannot reject a request in {:?} state",
                req.status
            )));
        }

        req.status = PromotionRequestStatus::Rejected;
        req.latest_decision = Some(PromotionDecision::Reject);
        req.updated_at = chrono::Utc::now().timestamp();

        let stored = self
            .repository
            .update_promotion_request(ctx.clone(), req)
            .await?;

        // Task 9.3 — source item is NOT touched (task 2.4 invariant)
        let rejector = ctx.user_id.clone();
        let _ = self
            .governance
            .publish_event(GovernanceEvent::KnowledgePromotionRejected {
                promotion_id: stored.id.clone(),
                reason: reason.to_string(),
                rejected_by: rejector,
                tenant_id: stored.tenant_id.clone(),
                timestamp: stored.updated_at,
            })
            .await;

        // Task 11.1 — metric: promotion rejected
        self.telemetry
            .record_promotion_rejected(&format!("{:?}", stored.target_layer), "manual");
        Ok(stored)
    }

    // ── Task 2.5 — Retarget ───────────────────────────────────────────────────

    #[tracing::instrument(skip_all, fields(promotion_id, ?new_target_layer, lifecycle_stage = "retarget"))]
    pub async fn retarget_promotion(
        &self,
        ctx: TenantContext,
        promotion_id: &str,
        new_target_layer: KnowledgeLayer,
        client_version: Option<i64>,
    ) -> Result<PromotionRequest, KnowledgeManagerError> {
        let mut req = self.load_promotion(ctx.clone(), promotion_id).await?;

        Self::enforce_tenant_match(&ctx, &req.tenant_id)?;
        Self::enforce_reviewer_layer_authority(&ctx, req.target_layer)?;

        // Task 8.1 — optimistic concurrency
        if let Some(v) = client_version
            && v != req.updated_at
        {
            return Err(KnowledgeManagerError::OptimisticConflict(req.updated_at));
        }

        if req.status != PromotionRequestStatus::PendingReview {
            return Err(KnowledgeManagerError::InvalidPromotionTransition(format!(
                "cannot retarget a request in {:?} state",
                req.status
            )));
        }

        // Validate new target is still upward
        let probe = PromotionRequest {
            target_layer: new_target_layer,
            ..req.clone()
        };
        probe
            .validate_layer_direction()
            .map_err(KnowledgeManagerError::Validation)?;

        Self::enforce_reviewer_layer_authority(&ctx, new_target_layer)?;

        req.target_layer = new_target_layer;
        req.latest_decision = Some(PromotionDecision::RetargetLayer);
        req.updated_at = chrono::Utc::now().timestamp();

        let stored = self
            .repository
            .update_promotion_request(ctx.clone(), req)
            .await?;

        // Task 9.4
        let actor = ctx.user_id.clone();
        let _ = self
            .governance
            .publish_event(GovernanceEvent::KnowledgePromotionRetargeted {
                promotion_id: stored.id.clone(),
                new_target_layer,
                retargeted_by: actor,
                tenant_id: stored.tenant_id.clone(),
                timestamp: stored.updated_at,
            })
            .await;

        // Task 11.1 — metric: promotion retargeted
        self.telemetry.record_promotion_retargeted(
            &format!("{:?}", stored.source_layer),
            &format!("{:?}", stored.target_layer),
        );
        Ok(stored)
    }

    // ── Tasks 2.6, 2.7, 2.8, 2.9, 2.10 — Apply ──────────────────────────────

    /// Applies an Approved promotion request:
    /// - Copies source content to target layer (non-destructive, task 2.7)
    /// - For PromotionMode::Full: marks source Superseded (task 2.8)
    /// - For PromotionMode::Partial: stores residual as new entry at source layer (task 2.9)
    /// - Persists PromotedFrom/PromotedTo + variant-role relations (task 2.10)
    ///
    /// Task 11.3: emits  counter on any error.
    #[tracing::instrument(skip_all, fields(promotion_id, lifecycle_stage = "apply"))]
    pub async fn apply_promotion(
        &self,
        ctx: TenantContext,
        promotion_id: &str,
    ) -> Result<PromotionRequest, KnowledgeManagerError> {
        let result = self.apply_promotion_inner(ctx, promotion_id).await;
        if let Err(ref e) = result {
            // Task 11.3 — alert-grade counter: any apply failure increments this
            self.telemetry.record_promotion_apply_failed(&e.to_string());
        }
        result
    }

    async fn apply_promotion_inner(
        &self,
        ctx: TenantContext,
        promotion_id: &str,
    ) -> Result<PromotionRequest, KnowledgeManagerError> {
        let mut req = self.load_promotion(ctx.clone(), promotion_id).await?;

        Self::enforce_tenant_match(&ctx, &req.tenant_id)?;

        // Task 8.2 / 8.3 — idempotency: already applied → return existing (no duplicate items)
        if req.status == PromotionRequestStatus::Applied {
            return Ok(req);
        }

        if req.status != PromotionRequestStatus::Approved {
            return Err(KnowledgeManagerError::InvalidPromotionTransition(format!(
                "cannot apply a request in {:?} state; must be Approved first",
                req.status
            )));
        }

        // Load source entry
        let source_entry = self
            .find_entry_by_id(ctx.clone(), &req.source_item_id)
            .await?
            .ok_or_else(|| KnowledgeManagerError::SourceNotFound(req.source_item_id.clone()))?;

        let now = chrono::Utc::now().timestamp();
        let tenant_id = ctx.tenant_id.clone();
        let actor = ctx.user_id.clone();

        // ── Build promoted entry ───────────────────────────────────────────────
        let promoted_id = uuid::Uuid::new_v4().to_string();
        let promoted_path = format!("promoted/{}", promoted_id);
        let promoted_entry = KnowledgeEntry {
            path: promoted_path.clone(),
            content: req.shared_content.clone(),
            layer: req.target_layer,
            kind: source_entry.kind,
            status: KnowledgeStatus::Accepted,
            summaries: source_entry.summaries.clone(),
            metadata: {
                let mut m = source_entry.metadata.clone();
                m.insert(
                    "variant_role".to_string(),
                    serde_json::json!(KnowledgeVariantRole::Canonical),
                );
                m.insert(
                    "promoted_from_id".to_string(),
                    serde_json::json!(req.source_item_id),
                );
                m
            },
            commit_hash: None,
            author: Some(actor.to_string()),
            updated_at: now,
        };

        // task 2.7 — non-destructive: store promoted entry WITHOUT touching source
        self.repository
            .store(
                ctx.clone(),
                promoted_entry,
                &format!("[promote] {promotion_id}"),
            )
            .await?;

        req.promoted_item_id = Some(promoted_path.clone());

        // ── Task 2.8 / 2.9 — mode-specific source handling ───────────────────
        let mut residual_item_id: Option<String> = None;

        match req.promotion_mode {
            PromotionMode::Full => {
                // Task 2.8: mark source Superseded only for Full replacement
                self.repository
                    .update_status(
                        ctx.clone(),
                        source_entry.layer,
                        &source_entry.path,
                        KnowledgeStatus::Superseded,
                        &format!("[supersede] promoted to {:?}", req.target_layer),
                    )
                    .await?;
            }
            PromotionMode::Partial => {
                // Task 2.9: store residual content as a new entry at source layer
                if let Some(residual_content) = &req.residual_content
                    && !residual_content.is_empty()
                {
                    let residual_id = uuid::Uuid::new_v4().to_string();
                    let residual_path = format!("residual/{}", residual_id);
                    let variant_role = req
                        .residual_role
                        .unwrap_or(KnowledgeVariantRole::Specialization);
                    let residual_entry = KnowledgeEntry {
                        path: residual_path.clone(),
                        content: residual_content.clone(),
                        layer: source_entry.layer,
                        kind: source_entry.kind,
                        status: KnowledgeStatus::Accepted,
                        summaries: HashMap::new(),
                        metadata: {
                            let mut m = HashMap::new();
                            m.insert("variant_role".to_string(), serde_json::json!(variant_role));
                            m.insert(
                                "residual_from_id".to_string(),
                                serde_json::json!(req.source_item_id),
                            );
                            m
                        },
                        commit_hash: None,
                        author: Some(actor.to_string()),
                        updated_at: now,
                    };
                    self.repository
                        .store(
                            ctx.clone(),
                            residual_entry,
                            &format!("[residual] {promotion_id}"),
                        )
                        .await?;
                    residual_item_id = Some(residual_path.clone());
                    req.residual_item_id = Some(residual_path.clone());
                }
            }
        }

        // ── Task 2.10 — persist semantic relations ────────────────────────────
        self.persist_promotion_relations(
            ctx.clone(),
            &req.source_item_id,
            &promoted_path,
            residual_item_id.as_deref(),
            req.latest_decision,
            now,
            &tenant_id,
            &actor,
        )
        .await?;

        // Finalize request as Applied
        req.status = PromotionRequestStatus::Applied;
        req.updated_at = now;

        let stored = self
            .repository
            .update_promotion_request(ctx.clone(), req)
            .await?;

        // Task 9.5 / 9.7 — include split mode in applied event for audit trail
        let _ = self
            .governance
            .publish_event(GovernanceEvent::KnowledgePromotionApplied {
                promotion_id: stored.id.clone(),
                promoted_item_id: promoted_id,
                residual_item_id,
                promotion_mode: stored.promotion_mode,
                tenant_id: stored.tenant_id.clone(),
                timestamp: stored.updated_at,
            })
            .await;

        // Task 11.1 — metric: promotion successfully applied
        self.telemetry
            .record_operation("promotion_apply", "success");

        Ok(stored)
    }

    // ── Task 10: Migration and compatibility helpers ───────────────────────────

    /// Task 10.1 / 10.4 — Backfill `variant_role` metadata for existing knowledge items.
    ///
    /// Scans all four knowledge layers and, for each entry that does not yet
    /// have a `variant_role` key in its metadata, assigns a sensible default
    /// based on the entry's current status:
    ///
    /// | Status        | Assigned variant_role |
    /// |---------------|----------------------|
    /// | Accepted      | Canonical            |
    /// | Superseded    | Superseded (marker)  |
    /// | Deprecated    | Superseded (marker)  |
    /// | Proposed      | (skipped)            |
    /// | Rejected      | (skipped)            |
    ///
    /// Returns the total number of entries that were updated.
    ///
    /// # Idempotency
    /// Already-migrated entries (those with an existing `variant_role`) are
    /// skipped, so the function is safe to call more than once.
    #[tracing::instrument(skip_all)]
    pub async fn backfill_variant_roles(
        &self,
        ctx: TenantContext,
    ) -> Result<usize, KnowledgeManagerError> {
        let layers = [
            KnowledgeLayer::Tenant,
            KnowledgeLayer::Org,
            KnowledgeLayer::Team,
            KnowledgeLayer::Project,
        ];

        let mut updated = 0usize;

        for layer in layers {
            let entries = self.repository.list(ctx.clone(), layer, "").await?;

            for mut entry in entries {
                // Skip entries that already have a variant_role assigned
                if entry.metadata.contains_key("variant_role") {
                    continue;
                }

                let role = match entry.status {
                    KnowledgeStatus::Accepted => KnowledgeVariantRole::Canonical,
                    KnowledgeStatus::Superseded | KnowledgeStatus::Deprecated => {
                        KnowledgeVariantRole::Superseded
                    }
                    // Proposed / Rejected have no promotion role yet
                    _ => continue,
                };

                entry
                    .metadata
                    .insert("variant_role".to_string(), serde_json::json!(role));

                self.repository
                    .store(ctx.clone(), entry, "[migrate] backfill variant_role")
                    .await?;

                updated += 1;
            }
        }

        tracing::info!(
            count = updated,
            "Backfilled variant_role for knowledge entries"
        );
        Ok(updated)
    }

    // ── Task 2.10 helper — explicit relation creation ─────────────────────────

    #[tracing::instrument(skip_all)]
    pub async fn create_relation(
        &self,
        ctx: TenantContext,
        relation: KnowledgeRelation,
    ) -> Result<KnowledgeRelation, KnowledgeManagerError> {
        Self::enforce_tenant_match(&ctx, &relation.tenant_id)?;

        // Duplicate check: same (source, target, type) already exists?
        let existing = self
            .repository
            .get_relations_for_item(ctx.clone(), &relation.source_id)
            .await?;
        if existing
            .iter()
            .any(|r| r.target_id == relation.target_id && r.relation_type == relation.relation_type)
        {
            return Err(KnowledgeManagerError::DuplicateRelation(
                relation.source_id.clone(),
                relation.target_id.clone(),
            ));
        }

        let stored = self
            .repository
            .store_relation(ctx.clone(), relation)
            .await?;

        // Task 9.6
        let _ = self
            .governance
            .publish_event(GovernanceEvent::KnowledgeRelationCreated {
                relation_id: stored.id.clone(),
                source_id: stored.source_id.clone(),
                target_id: stored.target_id.clone(),
                relation_type: stored.relation_type,
                tenant_id: stored.tenant_id.clone(),
                timestamp: stored.created_at,
            })
            .await;

        Ok(stored)
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    async fn load_promotion(
        &self,
        ctx: TenantContext,
        promotion_id: &str,
    ) -> Result<PromotionRequest, KnowledgeManagerError> {
        self.repository
            .get_promotion_request(ctx, promotion_id)
            .await?
            .ok_or_else(|| KnowledgeManagerError::PromotionNotFound(promotion_id.to_string()))
    }

    /// Searches all layers to find an entry whose path ends with the given id
    /// or whose metadata contains it.  For simplicity we search by path prefix
    /// matching the id directly (entries created by this system use uuid paths).
    async fn find_entry_by_id(
        &self,
        ctx: TenantContext,
        id: &str,
    ) -> Result<Option<KnowledgeEntry>, KnowledgeManagerError> {
        for layer in [
            KnowledgeLayer::Project,
            KnowledgeLayer::Team,
            KnowledgeLayer::Org,
            KnowledgeLayer::Tenant,
        ] {
            if let Some(entry) = self.repository.get(ctx.clone(), layer, id).await? {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }

    #[allow(clippy::too_many_arguments)]
    async fn persist_promotion_relations(
        &self,
        ctx: TenantContext,
        source_id: &str,
        promoted_id: &str,
        residual_id: Option<&str>,
        decision: Option<PromotionDecision>,
        now: i64,
        tenant_id: &mk_core::types::TenantId,
        actor: &mk_core::types::UserId,
    ) -> Result<(), KnowledgeManagerError> {
        // PromotedTo: source → promoted
        self.create_relation(
            ctx.clone(),
            KnowledgeRelation {
                id: uuid::Uuid::new_v4().to_string(),
                source_id: source_id.to_string(),
                target_id: promoted_id.to_string(),
                relation_type: KnowledgeRelationType::PromotedTo,
                tenant_id: tenant_id.clone(),
                created_by: actor.clone(),
                created_at: now,
                metadata: HashMap::new(),
            },
        )
        .await?;

        // PromotedFrom: promoted → source
        self.create_relation(
            ctx.clone(),
            KnowledgeRelation {
                id: uuid::Uuid::new_v4().to_string(),
                source_id: promoted_id.to_string(),
                target_id: source_id.to_string(),
                relation_type: KnowledgeRelationType::PromotedFrom,
                tenant_id: tenant_id.clone(),
                created_by: actor.clone(),
                created_at: now,
                metadata: HashMap::new(),
            },
        )
        .await?;

        // Decision-based variant relation: promoted → source
        let variant_relation_type = match decision {
            Some(PromotionDecision::ApproveAsSpecialization) => {
                Some(KnowledgeRelationType::Specializes)
            }
            Some(PromotionDecision::ApproveAsApplicability) => {
                Some(KnowledgeRelationType::ApplicableFrom)
            }
            Some(PromotionDecision::ApproveAsException) => Some(KnowledgeRelationType::ExceptionTo),
            Some(PromotionDecision::ApproveAsClarification) => {
                Some(KnowledgeRelationType::Clarifies)
            }
            Some(PromotionDecision::ApproveAsReplacement) => {
                Some(KnowledgeRelationType::Supersedes)
            }
            _ => None,
        };

        if let Some(rel_type) = variant_relation_type {
            self.create_relation(
                ctx.clone(),
                KnowledgeRelation {
                    id: uuid::Uuid::new_v4().to_string(),
                    source_id: promoted_id.to_string(),
                    target_id: source_id.to_string(),
                    relation_type: rel_type,
                    tenant_id: tenant_id.clone(),
                    created_by: actor.clone(),
                    created_at: now,
                    metadata: HashMap::new(),
                },
            )
            .await?;
        }

        // Residual relation if present
        if let Some(rid) = residual_id {
            self.create_relation(
                ctx,
                KnowledgeRelation {
                    id: uuid::Uuid::new_v4().to_string(),
                    source_id: source_id.to_string(),
                    target_id: rid.to_string(),
                    relation_type: KnowledgeRelationType::DerivedFrom,
                    tenant_id: tenant_id.clone(),
                    created_by: actor.clone(),
                    created_at: now,
                    metadata: HashMap::new(),
                },
            )
            .await?;
        }

        Ok(())
    }
}
