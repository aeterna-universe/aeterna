use crate::telemetry::KnowledgeTelemetry;
use mk_core::traits::{
    EmbeddingService, EventPublisher, LlmService, NotificationService, PromotionNotification,
    PromotionNotificationEventType,
};
use mk_core::types::{
    ConstraintSeverity, GovernanceEvent, KnowledgeLayer, Policy, PolicyViolation, TenantContext,
    ValidationResult,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use storage::events::EventError;
use thiserror::Error;

#[derive(Error, Debug, Serialize, Deserialize)]
pub enum GovernanceError {
    #[error("Event publisher error: {0}")]
    #[serde(skip)]
    Event(#[from] EventError),
    #[error("Embedding service error: {0}")]
    Embedding(String),
    #[error("LLM service error: {0}")]
    Llm(String),
    #[error("Storage error: {0}")]
    Storage(String),
    #[error("Validation failed: {0}")]
    Validation(String),
}

pub struct GovernanceEngine {
    policies: HashMap<KnowledgeLayer, Vec<Policy>>,
    telemetry: KnowledgeTelemetry,
    storage:
        Option<Arc<dyn mk_core::traits::StorageBackend<Error = storage::postgres::PostgresError>>>,
    event_publisher: Option<Arc<dyn EventPublisher<Error = EventError>>>,
    embedding_service: Option<Arc<dyn EmbeddingService<Error = anyhow::Error>>>,
    llm_service: Option<Arc<dyn LlmService<Error = Box<dyn std::error::Error + Send + Sync>>>>,
    knowledge_repository: Option<
        Arc<dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>>,
    >,
    /// Task 9.8: optional notification service for promotion lifecycle events
    notification_service: Option<Arc<dyn NotificationService>>,
}

impl GovernanceEngine {
    pub fn new() -> Self {
        Self {
            policies: HashMap::new(),
            telemetry: KnowledgeTelemetry,
            storage: None,
            event_publisher: None,
            embedding_service: None,
            llm_service: None,
            knowledge_repository: None,
            notification_service: None,
        }
    }

    pub fn with_storage(
        mut self,
        storage: Arc<dyn mk_core::traits::StorageBackend<Error = storage::postgres::PostgresError>>,
    ) -> Self {
        self.storage = Some(storage);
        self
    }

    pub fn with_event_publisher(
        mut self,
        publisher: Arc<dyn EventPublisher<Error = EventError>>,
    ) -> Self {
        self.event_publisher = Some(publisher);
        self
    }

    pub fn with_embedding_service(
        mut self,
        embedding_service: Arc<dyn EmbeddingService<Error = anyhow::Error>>,
    ) -> Self {
        self.embedding_service = Some(embedding_service);
        self
    }

    pub fn with_llm_service(
        mut self,
        llm_service: Arc<dyn LlmService<Error = Box<dyn std::error::Error + Send + Sync>>>,
    ) -> Self {
        self.llm_service = Some(llm_service);
        self
    }

    pub fn with_repository(
        mut self,
        repository: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        >,
    ) -> Self {
        self.knowledge_repository = Some(repository);
        self
    }

    /// Task 9.8 — Attach a notification service for promotion lifecycle events.
    pub fn with_notification_service(
        mut self,
        notification_service: Arc<dyn NotificationService>,
    ) -> Self {
        self.notification_service = Some(notification_service);
        self
    }

    pub fn storage(
        &self,
    ) -> Option<Arc<dyn mk_core::traits::StorageBackend<Error = storage::postgres::PostgresError>>>
    {
        self.storage.clone()
    }

    pub fn llm_service(
        &self,
    ) -> Option<Arc<dyn LlmService<Error = Box<dyn std::error::Error + Send + Sync>>>> {
        self.llm_service.clone()
    }

    pub fn repository(
        &self,
    ) -> Option<
        Arc<dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>>,
    > {
        self.knowledge_repository.clone()
    }
    pub async fn publish_event(&self, event: GovernanceEvent) -> Result<(), EventError> {
        // Task 9.8 — deliver promotion lifecycle notifications BEFORE publishing
        // (fire-and-forget; errors are logged but do not fail the request).
        if let Some(ns) = &self.notification_service
            && let Some(notification) = Self::event_to_notification(&event)
            && let Err(e) = ns.notify_promotion(notification.clone()).await
        {
            tracing::warn!(error = %e, "Promotion notification delivery failed");
            // Task 11.3 — alert-grade counter for notification delivery failures
            self.telemetry
                .record_notification_delivery_failed(&format!("{:?}", notification.event_type));
        }

        if let Some(publisher) = &self.event_publisher {
            publisher.publish(event).await
        } else {
            Ok(())
        }
    }

    /// Task 9.8 — Convert a `GovernanceEvent` to a `PromotionNotification` when applicable.
    fn event_to_notification(event: &GovernanceEvent) -> Option<PromotionNotification> {
        match event {
            GovernanceEvent::KnowledgePromotionRequested {
                promotion_id,
                source_item_id,
                requested_by,
                tenant_id,
                ..
            } => Some(PromotionNotification {
                event_type: PromotionNotificationEventType::Requested,
                promotion_id: promotion_id.clone(),
                source_item_id: source_item_id.clone(),
                proposer_id: requested_by.clone(),
                reason: None,
                tenant_id: tenant_id.clone(),
            }),
            GovernanceEvent::KnowledgePromotionApproved {
                promotion_id,
                approved_by,
                tenant_id,
                ..
            } => Some(PromotionNotification {
                event_type: PromotionNotificationEventType::Approved,
                promotion_id: promotion_id.clone(),
                source_item_id: String::new(),
                proposer_id: approved_by.clone(),
                reason: None,
                tenant_id: tenant_id.clone(),
            }),
            GovernanceEvent::KnowledgePromotionRejected {
                promotion_id,
                reason,
                rejected_by,
                tenant_id,
                ..
            } => Some(PromotionNotification {
                event_type: PromotionNotificationEventType::Rejected,
                promotion_id: promotion_id.clone(),
                source_item_id: String::new(),
                proposer_id: rejected_by.clone(),
                reason: Some(reason.clone()),
                tenant_id: tenant_id.clone(),
            }),
            GovernanceEvent::KnowledgePromotionRetargeted {
                promotion_id,
                retargeted_by,
                tenant_id,
                ..
            } => Some(PromotionNotification {
                event_type: PromotionNotificationEventType::Retargeted,
                promotion_id: promotion_id.clone(),
                source_item_id: String::new(),
                proposer_id: retargeted_by.clone(),
                reason: None,
                tenant_id: tenant_id.clone(),
            }),
            GovernanceEvent::KnowledgePromotionApplied {
                promotion_id,
                tenant_id,
                ..
            } => Some(PromotionNotification {
                event_type: PromotionNotificationEventType::Applied,
                promotion_id: promotion_id.clone(),
                source_item_id: String::new(),
                proposer_id: mk_core::types::UserId::default(),
                reason: None,
                tenant_id: tenant_id.clone(),
            }),
            _ => None,
        }
    }
}

impl Default for GovernanceEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl GovernanceEngine {
    pub fn add_policy(&mut self, policy: Policy) {
        self.policies.entry(policy.layer).or_default().push(policy);
    }

    pub fn event_publisher(&self) -> Option<Arc<dyn EventPublisher<Error = EventError>>> {
        self.event_publisher.clone()
    }

    pub fn validate(
        &self,
        target_layer: KnowledgeLayer,
        context: &HashMap<String, serde_json::Value>,
    ) -> ValidationResult {
        let mut resolved_map: HashMap<String, Policy> = HashMap::new();
        let mut mandatory_policies: HashMap<String, KnowledgeLayer> = HashMap::new();

        let layers = [
            KnowledgeLayer::Tenant,
            KnowledgeLayer::Org,
            KnowledgeLayer::Team,
            KnowledgeLayer::Project,
        ];

        for layer in &layers {
            if let Some(layer_policies) = self.policies.get(layer) {
                for policy in layer_policies {
                    let mut p = policy.clone();
                    p.layer = *layer;
                    self.merge_policy(&mut resolved_map, &mut mandatory_policies, p);
                }
            }
            if layer == &target_layer {
                break;
            }
        }

        let mut violations = Vec::new();
        let mut resolved_vec: Vec<Policy> = resolved_map.into_values().collect();
        resolved_vec.sort_by_key(|p| p.layer);

        for policy in resolved_vec {
            for rule in &policy.rules {
                if let Some(violation) = self.evaluate_rule(&policy, rule, context) {
                    self.telemetry.record_violation(
                        &format!("{:?}", policy.layer),
                        &format!("{:?}", rule.severity),
                    );
                    violations.push(violation);
                }
            }
        }

        ValidationResult {
            is_valid: violations.is_empty(),
            violations,
        }
    }

    pub async fn validate_with_context(
        &self,
        target_layer: KnowledgeLayer,
        context: &HashMap<String, serde_json::Value>,
        tenant_ctx: Option<&TenantContext>,
    ) -> Result<ValidationResult, GovernanceError> {
        let mut violations = Vec::new();

        let active_policies = self
            .resolve_active_policies(target_layer, context, tenant_ctx)
            .await;

        for policy in active_policies {
            for rule in &policy.rules {
                if let Some(violation) = self.evaluate_rule(&policy, rule, context) {
                    self.telemetry.record_violation(
                        &format!("{:?}", policy.layer),
                        &format!("{:?}", rule.severity),
                    );
                    violations.push(violation);
                }
            }
        }

        if !violations.is_empty() {
            self.emit_drift_event(context, tenant_ctx, &violations)
                .await;
        }

        Ok(ValidationResult {
            is_valid: violations.is_empty(),
            violations,
        })
    }

    async fn resolve_active_policies(
        &self,
        target_layer: KnowledgeLayer,
        context: &HashMap<String, serde_json::Value>,
        tenant_ctx: Option<&TenantContext>,
    ) -> Vec<Policy> {
        let mut resolved_map: HashMap<String, Policy> = HashMap::new();
        let mut mandatory_policies: HashMap<String, KnowledgeLayer> = HashMap::new();

        let layers = [
            KnowledgeLayer::Tenant,
            KnowledgeLayer::Org,
            KnowledgeLayer::Team,
            KnowledgeLayer::Project,
        ];

        for layer in &layers {
            if let Some(layer_policies) = self.policies.get(layer) {
                for policy in layer_policies {
                    self.merge_policy(&mut resolved_map, &mut mandatory_policies, policy.clone());
                }
            }
            if layer == &target_layer {
                break;
            }
        }

        if let Some(storage) = &self.storage {
            let unit_id = context
                .get("unitId")
                .or_else(|| context.get("projectId"))
                .and_then(|v| v.as_str());

            if let Some(uid) = unit_id {
                let ctx = tenant_ctx.cloned().unwrap_or_default();

                let mut units = Vec::new();
                if let Ok(mut ancestors) = storage.get_ancestors(ctx.clone(), uid).await {
                    units.append(&mut ancestors);
                }

                units.reverse();

                for unit in units {
                    if let Ok(unit_policies) =
                        storage.get_unit_policies(ctx.clone(), &unit.id).await
                    {
                        for policy in unit_policies {
                            self.merge_policy(&mut resolved_map, &mut mandatory_policies, policy);
                        }
                    }
                }

                if let Ok(unit_policies) = storage.get_unit_policies(ctx, uid).await {
                    for policy in unit_policies {
                        self.merge_policy(&mut resolved_map, &mut mandatory_policies, policy);
                    }
                }
            }
        }

        resolved_map.into_values().collect()
    }

    fn merge_policy(
        &self,
        resolved: &mut HashMap<String, Policy>,
        mandatory_policies: &mut HashMap<String, KnowledgeLayer>,
        incoming: Policy,
    ) {
        use mk_core::types::{PolicyMode, RuleMergeStrategy};

        let policy_id = incoming.id.clone();

        if let Some(mandatory_layer) = mandatory_policies.get(&policy_id)
            && incoming.layer != *mandatory_layer
            && incoming.merge_strategy != RuleMergeStrategy::Override
        {
            return;
        }

        if incoming.mode == PolicyMode::Mandatory {
            mandatory_policies.insert(policy_id.clone(), incoming.layer);
        }

        if let Some(existing) = resolved.get_mut(&policy_id) {
            match incoming.merge_strategy {
                RuleMergeStrategy::Override => {
                    *existing = incoming;
                }
                RuleMergeStrategy::Merge => {
                    for rule in incoming.rules {
                        if !existing.rules.iter().any(|r| r.id == rule.id) {
                            existing.rules.push(rule);
                        }
                    }
                    for (k, v) in incoming.metadata {
                        existing.metadata.insert(k, v);
                    }
                    existing.layer = incoming.layer;
                }
                RuleMergeStrategy::Intersect => {
                    existing
                        .rules
                        .retain(|r| incoming.rules.iter().any(|ir| ir.id == r.id));
                    existing.layer = incoming.layer;
                }
            }
        } else {
            resolved.insert(policy_id, incoming);
        }
    }

    pub async fn check_drift(
        &self,
        tenant_ctx: &TenantContext,
        _project_id: &str,
        context: &HashMap<String, serde_json::Value>,
    ) -> Result<f32, GovernanceError> {
        let mut violations = Vec::new();
        let mut confidence: f32 = 1.0;

        let content = context.get("content").and_then(|v| v.as_str());
        if let Some(c) = content
            && self.embedding_service.is_some()
        {
            let mut semantic_violations = self.check_contradictions(tenant_ctx, c, 0.8).await?;
            if !semantic_violations.is_empty() {
                confidence = confidence.min(0.85);
            }
            violations.append(&mut semantic_violations);
        }

        let active_policies = self
            .resolve_active_policies(KnowledgeLayer::Project, context, Some(tenant_ctx))
            .await;

        for policy in &active_policies {
            for rule in &policy.rules {
                if let Some(violation) = self.evaluate_rule(policy, rule, context) {
                    violations.push(violation);
                }
            }
        }

        let mandatory_policies_count = active_policies
            .iter()
            .filter(|p| p.mode == mk_core::types::PolicyMode::Mandatory)
            .count();

        if mandatory_policies_count == 0 {
            violations.push(PolicyViolation {
                rule_id: "missing_mandatory_policies".to_string(),
                policy_id: "governance_requirement".to_string(),
                severity: ConstraintSeverity::Warn,
                message: "No mandatory policies detected for this project layer".to_string(),
                context: context.clone(),
            });
        }

        for policy in &active_policies {
            if let Some(expected_hash) =
                policy.metadata.get("version_hash").and_then(|v| v.as_str())
            {
                let actual_hash = context.get("version_hash").and_then(|v| v.as_str());
                if let Some(actual) = actual_hash
                    && actual != expected_hash
                {
                    violations.push(PolicyViolation {
                        rule_id: "stale_policy_reference".to_string(),
                        policy_id: policy.id.clone(),
                        severity: ConstraintSeverity::Warn,
                        message: format!(
                            "Project uses stale policy version (expected: {}, actual: {})",
                            expected_hash, actual
                        ),
                        context: context.clone(),
                    });
                }
            }
        }

        if let Some(c) = content
            && let Some(llm_violations) = self
                .analyze_drift_with_llm(c, &active_policies, context)
                .await
        {
            confidence = confidence.min(0.75);
            for v in llm_violations {
                if !violations
                    .iter()
                    .any(|existing| existing.rule_id == v.rule_id)
                {
                    violations.push(v);
                }
            }
        }

        let (active_violations, suppressed_violations) = if let Some(storage) = &self.storage {
            let suppressions = storage
                .list_suppressions(tenant_ctx.clone(), _project_id)
                .await
                .map_err(|e| GovernanceError::Storage(e.to_string()))?;

            let active_suppressions: Vec<_> = suppressions
                .into_iter()
                .filter(|s| !s.is_expired())
                .collect();

            self.apply_suppressions(violations, &active_suppressions)
        } else {
            (violations, Vec::new())
        };

        let drift_config = if let Some(storage) = &self.storage {
            storage
                .get_drift_config(tenant_ctx.clone(), _project_id)
                .await
                .ok()
                .flatten()
        } else {
            None
        };

        let config = drift_config.unwrap_or_default();

        if config.auto_suppress_info {
            let filtered: Vec<_> = active_violations
                .iter()
                .filter(|v| v.severity != ConstraintSeverity::Info)
                .cloned()
                .collect();
            let auto_suppressed: Vec<_> = active_violations
                .iter()
                .filter(|v| v.severity == ConstraintSeverity::Info)
                .cloned()
                .collect();
            let mut all_suppressed = suppressed_violations;
            all_suppressed.extend(auto_suppressed);

            let drift_score = self.calculate_drift_score(&filtered);

            if drift_score > 0.0 {
                self.emit_drift_event(context, Some(tenant_ctx), &filtered)
                    .await;
            }

            if let Some(storage) = &self.storage {
                let drift_result = mk_core::types::DriftResult::new(
                    _project_id.to_string(),
                    tenant_ctx.tenant_id.clone(),
                    filtered,
                )
                .with_confidence(confidence)
                .with_suppressions(all_suppressed);
                let _ = storage.store_drift_result(drift_result).await;
            }

            return Ok(drift_score);
        }

        let drift_score = self.calculate_drift_score(&active_violations);

        if drift_score > 0.0 {
            self.emit_drift_event(context, Some(tenant_ctx), &active_violations)
                .await;
        }

        if let Some(storage) = &self.storage {
            let drift_result = mk_core::types::DriftResult::new(
                _project_id.to_string(),
                tenant_ctx.tenant_id.clone(),
                active_violations,
            )
            .with_confidence(confidence)
            .with_suppressions(suppressed_violations);
            let _ = storage.store_drift_result(drift_result).await;
        }

        Ok(drift_score)
    }

    fn apply_suppressions(
        &self,
        violations: Vec<PolicyViolation>,
        suppressions: &[mk_core::types::DriftSuppression],
    ) -> (Vec<PolicyViolation>, Vec<PolicyViolation>) {
        let mut active = Vec::new();
        let mut suppressed = Vec::new();

        for violation in violations {
            let is_suppressed = suppressions.iter().any(|s| s.matches(&violation));
            if is_suppressed {
                suppressed.push(violation);
            } else {
                active.push(violation);
            }
        }

        (active, suppressed)
    }

    async fn analyze_drift_with_llm(
        &self,
        content: &str,
        policies: &[Policy],
        context: &HashMap<String, serde_json::Value>,
    ) -> Option<Vec<PolicyViolation>> {
        let llm = self.llm_service.as_ref()?;

        if policies.is_empty() {
            return None;
        }

        match llm.analyze_drift(content, policies).await {
            Ok(result) => {
                if result.is_valid {
                    return None;
                }

                let violations = result
                    .violations
                    .into_iter()
                    .map(|v| PolicyViolation {
                        rule_id: format!("llm_{}", v.rule_id),
                        policy_id: v.policy_id,
                        severity: v.severity,
                        message: format!("[LLM Analysis] {}", v.message),
                        context: context.clone(),
                    })
                    .collect();

                Some(violations)
            }
            Err(e) => {
                tracing::warn!("LLM drift analysis failed: {}", e);
                None
            }
        }
    }

    fn calculate_drift_score(&self, violations: &[PolicyViolation]) -> f32 {
        if violations.is_empty() {
            return 0.0;
        }

        let score = violations
            .iter()
            .map(|v| match v.severity {
                ConstraintSeverity::Block => 1.0,
                ConstraintSeverity::Warn => 0.5,
                ConstraintSeverity::Info => 0.1,
            })
            .sum::<f32>();

        score.min(1.0)
    }

    async fn emit_drift_event(
        &self,
        context: &HashMap<String, serde_json::Value>,
        tenant_ctx: Option<&TenantContext>,
        violations: &[PolicyViolation],
    ) {
        if let Some(publisher) = &self.event_publisher {
            let project_id = context
                .get("projectId")
                .and_then(|v| v.as_str())
                .or_else(|| context.get("unitId").and_then(|v| v.as_str()));

            if let Some(pid) = project_id {
                let drift_score = violations
                    .iter()
                    .map(|v| match v.severity {
                        mk_core::types::ConstraintSeverity::Block => 1.0,
                        mk_core::types::ConstraintSeverity::Warn => 0.5,
                        mk_core::types::ConstraintSeverity::Info => 0.1,
                    })
                    .sum::<f32>();

                let _ = publisher
                    .publish(GovernanceEvent::DriftDetected {
                        project_id: pid.to_string(),
                        tenant_id: tenant_ctx.map(|c| c.tenant_id.clone()).unwrap_or_default(),
                        drift_score: drift_score.min(1.0),
                        timestamp: chrono::Utc::now().timestamp(),
                    })
                    .await;
            }
        }
    }

    fn evaluate_rule(
        &self,
        policy: &Policy,
        rule: &mk_core::types::PolicyRule,
        context: &HashMap<String, serde_json::Value>,
    ) -> Option<PolicyViolation> {
        use mk_core::types::{ConstraintOperator, RuleType};

        let target_key = match rule.target {
            mk_core::types::ConstraintTarget::File => "path",
            mk_core::types::ConstraintTarget::Code => "content",
            mk_core::types::ConstraintTarget::Dependency => "dependencies",
            mk_core::types::ConstraintTarget::Import => "imports",
            mk_core::types::ConstraintTarget::Config => "config",
        };

        let value = context.get(target_key);

        let is_condition_met = match rule.operator {
            ConstraintOperator::MustExist => value.is_some(),
            ConstraintOperator::MustNotExist => value.is_none(),
            ConstraintOperator::MustUse => {
                if let Some(v) = value {
                    if let Some(arr) = v.as_array() {
                        arr.contains(&rule.value)
                    } else {
                        v == &rule.value
                    }
                } else {
                    false
                }
            }
            ConstraintOperator::MustNotUse => {
                if let Some(v) = value {
                    if let Some(arr) = v.as_array() {
                        !arr.contains(&rule.value)
                    } else {
                        v != &rule.value
                    }
                } else {
                    true
                }
            }
            ConstraintOperator::MustMatch => {
                if let Some(v) = value {
                    if let Some(s) = v.as_str() {
                        if let Some(re_str) = rule.value.as_str() {
                            if let Ok(re) = regex::Regex::new(re_str) {
                                re.is_match(s)
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            ConstraintOperator::MustNotMatch => {
                if let Some(v) = value {
                    if let Some(s) = v.as_str() {
                        if let Some(re_str) = rule.value.as_str() {
                            if let Ok(re) = regex::Regex::new(re_str) {
                                !re.is_match(s)
                            } else {
                                true
                            }
                        } else {
                            true
                        }
                    } else {
                        true
                    }
                } else {
                    true
                }
            }
        };

        let is_violated = match rule.rule_type {
            RuleType::Allow => !is_condition_met,
            RuleType::Deny => is_condition_met,
        };

        if is_violated {
            Some(PolicyViolation {
                rule_id: rule.id.clone(),
                policy_id: policy.id.clone(),
                severity: rule.severity,
                message: rule.message.clone(),
                context: context.clone(),
            })
        } else {
            None
        }
    }

    pub async fn check_contradictions(
        &self,
        tenant_ctx: &TenantContext,
        content: &str,
        threshold: f32,
    ) -> Result<Vec<PolicyViolation>, GovernanceError> {
        let embedding_service = self.embedding_service.as_ref().ok_or_else(|| {
            GovernanceError::Embedding("Embedding service not configured".to_string())
        })?;

        let content_embedding = embedding_service
            .embed(content)
            .await
            .map_err(|e| GovernanceError::Embedding(e.to_string()))?;

        let mut violations = Vec::new();

        let mut context = HashMap::new();
        context.insert("content".to_string(), serde_json::json!(content));

        let active_policies = self
            .resolve_active_policies(KnowledgeLayer::Project, &context, Some(tenant_ctx))
            .await;

        for policy in active_policies {
            for rule in &policy.rules {
                if let Some(rule_embedding_val) =
                    policy.metadata.get(&format!("rule_{}_embedding", rule.id))
                    && let Ok(rule_embedding) =
                        serde_json::from_value::<Vec<f32>>(rule_embedding_val.clone())
                {
                    let similarity = self.cosine_similarity(&content_embedding, &rule_embedding);
                    if similarity > threshold {
                        violations.push(PolicyViolation {
                            rule_id: rule.id.clone(),
                            policy_id: policy.id.clone(),
                            severity: rule.severity,
                            message: format!(
                                "Semantic contradiction detected (similarity: {:.2}): {}",
                                similarity, rule.message
                            ),
                            context: context.clone(),
                        });
                    }
                }
            }
        }

        Ok(violations)
    }

    fn cosine_similarity(&self, v1: &[f32], v2: &[f32]) -> f32 {
        if v1.len() != v2.len() || v1.is_empty() {
            return 0.0;
        }
        let dot_product: f32 = v1.iter().zip(v2.iter()).map(|(a, b)| a * b).sum();
        let norm1: f32 = v1.iter().map(|a| a * a).sum::<f32>().sqrt();
        let norm2: f32 = v2.iter().map(|a| a * a).sum::<f32>().sqrt();

        if norm1 == 0.0 || norm2 == 0.0 {
            0.0
        } else {
            dot_product / (norm1 * norm2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mk_core::types::{ConstraintOperator, ConstraintSeverity, ConstraintTarget, PolicyRule};

    #[test]
    fn test_governance_engine_evaluation() {
        let mut engine = GovernanceEngine::new();

        let company_policy = Policy {
            id: "p1".to_string(),
            name: "Security Standards".to_string(),
            description: None,
            layer: KnowledgeLayer::Tenant,
            rules: vec![
                PolicyRule {
                    id: "r1".to_string(),
                    target: ConstraintTarget::Dependency,
                    operator: ConstraintOperator::MustNotUse,
                    value: serde_json::json!("unsafe-lib"),
                    severity: ConstraintSeverity::Block,
                    message: "unsafe-lib is banned".to_string(),
                    rule_type: mk_core::types::RuleType::Allow,
                },
                PolicyRule {
                    id: "r2".to_string(),
                    target: ConstraintTarget::Code,
                    operator: ConstraintOperator::MustMatch,
                    value: serde_json::json!("^# ADR"),
                    severity: ConstraintSeverity::Warn,
                    message: "ADRs must start with # ADR".to_string(),
                    rule_type: mk_core::types::RuleType::Allow,
                },
            ],
            metadata: HashMap::new(),
            mode: mk_core::types::PolicyMode::Optional,
            merge_strategy: mk_core::types::RuleMergeStrategy::Merge,
        };

        engine.add_policy(company_policy);

        let mut context = HashMap::new();
        context.insert(
            "dependencies".to_string(),
            serde_json::json!(["safe-lib", "unsafe-lib"]),
        );
        context.insert("content".to_string(), serde_json::json!("# ADR 001\n..."));

        let result = engine.validate(KnowledgeLayer::Project, &context);
        assert!(!result.is_valid);
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].rule_id, "r1");

        let mut context = HashMap::new();
        context.insert("dependencies".to_string(), serde_json::json!(["safe-lib"]));
        context.insert("content".to_string(), serde_json::json!("ADR 001\n..."));

        let result = engine.validate(KnowledgeLayer::Project, &context);
        assert!(!result.is_valid);
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].rule_id, "r2");

        let mut context = HashMap::new();
        context.insert("dependencies".to_string(), serde_json::json!(["safe-lib"]));
        context.insert("content".to_string(), serde_json::json!("# ADR 001\n..."));

        let result = engine.validate(KnowledgeLayer::Project, &context);
        assert!(result.is_valid);
    }

    fn create_test_policy(rule: PolicyRule) -> Policy {
        Policy {
            id: "test-policy".to_string(),
            name: "Test Policy".to_string(),
            description: None,
            layer: KnowledgeLayer::Project,
            rules: vec![rule],
            metadata: HashMap::new(),
            mode: mk_core::types::PolicyMode::Optional,
            merge_strategy: mk_core::types::RuleMergeStrategy::Merge,
        }
    }

    fn create_rule(
        operator: ConstraintOperator,
        target: ConstraintTarget,
        value: serde_json::Value,
    ) -> PolicyRule {
        PolicyRule {
            id: "test-rule".to_string(),
            target,
            operator,
            value,
            severity: ConstraintSeverity::Block,
            message: "Test rule violation".to_string(),
            rule_type: mk_core::types::RuleType::Allow,
        }
    }

    #[test]
    fn test_evaluate_rule_must_exist_passes_when_value_present() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustExist,
            ConstraintTarget::File,
            serde_json::json!(null),
        );
        let policy = create_test_policy(rule.clone());

        let mut context = HashMap::new();
        context.insert("path".to_string(), serde_json::json!("src/main.rs"));

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_none());
    }

    #[test]
    fn test_evaluate_rule_must_exist_fails_when_value_absent() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustExist,
            ConstraintTarget::File,
            serde_json::json!(null),
        );
        let policy = create_test_policy(rule.clone());

        let context = HashMap::new();

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_some());
    }

    #[test]
    fn test_evaluate_rule_must_not_exist_passes_when_value_absent() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustNotExist,
            ConstraintTarget::File,
            serde_json::json!(null),
        );
        let policy = create_test_policy(rule.clone());

        let context = HashMap::new();

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_none());
    }

    #[test]
    fn test_evaluate_rule_must_not_exist_fails_when_value_present() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustNotExist,
            ConstraintTarget::File,
            serde_json::json!(null),
        );
        let policy = create_test_policy(rule.clone());

        let mut context = HashMap::new();
        context.insert("path".to_string(), serde_json::json!("forbidden.txt"));

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_some());
    }

    #[test]
    fn test_evaluate_rule_must_use_with_array_passes_when_value_in_array() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustUse,
            ConstraintTarget::Dependency,
            serde_json::json!("required-lib"),
        );
        let policy = create_test_policy(rule.clone());

        let mut context = HashMap::new();
        context.insert(
            "dependencies".to_string(),
            serde_json::json!(["required-lib", "other-lib"]),
        );

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_none());
    }

    #[test]
    fn test_evaluate_rule_must_use_with_array_fails_when_value_not_in_array() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustUse,
            ConstraintTarget::Dependency,
            serde_json::json!("required-lib"),
        );
        let policy = create_test_policy(rule.clone());

        let mut context = HashMap::new();
        context.insert("dependencies".to_string(), serde_json::json!(["other-lib"]));

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_some());
    }

    #[test]
    fn test_evaluate_rule_must_use_with_scalar_passes_when_values_match() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustUse,
            ConstraintTarget::Config,
            serde_json::json!("production"),
        );
        let policy = create_test_policy(rule.clone());

        let mut context = HashMap::new();
        context.insert("config".to_string(), serde_json::json!("production"));

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_none());
    }

    #[test]
    fn test_evaluate_rule_must_use_fails_when_value_absent() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustUse,
            ConstraintTarget::Dependency,
            serde_json::json!("required-lib"),
        );
        let policy = create_test_policy(rule.clone());

        let context = HashMap::new();

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_some());
    }

    #[test]
    fn test_evaluate_rule_must_not_use_with_array_passes_when_value_not_in_array() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustNotUse,
            ConstraintTarget::Dependency,
            serde_json::json!("banned-lib"),
        );
        let policy = create_test_policy(rule.clone());

        let mut context = HashMap::new();
        context.insert(
            "dependencies".to_string(),
            serde_json::json!(["safe-lib", "another-lib"]),
        );

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_none());
    }

    #[test]
    fn test_evaluate_rule_must_not_use_with_array_fails_when_value_in_array() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustNotUse,
            ConstraintTarget::Dependency,
            serde_json::json!("banned-lib"),
        );
        let policy = create_test_policy(rule.clone());

        let mut context = HashMap::new();
        context.insert(
            "dependencies".to_string(),
            serde_json::json!(["safe-lib", "banned-lib"]),
        );

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_some());
    }

    #[test]
    fn test_evaluate_rule_must_not_use_passes_when_value_absent() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustNotUse,
            ConstraintTarget::Dependency,
            serde_json::json!("banned-lib"),
        );
        let policy = create_test_policy(rule.clone());

        let context = HashMap::new();

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_none());
    }

    #[test]
    fn test_evaluate_rule_must_match_passes_when_regex_matches() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustMatch,
            ConstraintTarget::Code,
            serde_json::json!("^# ADR"),
        );
        let policy = create_test_policy(rule.clone());

        let mut context = HashMap::new();
        context.insert(
            "content".to_string(),
            serde_json::json!("# ADR 001\nDecision..."),
        );

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_none());
    }

    #[test]
    fn test_evaluate_rule_must_match_fails_when_regex_does_not_match() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustMatch,
            ConstraintTarget::Code,
            serde_json::json!("^# ADR"),
        );
        let policy = create_test_policy(rule.clone());

        let mut context = HashMap::new();
        context.insert(
            "content".to_string(),
            serde_json::json!("Some other content"),
        );

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_some());
    }

    #[test]
    fn test_evaluate_rule_must_match_fails_when_value_absent() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustMatch,
            ConstraintTarget::Code,
            serde_json::json!("^# ADR"),
        );
        let policy = create_test_policy(rule.clone());

        let context = HashMap::new();

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_some());
    }

    #[test]
    fn test_evaluate_rule_must_match_fails_when_value_not_string() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustMatch,
            ConstraintTarget::Code,
            serde_json::json!("^# ADR"),
        );
        let policy = create_test_policy(rule.clone());

        let mut context = HashMap::new();
        context.insert("content".to_string(), serde_json::json!(12345));

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_some());
    }

    #[test]
    fn test_evaluate_rule_must_match_fails_when_regex_invalid() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustMatch,
            ConstraintTarget::Code,
            serde_json::json!("[invalid(regex"),
        );
        let policy = create_test_policy(rule.clone());

        let mut context = HashMap::new();
        context.insert("content".to_string(), serde_json::json!("any content"));

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_some());
    }

    #[test]
    fn test_evaluate_rule_must_not_match_passes_when_regex_does_not_match() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustNotMatch,
            ConstraintTarget::Code,
            serde_json::json!("TODO|FIXME"),
        );
        let policy = create_test_policy(rule.clone());

        let mut context = HashMap::new();
        context.insert("content".to_string(), serde_json::json!("Clean code here"));

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_none());
    }

    #[test]
    fn test_evaluate_rule_must_not_match_fails_when_regex_matches() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustNotMatch,
            ConstraintTarget::Code,
            serde_json::json!("TODO|FIXME"),
        );
        let policy = create_test_policy(rule.clone());

        let mut context = HashMap::new();
        context.insert(
            "content".to_string(),
            serde_json::json!("// TODO: fix this later"),
        );

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_some());
    }

    #[test]
    fn test_evaluate_rule_must_not_match_passes_when_value_absent() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustNotMatch,
            ConstraintTarget::Code,
            serde_json::json!("TODO"),
        );
        let policy = create_test_policy(rule.clone());

        let context = HashMap::new();

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_none());
    }

    #[test]
    fn test_evaluate_rule_must_not_match_passes_when_value_not_string() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustNotMatch,
            ConstraintTarget::Code,
            serde_json::json!("pattern"),
        );
        let policy = create_test_policy(rule.clone());

        let mut context = HashMap::new();
        context.insert(
            "content".to_string(),
            serde_json::json!(["not", "a", "string"]),
        );

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_none());
    }

    #[test]
    fn test_evaluate_rule_must_not_match_passes_when_regex_invalid() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustNotMatch,
            ConstraintTarget::Code,
            serde_json::json!("[invalid(regex"),
        );
        let policy = create_test_policy(rule.clone());

        let mut context = HashMap::new();
        context.insert("content".to_string(), serde_json::json!("any content"));

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_none());
    }

    #[test]
    fn test_evaluate_rule_must_not_match_passes_when_pattern_not_string() {
        let engine = GovernanceEngine::new();
        let rule = create_rule(
            ConstraintOperator::MustNotMatch,
            ConstraintTarget::Code,
            serde_json::json!(12345),
        );
        let policy = create_test_policy(rule.clone());

        let mut context = HashMap::new();
        context.insert("content".to_string(), serde_json::json!("any content"));

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_none());
    }

    #[test]
    fn test_evaluate_rule_deny_rule_type_inverts_logic() {
        let engine = GovernanceEngine::new();
        let mut rule = create_rule(
            ConstraintOperator::MustExist,
            ConstraintTarget::File,
            serde_json::json!(null),
        );
        rule.rule_type = mk_core::types::RuleType::Deny;
        let policy = create_test_policy(rule.clone());

        let mut context = HashMap::new();
        context.insert("path".to_string(), serde_json::json!("src/main.rs"));

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_some());
    }

    #[test]
    fn test_evaluate_rule_all_constraint_targets() {
        let engine = GovernanceEngine::new();

        let targets_and_keys = [
            (ConstraintTarget::File, "path"),
            (ConstraintTarget::Code, "content"),
            (ConstraintTarget::Dependency, "dependencies"),
            (ConstraintTarget::Import, "imports"),
            (ConstraintTarget::Config, "config"),
        ];

        for (target, key) in targets_and_keys {
            let rule = create_rule(
                ConstraintOperator::MustExist,
                target,
                serde_json::json!(null),
            );
            let policy = create_test_policy(rule.clone());

            let mut context = HashMap::new();
            context.insert(key.to_string(), serde_json::json!("value"));

            let result = engine.evaluate_rule(&policy, &rule, &context);
            assert!(
                result.is_none(),
                "Target {:?} with key {} should pass",
                target,
                key
            );
        }
    }

    #[test]
    fn test_evaluate_rule_violation_contains_correct_metadata() {
        let engine = GovernanceEngine::new();
        let mut rule = create_rule(
            ConstraintOperator::MustExist,
            ConstraintTarget::File,
            serde_json::json!(null),
        );
        rule.id = "specific-rule-id".to_string();
        rule.message = "Custom error message".to_string();
        rule.severity = ConstraintSeverity::Warn;

        let mut policy = create_test_policy(rule.clone());
        policy.id = "specific-policy-id".to_string();

        let mut context = HashMap::new();
        context.insert("other_key".to_string(), serde_json::json!("value"));

        let result = engine.evaluate_rule(&policy, &rule, &context);
        assert!(result.is_some());

        let violation = result.unwrap();
        assert_eq!(violation.rule_id, "specific-rule-id");
        assert_eq!(violation.policy_id, "specific-policy-id");
        assert_eq!(violation.message, "Custom error message");
        assert_eq!(violation.severity, ConstraintSeverity::Warn);
        assert!(violation.context.contains_key("other_key"));
    }

    #[test]
    fn test_cosine_similarity_identical_vectors() {
        let engine = GovernanceEngine::new();
        let v1 = vec![1.0, 2.0, 3.0];
        let v2 = vec![1.0, 2.0, 3.0];
        let similarity = engine.cosine_similarity(&v1, &v2);
        assert!((similarity - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_cosine_similarity_orthogonal_vectors() {
        let engine = GovernanceEngine::new();
        let v1 = vec![1.0, 0.0];
        let v2 = vec![0.0, 1.0];
        let similarity = engine.cosine_similarity(&v1, &v2);
        assert!(similarity.abs() < 0.0001);
    }

    #[test]
    fn test_cosine_similarity_different_lengths() {
        let engine = GovernanceEngine::new();
        let v1 = vec![1.0, 2.0, 3.0];
        let v2 = vec![1.0, 2.0];
        let similarity = engine.cosine_similarity(&v1, &v2);
        assert_eq!(similarity, 0.0);
    }

    #[test]
    fn test_cosine_similarity_empty_vectors() {
        let engine = GovernanceEngine::new();
        let v1: Vec<f32> = vec![];
        let v2: Vec<f32> = vec![];
        let similarity = engine.cosine_similarity(&v1, &v2);
        assert_eq!(similarity, 0.0);
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let engine = GovernanceEngine::new();
        let v1 = vec![0.0, 0.0, 0.0];
        let v2 = vec![1.0, 2.0, 3.0];
        let similarity = engine.cosine_similarity(&v1, &v2);
        assert_eq!(similarity, 0.0);
    }

    #[test]
    fn test_governance_engine_default() {
        let engine = GovernanceEngine::default();
        assert!(engine.storage().is_none());
        assert!(engine.llm_service().is_none());
        assert!(engine.repository().is_none());
        assert!(engine.event_publisher().is_none());
    }

    #[test]
    fn test_calculate_drift_score_empty_violations() {
        let engine = GovernanceEngine::new();
        let violations: Vec<PolicyViolation> = vec![];
        let score = engine.calculate_drift_score(&violations);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_calculate_drift_score_single_block() {
        let engine = GovernanceEngine::new();
        let violations = vec![PolicyViolation {
            rule_id: "test".to_string(),
            policy_id: "test".to_string(),
            severity: ConstraintSeverity::Block,
            message: "Test".to_string(),
            context: HashMap::new(),
        }];
        let score = engine.calculate_drift_score(&violations);
        assert_eq!(score, 1.0);
    }

    #[test]
    fn test_calculate_drift_score_single_warn() {
        let engine = GovernanceEngine::new();
        let violations = vec![PolicyViolation {
            rule_id: "test".to_string(),
            policy_id: "test".to_string(),
            severity: ConstraintSeverity::Warn,
            message: "Test".to_string(),
            context: HashMap::new(),
        }];
        let score = engine.calculate_drift_score(&violations);
        assert!((score - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_calculate_drift_score_single_info() {
        let engine = GovernanceEngine::new();
        let violations = vec![PolicyViolation {
            rule_id: "test".to_string(),
            policy_id: "test".to_string(),
            severity: ConstraintSeverity::Info,
            message: "Test".to_string(),
            context: HashMap::new(),
        }];
        let score = engine.calculate_drift_score(&violations);
        assert!((score - 0.1).abs() < 0.001);
    }

    #[test]
    fn test_calculate_drift_score_capped_at_one() {
        let engine = GovernanceEngine::new();
        let violations = vec![
            PolicyViolation {
                rule_id: "test1".to_string(),
                policy_id: "test".to_string(),
                severity: ConstraintSeverity::Block,
                message: "Test".to_string(),
                context: HashMap::new(),
            },
            PolicyViolation {
                rule_id: "test2".to_string(),
                policy_id: "test".to_string(),
                severity: ConstraintSeverity::Block,
                message: "Test".to_string(),
                context: HashMap::new(),
            },
        ];
        let score = engine.calculate_drift_score(&violations);
        assert_eq!(score, 1.0);
    }

    #[test]
    fn test_calculate_drift_score_mixed_severities() {
        let engine = GovernanceEngine::new();
        let violations = vec![
            PolicyViolation {
                rule_id: "warn".to_string(),
                policy_id: "test".to_string(),
                severity: ConstraintSeverity::Warn,
                message: "Test".to_string(),
                context: HashMap::new(),
            },
            PolicyViolation {
                rule_id: "info".to_string(),
                policy_id: "test".to_string(),
                severity: ConstraintSeverity::Info,
                message: "Test".to_string(),
                context: HashMap::new(),
            },
        ];
        let score = engine.calculate_drift_score(&violations);
        assert!((score - 0.6).abs() < 0.001);
    }

    #[test]
    fn test_apply_suppressions_no_suppressions() {
        let engine = GovernanceEngine::new();
        let violations = vec![PolicyViolation {
            rule_id: "rule1".to_string(),
            policy_id: "policy1".to_string(),
            severity: ConstraintSeverity::Block,
            message: "Test".to_string(),
            context: HashMap::new(),
        }];
        let suppressions: Vec<mk_core::types::DriftSuppression> = vec![];

        let (active, suppressed) = engine.apply_suppressions(violations, &suppressions);
        assert_eq!(active.len(), 1);
        assert_eq!(suppressed.len(), 0);
    }

    #[test]
    fn test_apply_suppressions_with_matching_suppression() {
        use mk_core::types::{TenantId, UserId};
        let engine = GovernanceEngine::new();
        let violations = vec![
            PolicyViolation {
                rule_id: "rule1".to_string(),
                policy_id: "policy1".to_string(),
                severity: ConstraintSeverity::Block,
                message: "Test message for policy1".to_string(),
                context: HashMap::new(),
            },
            PolicyViolation {
                rule_id: "rule2".to_string(),
                policy_id: "policy2".to_string(),
                severity: ConstraintSeverity::Warn,
                message: "Test message for policy2".to_string(),
                context: HashMap::new(),
            },
        ];
        let suppressions = vec![
            mk_core::types::DriftSuppression::new(
                "project1".to_string(),
                TenantId::new("tenant1".to_string()).unwrap(),
                "policy1".to_string(),
                "Test suppression".to_string(),
                UserId::new("tester".to_string()).unwrap(),
            )
            .with_expiry(chrono::Utc::now().timestamp() + 3600),
        ];

        let (active, suppressed) = engine.apply_suppressions(violations, &suppressions);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].rule_id, "rule2");
        assert_eq!(suppressed.len(), 1);
        assert_eq!(suppressed[0].rule_id, "rule1");
    }

    #[test]
    fn test_apply_suppressions_all_suppressed() {
        use mk_core::types::{TenantId, UserId};
        let engine = GovernanceEngine::new();
        let violations = vec![PolicyViolation {
            rule_id: "rule1".to_string(),
            policy_id: "policy1".to_string(),
            severity: ConstraintSeverity::Block,
            message: "Test message".to_string(),
            context: HashMap::new(),
        }];
        let suppressions = vec![mk_core::types::DriftSuppression::new(
            "project1".to_string(),
            TenantId::new("tenant1".to_string()).unwrap(),
            "policy1".to_string(),
            "Test suppression".to_string(),
            UserId::new("tester".to_string()).unwrap(),
        )];

        let (active, suppressed) = engine.apply_suppressions(violations, &suppressions);
        assert_eq!(active.len(), 0);
        assert_eq!(suppressed.len(), 1);
    }

    #[test]
    fn test_apply_suppressions_with_rule_pattern() {
        use mk_core::types::{TenantId, UserId};
        let engine = GovernanceEngine::new();
        let violations = vec![PolicyViolation {
            rule_id: "rule1".to_string(),
            policy_id: "policy1".to_string(),
            severity: ConstraintSeverity::Block,
            message: "Violation for rule1 detected".to_string(),
            context: HashMap::new(),
        }];
        let suppressions = vec![
            mk_core::types::DriftSuppression::new(
                "project1".to_string(),
                TenantId::new("tenant1".to_string()).unwrap(),
                "policy1".to_string(),
                "Test suppression".to_string(),
                UserId::new("tester".to_string()).unwrap(),
            )
            .with_pattern("rule1".to_string()),
        ];

        let (active, suppressed) = engine.apply_suppressions(violations, &suppressions);
        assert_eq!(active.len(), 0);
        assert_eq!(suppressed.len(), 1);
    }

    #[test]
    fn test_apply_suppressions_pattern_not_matching() {
        use mk_core::types::{TenantId, UserId};
        let engine = GovernanceEngine::new();
        let violations = vec![PolicyViolation {
            rule_id: "rule1".to_string(),
            policy_id: "policy1".to_string(),
            severity: ConstraintSeverity::Block,
            message: "Some other message".to_string(),
            context: HashMap::new(),
        }];
        let suppressions = vec![
            mk_core::types::DriftSuppression::new(
                "project1".to_string(),
                TenantId::new("tenant1".to_string()).unwrap(),
                "policy1".to_string(),
                "Test suppression".to_string(),
                UserId::new("tester".to_string()).unwrap(),
            )
            .with_pattern("specific_pattern".to_string()),
        ];

        let (active, suppressed) = engine.apply_suppressions(violations, &suppressions);
        assert_eq!(active.len(), 1);
        assert_eq!(suppressed.len(), 0);
    }

    #[test]
    fn test_merge_policy_override_strategy() {
        let mut engine = GovernanceEngine::new();

        let company_policy = Policy {
            id: "merge-test".to_string(),
            name: "Tenant Policy".to_string(),
            description: None,
            layer: KnowledgeLayer::Tenant,
            mode: mk_core::types::PolicyMode::Optional,
            merge_strategy: mk_core::types::RuleMergeStrategy::Merge,
            rules: vec![PolicyRule {
                id: "r1".to_string(),
                target: ConstraintTarget::Code,
                operator: ConstraintOperator::MustExist,
                value: serde_json::json!(null),
                severity: ConstraintSeverity::Block,
                message: "Tenant rule".to_string(),
                rule_type: mk_core::types::RuleType::Allow,
            }],
            metadata: HashMap::new(),
        };

        let org_policy = Policy {
            id: "merge-test".to_string(),
            name: "Org Policy".to_string(),
            description: None,
            layer: KnowledgeLayer::Org,
            mode: mk_core::types::PolicyMode::Optional,
            merge_strategy: mk_core::types::RuleMergeStrategy::Override,
            rules: vec![PolicyRule {
                id: "r2".to_string(),
                target: ConstraintTarget::Code,
                operator: ConstraintOperator::MustExist,
                value: serde_json::json!(null),
                severity: ConstraintSeverity::Warn,
                message: "Org rule".to_string(),
                rule_type: mk_core::types::RuleType::Allow,
            }],
            metadata: HashMap::new(),
        };

        engine.add_policy(company_policy);
        engine.add_policy(org_policy);

        let context = HashMap::new();
        let result = engine.validate(KnowledgeLayer::Org, &context);

        assert!(!result.is_valid);
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].message, "Org rule");
    }

    #[test]
    fn test_merge_policy_intersect_strategy() {
        let mut engine = GovernanceEngine::new();

        let company_policy = Policy {
            id: "intersect-test".to_string(),
            name: "Tenant Policy".to_string(),
            description: None,
            layer: KnowledgeLayer::Tenant,
            mode: mk_core::types::PolicyMode::Optional,
            merge_strategy: mk_core::types::RuleMergeStrategy::Merge,
            rules: vec![
                PolicyRule {
                    id: "r1".to_string(),
                    target: ConstraintTarget::Code,
                    operator: ConstraintOperator::MustExist,
                    value: serde_json::json!(null),
                    severity: ConstraintSeverity::Block,
                    message: "Rule 1".to_string(),
                    rule_type: mk_core::types::RuleType::Allow,
                },
                PolicyRule {
                    id: "r2".to_string(),
                    target: ConstraintTarget::File,
                    operator: ConstraintOperator::MustExist,
                    value: serde_json::json!(null),
                    severity: ConstraintSeverity::Block,
                    message: "Rule 2".to_string(),
                    rule_type: mk_core::types::RuleType::Allow,
                },
            ],
            metadata: HashMap::new(),
        };

        let org_policy = Policy {
            id: "intersect-test".to_string(),
            name: "Org Policy".to_string(),
            description: None,
            layer: KnowledgeLayer::Org,
            mode: mk_core::types::PolicyMode::Optional,
            merge_strategy: mk_core::types::RuleMergeStrategy::Intersect,
            rules: vec![PolicyRule {
                id: "r1".to_string(),
                target: ConstraintTarget::Code,
                operator: ConstraintOperator::MustExist,
                value: serde_json::json!(null),
                severity: ConstraintSeverity::Warn,
                message: "Rule 1 only".to_string(),
                rule_type: mk_core::types::RuleType::Allow,
            }],
            metadata: HashMap::new(),
        };

        engine.add_policy(company_policy);
        engine.add_policy(org_policy);

        let context = HashMap::new();
        let result = engine.validate(KnowledgeLayer::Org, &context);

        assert!(!result.is_valid);
        assert_eq!(result.violations.len(), 1);
    }

    #[test]
    fn test_merge_policy_metadata_merged() {
        let mut engine = GovernanceEngine::new();

        let mut metadata1 = HashMap::new();
        metadata1.insert("key1".to_string(), serde_json::json!("value1"));

        let company_policy = Policy {
            id: "metadata-test".to_string(),
            name: "Tenant Policy".to_string(),
            description: None,
            layer: KnowledgeLayer::Tenant,
            mode: mk_core::types::PolicyMode::Optional,
            merge_strategy: mk_core::types::RuleMergeStrategy::Merge,
            rules: vec![],
            metadata: metadata1,
        };

        let mut metadata2 = HashMap::new();
        metadata2.insert("key2".to_string(), serde_json::json!("value2"));

        let org_policy = Policy {
            id: "metadata-test".to_string(),
            name: "Org Policy".to_string(),
            description: None,
            layer: KnowledgeLayer::Org,
            mode: mk_core::types::PolicyMode::Optional,
            merge_strategy: mk_core::types::RuleMergeStrategy::Merge,
            rules: vec![],
            metadata: metadata2,
        };

        engine.add_policy(company_policy);
        engine.add_policy(org_policy);

        let context = HashMap::new();
        let result = engine.validate(KnowledgeLayer::Org, &context);

        assert!(result.is_valid);
    }

    #[test]
    fn test_mandatory_policy_cannot_be_overridden_at_lower_layer() {
        let mut engine = GovernanceEngine::new();

        let company_policy = Policy {
            id: "mandatory-test".to_string(),
            name: "Mandatory Tenant Policy".to_string(),
            description: None,
            layer: KnowledgeLayer::Tenant,
            mode: mk_core::types::PolicyMode::Mandatory,
            merge_strategy: mk_core::types::RuleMergeStrategy::Merge,
            rules: vec![PolicyRule {
                id: "mandatory-rule".to_string(),
                target: ConstraintTarget::Code,
                operator: ConstraintOperator::MustNotMatch,
                value: serde_json::json!("FORBIDDEN"),
                severity: ConstraintSeverity::Block,
                message: "Forbidden content".to_string(),
                rule_type: mk_core::types::RuleType::Allow,
            }],
            metadata: HashMap::new(),
        };

        let org_policy = Policy {
            id: "mandatory-test".to_string(),
            name: "Org Override Attempt".to_string(),
            description: None,
            layer: KnowledgeLayer::Org,
            mode: mk_core::types::PolicyMode::Optional,
            merge_strategy: mk_core::types::RuleMergeStrategy::Merge,
            rules: vec![],
            metadata: HashMap::new(),
        };

        engine.add_policy(company_policy);
        engine.add_policy(org_policy);

        let mut context = HashMap::new();
        context.insert("content".to_string(), serde_json::json!("FORBIDDEN text"));

        let result = engine.validate(KnowledgeLayer::Org, &context);
        assert!(
            !result.is_valid,
            "Mandatory policy should still apply despite org override attempt"
        );
    }

    #[tokio::test]
    async fn test_publish_event_without_publisher() {
        let engine = GovernanceEngine::new();
        let event = mk_core::types::GovernanceEvent::DriftDetected {
            project_id: "test".to_string(),
            tenant_id: mk_core::types::TenantId::new("test".to_string()).unwrap(),
            drift_score: 0.5,
            timestamp: chrono::Utc::now().timestamp(),
        };

        let result = engine.publish_event(event).await;
        assert!(result.is_ok());
    }
}
