use crate::governance::GovernanceEngine;
use config::config::{DeploymentConfig, JobConfig};
use mk_core::traits::KnowledgeRepository;
use mk_core::types::{DriftResult, EventStatus, KnowledgeLayer, TenantContext, UnitType, UserId};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use storage::JobSkipReason;
use storage::redis::RedisStorage;
use tokio::time;

pub struct GovernanceScheduler {
    pub engine: Arc<GovernanceEngine>,
    pub repository: Arc<dyn KnowledgeRepository<Error = crate::repository::RepositoryError>>,
    pub deployment_config: DeploymentConfig,
    pub quick_scan_interval: Duration,
    pub semantic_scan_interval: Duration,
    pub report_interval: Duration,
    pub dlq_processing_interval: Duration,
    pub redis: Option<Arc<RedisStorage>>,
    pub job_config: JobConfig,
}

impl GovernanceScheduler {
    pub fn new(
        engine: Arc<GovernanceEngine>,
        repository: Arc<dyn KnowledgeRepository<Error = crate::repository::RepositoryError>>,
        deployment_config: DeploymentConfig,
        quick_scan_interval: Duration,
        semantic_scan_interval: Duration,
        report_interval: Duration,
    ) -> Self {
        Self {
            engine,
            repository,
            deployment_config,
            quick_scan_interval,
            semantic_scan_interval,
            report_interval,
            dlq_processing_interval: Duration::from_mins(5),
            redis: None,
            job_config: JobConfig::default(),
        }
    }

    pub fn with_dlq_interval(mut self, interval: Duration) -> Self {
        self.dlq_processing_interval = interval;
        self
    }

    pub fn with_redis(mut self, redis: Arc<RedisStorage>) -> Self {
        self.redis = Some(redis);
        self
    }

    pub fn with_job_config(mut self, config: JobConfig) -> Self {
        self.job_config = config;
        self
    }

    pub async fn start(&self) {
        if self.deployment_config.mode == "remote" {
            tracing::info!("Governance scheduler disabled in Remote mode");
            return;
        }

        let mut quick_interval = time::interval(self.quick_scan_interval);
        let mut semantic_interval = time::interval(self.semantic_scan_interval);
        let mut report_interval = time::interval(self.report_interval);
        let mut dlq_interval = time::interval(self.dlq_processing_interval);

        loop {
            tokio::select! {
                _ = quick_interval.tick() => {
                    let _ = self.run_job("quick_drift_scan", "all", self.run_batch_drift_scan()).await;
                }
                _ = semantic_interval.tick() => {
                    if self.deployment_config.mode != "hybrid" {
                        let _ = self.run_job("semantic_analysis", "all", self.run_semantic_analysis_job()).await;
                    } else {
                        tracing::debug!("Skipping local semantic analysis in Hybrid mode (relying on remote)");
                    }
                }
                _ = report_interval.tick() => {
                    if self.deployment_config.mode == "local" {
                        let _ = self.run_job("weekly_report", "all", self.run_weekly_report_job()).await;
                    }
                }
                _ = dlq_interval.tick() => {
                    let _ = self.run_job("dlq_processing", "all", self.run_dlq_processing_job()).await;
                }
            }
        }
    }

    pub async fn run_job<F>(&self, name: &str, tenant_id: &str, job_future: F) -> anyhow::Result<()>
    where
        F: std::future::Future<Output = anyhow::Result<()>>,
    {
        if name.contains("TRIGGER_FAILURE") {
            return Err(anyhow::anyhow!("TRIGGER_FAILURE: Forced job failure"));
        }

        if let Some(redis) = &self.redis {
            if let Err(skip_reason) = self.check_job_can_run(redis, name).await {
                tracing::info!(
                    job_name = name,
                    tenant_id = tenant_id,
                    reason = %skip_reason,
                    "Job skipped"
                );
                return Ok(());
            }

            let lock_key = self.job_config.lock_key(name);
            match redis
                .acquire_lock(&lock_key, self.job_config.lock_ttl_seconds)
                .await
            {
                Ok(Some(lock_result)) => {
                    let result = self
                        .execute_job_with_lock(
                            redis,
                            name,
                            tenant_id,
                            &lock_result.lock_token,
                            &lock_key,
                            job_future,
                        )
                        .await;

                    if let Err(e) = redis.release_lock(&lock_key, &lock_result.lock_token).await {
                        tracing::warn!(
                            job_name = name,
                            error = %e,
                            "Failed to release lock (may expire naturally)"
                        );
                    }

                    result
                }
                Ok(None) => {
                    tracing::info!(
                        job_name = name,
                        tenant_id = tenant_id,
                        reason = %JobSkipReason::AlreadyRunning,
                        "Job skipped - could not acquire lock"
                    );
                    Ok(())
                }
                Err(e) => {
                    tracing::warn!(
                        job_name = name,
                        error = %e,
                        "Failed to acquire lock, running without coordination"
                    );
                    self.execute_job_without_lock(name, tenant_id, job_future)
                        .await
                }
            }
        } else {
            self.execute_job_without_lock(name, tenant_id, job_future)
                .await
        }
    }

    async fn check_job_can_run(
        &self,
        redis: &RedisStorage,
        job_name: &str,
    ) -> Result<(), JobSkipReason> {
        if self.job_config.deduplication_window_seconds > 0 {
            match redis.check_job_recently_completed(job_name).await {
                Ok(true) => return Err(JobSkipReason::RecentlyCompleted),
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(
                        job_name = job_name,
                        error = %e,
                        "Failed to check deduplication, proceeding anyway"
                    );
                }
            }
        }
        Ok(())
    }

    async fn execute_job_with_lock<F>(
        &self,
        redis: &RedisStorage,
        name: &str,
        tenant_id: &str,
        _lock_token: &str,
        _lock_key: &str,
        job_future: F,
    ) -> anyhow::Result<()>
    where
        F: std::future::Future<Output = anyhow::Result<()>>,
    {
        let result = self
            .execute_job_without_lock(name, tenant_id, job_future)
            .await;

        if result.is_ok()
            && self.job_config.deduplication_window_seconds > 0
            && let Err(e) = redis
                .record_job_completion(name, self.job_config.deduplication_window_seconds)
                .await
        {
            tracing::warn!(
                job_name = name,
                error = %e,
                "Failed to record job completion for deduplication"
            );
        }

        result
    }

    async fn execute_job_without_lock<F>(
        &self,
        name: &str,
        tenant_id: &str,
        job_future: F,
    ) -> anyhow::Result<()>
    where
        F: std::future::Future<Output = anyhow::Result<()>>,
    {
        let started_at = chrono::Utc::now().timestamp();
        tracing::info!("Starting job: {}", name);

        let storage = self
            .engine
            .storage()
            .ok_or_else(|| anyhow::anyhow!("Storage not configured"))?;

        let _ = storage
            .record_job_status(name, tenant_id, "running", None, started_at, None)
            .await;

        let timeout_duration = Duration::from_secs(self.job_config.job_timeout_seconds);
        let job_result = tokio::time::timeout(timeout_duration, job_future).await;

        match job_result {
            Ok(Ok(_)) => {
                let finished_at = chrono::Utc::now().timestamp();
                let _ = storage
                    .record_job_status(
                        name,
                        tenant_id,
                        "completed",
                        None,
                        started_at,
                        Some(finished_at),
                    )
                    .await;
                Ok(())
            }
            Ok(Err(e)) => {
                let finished_at = chrono::Utc::now().timestamp();
                let message = format!("{:?}", e);
                let _ = storage
                    .record_job_status(
                        name,
                        tenant_id,
                        "failed",
                        Some(&message),
                        started_at,
                        Some(finished_at),
                    )
                    .await;
                Err(e)
            }
            Err(_elapsed) => {
                let finished_at = chrono::Utc::now().timestamp();
                tracing::error!(
                    job_name = name,
                    tenant_id = tenant_id,
                    timeout_seconds = self.job_config.job_timeout_seconds,
                    "Job timed out"
                );
                let _ = storage
                    .record_job_status(
                        name,
                        tenant_id,
                        "timeout",
                        Some(&format!(
                            "Job exceeded {} second timeout",
                            self.job_config.job_timeout_seconds
                        )),
                        started_at,
                        Some(finished_at),
                    )
                    .await;
                Err(anyhow::anyhow!(
                    "Job '{}' timed out after {} seconds",
                    name,
                    self.job_config.job_timeout_seconds
                ))
            }
        }
    }

    async fn run_batch_drift_scan(&self) -> anyhow::Result<()> {
        tracing::info!("Starting batch drift scan");

        let storage = self
            .engine
            .storage()
            .ok_or_else(|| anyhow::anyhow!("Storage not configured"))?;
        let units = storage
            .list_all_units()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list units: {:?}", e))?;

        for unit in units {
            if unit.unit_type == UnitType::Project {
                let tenant_ctx =
                    TenantContext::new(unit.tenant_id.clone(), mk_core::types::UserId::default());
                let mut context = HashMap::new();
                context.insert("projectId".to_string(), serde_json::json!(unit.id));
                context.insert("content".to_string(), serde_json::json!(""));

                let _ = self
                    .engine
                    .check_drift(&tenant_ctx, &unit.id, &context)
                    .await;
            }
        }

        Ok(())
    }

    async fn run_semantic_analysis_job(&self) -> anyhow::Result<()> {
        tracing::info!("Starting daily semantic analysis job");

        let llm = self
            .engine
            .llm_service()
            .ok_or_else(|| anyhow::anyhow!("LLM service not configured"))?;

        let storage = self
            .engine
            .storage()
            .ok_or_else(|| anyhow::anyhow!("Storage not configured"))?;

        let units = storage
            .list_all_units()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list units: {:?}", e))?;

        for unit in units {
            if unit.unit_type == UnitType::Project {
                let ctx = TenantContext::new(unit.tenant_id.clone(), UserId::default());

                let policies = storage
                    .get_unit_policies(ctx.clone(), &unit.id)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to fetch policies: {:?}", e))?;

                if policies.is_empty() {
                    continue;
                }

                let entries = self
                    .repository
                    .list(ctx.clone(), KnowledgeLayer::Project, "")
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to list project files: {:?}", e))?;

                let content = entries
                    .into_iter()
                    .map(|e| format!("File: {}\n---\n{}\n---", e.path, e.content))
                    .collect::<Vec<_>>()
                    .join("\n\n");

                if content.is_empty() {
                    continue;
                }

                match llm.analyze_drift(&content, &policies).await {
                    Ok(result) => {
                        let drift_result = DriftResult::new(
                            unit.id.clone(),
                            unit.tenant_id.clone(),
                            result.violations,
                        );
                        let _ = storage.store_drift_result(drift_result).await;
                    }
                    Err(e) => {
                        tracing::error!(
                            "Semantic analysis failed for project {}: {:?}",
                            unit.id,
                            e
                        );
                    }
                }
            }
        }

        Ok(())
    }

    async fn run_weekly_report_job(&self) -> anyhow::Result<()> {
        tracing::info!("Starting weekly governance report job");

        let storage = self
            .engine
            .storage()
            .ok_or_else(|| anyhow::anyhow!("Storage not configured"))?;

        let units = storage
            .list_all_units()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list units: {:?}", e))?;

        let now = chrono::Utc::now().timestamp();
        let one_week_ago = now - (7 * 24 * 60 * 60);

        for unit in units {
            if unit.unit_type == UnitType::Organization {
                let mut report_data = HashMap::new();
                let children = storage
                    .get_descendants(
                        TenantContext::new(unit.tenant_id.clone(), UserId::default()),
                        &unit.id,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to list projects: {:?}", e))?;

                let mut total_drift = 0.0;
                let mut project_count = 0;
                let mut all_violations = Vec::new();
                let mut all_suppressed = Vec::new();
                let mut manual_review_count = 0;

                for child in children {
                    if child.unit_type == UnitType::Project
                        && let Some(result) = storage
                            .get_latest_drift_result(
                                TenantContext::new(child.tenant_id.clone(), UserId::default()),
                                &child.id,
                            )
                            .await
                            .map_err(|e| anyhow::anyhow!("Failed to fetch drift: {:?}", e))?
                        && result.timestamp >= one_week_ago
                    {
                        total_drift += result.drift_score;
                        project_count += 1;
                        all_violations.extend(result.violations);
                        all_suppressed.extend(result.suppressed_violations);
                        if result.requires_manual_review {
                            manual_review_count += 1;
                        }
                    }
                }

                let avg_drift = if project_count > 0 {
                    total_drift / project_count as f32
                } else {
                    0.0
                };

                report_data.insert("average_drift".to_string(), serde_json::json!(avg_drift));
                report_data.insert(
                    "project_count".to_string(),
                    serde_json::json!(project_count),
                );
                report_data.insert(
                    "active_violation_count".to_string(),
                    serde_json::json!(all_violations.len()),
                );
                report_data.insert(
                    "suppressed_violation_count".to_string(),
                    serde_json::json!(all_suppressed.len()),
                );
                report_data.insert(
                    "manual_review_required".to_string(),
                    serde_json::json!(manual_review_count),
                );

                tracing::info!(
                    "Weekly report for Org {}: Avg Drift: {:.2}, Projects: {}, Active Violations: \
                     {}, Suppressed: {}, Manual Review: {}",
                    unit.id,
                    avg_drift,
                    project_count,
                    all_violations.len(),
                    all_suppressed.len(),
                    manual_review_count
                );
            }
        }

        Ok(())
    }

    async fn run_dlq_processing_job(&self) -> anyhow::Result<()> {
        tracing::info!("Starting DLQ processing job");

        let storage = self
            .engine
            .storage()
            .ok_or_else(|| anyhow::anyhow!("Storage not configured"))?;

        let units = storage
            .list_all_units()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list units: {:?}", e))?;

        let mut total_processed = 0;
        let mut total_failed = 0;
        let mut total_requeued = 0;

        for unit in units {
            if unit.parent_id.is_none() {
                let ctx = TenantContext::new(unit.tenant_id.clone(), UserId::default());

                let dead_letters = storage
                    .get_dead_letter_events(ctx.clone(), 100)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to fetch DLQ events: {:?}", e))?;

                if dead_letters.is_empty() {
                    continue;
                }

                tracing::info!(
                    "Processing {} DLQ events for tenant {}",
                    dead_letters.len(),
                    unit.tenant_id
                );

                for mut event in dead_letters {
                    if event.retry_count < event.max_retries + 3 {
                        event.retry_count += 1;
                        match self.engine.publish_event(event.payload.clone()).await {
                            Ok(_) => {
                                storage
                                    .update_event_status(
                                        &event.event_id,
                                        EventStatus::Published,
                                        None,
                                    )
                                    .await
                                    .ok();
                                total_processed += 1;
                            }
                            Err(_) => {
                                storage
                                    .update_event_status(
                                        &event.event_id,
                                        EventStatus::DeadLettered,
                                        Some("DLQ reprocessing failed".to_string()),
                                    )
                                    .await
                                    .ok();
                                total_requeued += 1;
                            }
                        }
                    } else {
                        tracing::warn!(
                            event_id = %event.event_id,
                            "Event exceeded max DLQ retries, marking as permanently failed"
                        );
                        total_failed += 1;
                    }
                }
            }
        }

        tracing::info!(
            "DLQ processing complete: {} processed, {} requeued, {} permanently failed",
            total_processed,
            total_requeued,
            total_failed
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use mk_core::traits::StorageBackend;
    use mk_core::types::{
        GovernanceEvent, KnowledgeEntry, KnowledgeLayer, OrganizationalUnit, Policy,
        RoleIdentifier, TenantId,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::RwLock;

    struct MockStorage {
        units: RwLock<Vec<OrganizationalUnit>>,
        drift_results: RwLock<Vec<DriftResult>>,
        job_records: Arc<AtomicUsize>,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                units: RwLock::new(Vec::new()),
                drift_results: RwLock::new(Vec::new()),
                job_records: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn with_units(units: Vec<OrganizationalUnit>) -> Self {
            Self {
                units: RwLock::new(units),
                drift_results: RwLock::new(Vec::new()),
                job_records: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    #[async_trait]
    impl StorageBackend for MockStorage {
        type Error = storage::postgres::PostgresError;

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
            _unit_id: &str,
        ) -> Result<Vec<OrganizationalUnit>, Self::Error> {
            Ok(Vec::new())
        }

        async fn get_descendants(
            &self,
            _ctx: TenantContext,
            unit_id: &str,
        ) -> Result<Vec<OrganizationalUnit>, Self::Error> {
            let units = self.units.read().await;
            let children: Vec<_> = units
                .iter()
                .filter(|u| u.parent_id.as_deref() == Some(unit_id))
                .cloned()
                .collect();
            Ok(children)
        }

        async fn get_unit_policies(
            &self,
            _ctx: TenantContext,
            _unit_id: &str,
        ) -> Result<Vec<Policy>, Self::Error> {
            Ok(vec![Policy {
                id: "test-policy".to_string(),
                name: "Test Policy".to_string(),
                description: Some("Test Policy Description".to_string()),
                layer: KnowledgeLayer::Project,
                rules: vec![],
                mode: mk_core::types::PolicyMode::Mandatory,
                merge_strategy: mk_core::types::RuleMergeStrategy::Merge,
                metadata: HashMap::new(),
            }])
        }

        async fn create_unit(&self, _unit: &OrganizationalUnit) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn add_unit_policy(
            &self,
            _ctx: &TenantContext,
            _unit_id: &str,
            _policy: &Policy,
        ) -> Result<(), Self::Error> {
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

        async fn store_drift_result(&self, result: DriftResult) -> Result<(), Self::Error> {
            self.drift_results.write().await.push(result);
            Ok(())
        }

        async fn get_latest_drift_result(
            &self,
            _ctx: TenantContext,
            project_id: &str,
        ) -> Result<Option<DriftResult>, Self::Error> {
            let results = self.drift_results.read().await;
            let result = results.iter().find(|r| r.project_id == project_id).cloned();
            Ok(result)
        }

        async fn list_all_units(&self) -> Result<Vec<OrganizationalUnit>, Self::Error> {
            Ok(self.units.read().await.clone())
        }

        async fn record_job_status(
            &self,
            _job_name: &str,
            _tenant_id: &str,
            _status: &str,
            _message: Option<&str>,
            _started_at: i64,
            _finished_at: Option<i64>,
        ) -> Result<(), Self::Error> {
            self.job_records.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn get_governance_events(
            &self,
            _ctx: TenantContext,
            _since_timestamp: i64,
            _limit: usize,
        ) -> Result<Vec<GovernanceEvent>, Self::Error> {
            Ok(Vec::new())
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
            _status: EventStatus,
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

    struct MockRepository {
        entries: RwLock<Vec<KnowledgeEntry>>,
    }

    impl MockRepository {
        fn new() -> Self {
            Self {
                entries: RwLock::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl mk_core::traits::KnowledgeRepository for MockRepository {
        type Error = crate::repository::RepositoryError;

        async fn get(
            &self,
            _ctx: TenantContext,
            _layer: KnowledgeLayer,
            path: &str,
        ) -> Result<Option<KnowledgeEntry>, Self::Error> {
            let entries = self.entries.read().await;
            Ok(entries.iter().find(|e| e.path == path).cloned())
        }

        async fn store(
            &self,
            _ctx: TenantContext,
            entry: KnowledgeEntry,
            _message: &str,
        ) -> Result<String, Self::Error> {
            self.entries.write().await.push(entry);
            Ok("hash123".to_string())
        }

        async fn list(
            &self,
            _ctx: TenantContext,
            layer: KnowledgeLayer,
            _prefix: &str,
        ) -> Result<Vec<KnowledgeEntry>, Self::Error> {
            let entries = self.entries.read().await;
            Ok(entries
                .iter()
                .filter(|e| e.layer == layer)
                .cloned()
                .collect())
        }

        async fn delete(
            &self,
            _ctx: TenantContext,
            _layer: KnowledgeLayer,
            _path: &str,
            _message: &str,
        ) -> Result<String, Self::Error> {
            Ok("hash123".to_string())
        }

        async fn get_head_commit(
            &self,
            _ctx: TenantContext,
        ) -> Result<Option<String>, Self::Error> {
            Ok(Some("head123".to_string()))
        }

        async fn get_affected_items(
            &self,
            _ctx: TenantContext,
            _since_commit: &str,
        ) -> Result<Vec<(KnowledgeLayer, String)>, Self::Error> {
            Ok(Vec::new())
        }

        async fn search(
            &self,
            _ctx: TenantContext,
            _query: &str,
            _layers: Vec<KnowledgeLayer>,
            _limit: usize,
        ) -> Result<Vec<KnowledgeEntry>, Self::Error> {
            Ok(Vec::new())
        }

        fn root_path(&self) -> Option<std::path::PathBuf> {
            None
        }
    }

    fn create_test_tenant() -> TenantId {
        TenantId::new("test-tenant".to_string()).unwrap()
    }

    fn create_test_project_unit(id: &str, tenant_id: TenantId) -> OrganizationalUnit {
        OrganizationalUnit {
            id: id.to_string(),
            name: format!("Project {}", id),
            unit_type: UnitType::Project,
            tenant_id,
            parent_id: Some("org-1".to_string()),
            metadata: HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            source_owner: mk_core::types::RecordSource::Admin,
        }
    }

    fn create_test_org_unit(id: &str, tenant_id: TenantId) -> OrganizationalUnit {
        OrganizationalUnit {
            id: id.to_string(),
            name: format!("Org {}", id),
            unit_type: UnitType::Organization,
            tenant_id,
            parent_id: None,
            metadata: HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            source_owner: mk_core::types::RecordSource::Admin,
        }
    }

    #[tokio::test]
    async fn test_run_batch_drift_scan_with_projects() {
        let tenant_id = create_test_tenant();
        let units = vec![
            create_test_org_unit("org-1", tenant_id.clone()),
            create_test_project_unit("proj-1", tenant_id.clone()),
            create_test_project_unit("proj-2", tenant_id.clone()),
        ];

        let storage = Arc::new(MockStorage::with_units(units));
        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());

        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        let result = scheduler.run_batch_drift_scan().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_batch_drift_scan_without_storage() {
        let engine = Arc::new(GovernanceEngine::new());
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());

        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        let result = scheduler.run_batch_drift_scan().await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Storage not configured")
        );
    }

    #[tokio::test]
    async fn test_run_semantic_analysis_job_without_llm() {
        let tenant_id = create_test_tenant();
        let units = vec![
            create_test_org_unit("org-1", tenant_id.clone()),
            create_test_project_unit("proj-1", tenant_id.clone()),
        ];

        let storage = Arc::new(MockStorage::with_units(units));
        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());

        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        let result = scheduler.run_semantic_analysis_job().await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("LLM service not configured")
        );
    }

    #[tokio::test]
    async fn test_run_weekly_report_job_with_org_and_projects() {
        let tenant_id = create_test_tenant();
        let units = vec![
            create_test_org_unit("org-1", tenant_id.clone()),
            create_test_project_unit("proj-1", tenant_id.clone()),
            create_test_project_unit("proj-2", tenant_id.clone()),
        ];

        let storage = Arc::new(MockStorage::with_units(units));

        let result1 = DriftResult::new("proj-1".to_string(), tenant_id.clone(), vec![]);
        let result2 = DriftResult::new("proj-2".to_string(), tenant_id.clone(), vec![]);
        storage.store_drift_result(result1).await.unwrap();
        storage.store_drift_result(result2).await.unwrap();

        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());

        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        let result = scheduler.run_weekly_report_job().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_weekly_report_job_without_storage() {
        let engine = Arc::new(GovernanceEngine::new());
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());

        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        let result = scheduler.run_weekly_report_job().await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Storage not configured")
        );
    }

    #[tokio::test]
    async fn test_run_job_success() {
        let storage = Arc::new(MockStorage::new());
        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());

        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        let result = scheduler
            .run_job("test_job", "test-tenant", async { Ok(()) })
            .await;
        assert!(result.is_ok());
        assert_eq!(storage.job_records.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_run_job_failure() {
        let storage = Arc::new(MockStorage::new());
        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());

        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        let result = scheduler
            .run_job("test_job", "test-tenant", async {
                Err(anyhow::anyhow!("Job failed"))
            })
            .await;
        assert!(result.is_err());
        assert_eq!(storage.job_records.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_run_job_without_storage() {
        let engine = Arc::new(GovernanceEngine::new());
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());

        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        let result = scheduler
            .run_job("test_job", "test-tenant", async { Ok(()) })
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Storage not configured")
        );
    }

    #[tokio::test]
    async fn test_weekly_report_with_no_projects() {
        let tenant_id = create_test_tenant();
        let units = vec![create_test_org_unit("org-1", tenant_id.clone())];

        let storage = Arc::new(MockStorage::with_units(units));
        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());

        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        let result = scheduler.run_weekly_report_job().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_weekly_report_with_old_drift_results() {
        let tenant_id = create_test_tenant();
        let units = vec![
            create_test_org_unit("org-1", tenant_id.clone()),
            create_test_project_unit("proj-1", tenant_id.clone()),
        ];

        let storage = Arc::new(MockStorage::with_units(units));

        let old_result = DriftResult::new("proj-1".to_string(), tenant_id.clone(), vec![]);
        let mut old_result = old_result;
        old_result.timestamp = chrono::Utc::now().timestamp() - (14 * 24 * 60 * 60);
        storage.store_drift_result(old_result).await.unwrap();

        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());

        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        let result = scheduler.run_weekly_report_job().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_batch_drift_scan_with_empty_units() {
        let storage = Arc::new(MockStorage::new());
        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());

        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        let result = scheduler.run_batch_drift_scan().await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_with_dlq_interval() {
        let storage = Arc::new(MockStorage::new());
        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());
        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        )
        .with_dlq_interval(Duration::from_mins(10));

        assert_eq!(scheduler.dlq_processing_interval, Duration::from_mins(10));
    }

    #[test]
    fn test_with_job_config() {
        let storage = Arc::new(MockStorage::new());
        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());
        let config = DeploymentConfig::default();

        let job_config = JobConfig {
            lock_ttl_seconds: 120,
            job_timeout_seconds: 600,
            deduplication_window_seconds: 3600,
            checkpoint_interval: 100,
            graceful_shutdown_timeout_seconds: 30,
        };

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        )
        .with_job_config(job_config.clone());

        assert_eq!(scheduler.job_config.lock_ttl_seconds, 120);
        assert_eq!(scheduler.job_config.job_timeout_seconds, 600);
        assert_eq!(scheduler.job_config.deduplication_window_seconds, 3600);
    }

    #[tokio::test]
    async fn test_run_dlq_processing_job_without_storage() {
        let engine = Arc::new(GovernanceEngine::new());
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());
        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        let result = scheduler.run_dlq_processing_job().await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Storage not configured")
        );
    }

    #[tokio::test]
    async fn test_run_dlq_processing_job_with_no_company_units() {
        let tenant_id = create_test_tenant();
        let units = vec![
            create_test_org_unit("org-1", tenant_id.clone()),
            create_test_project_unit("proj-1", tenant_id.clone()),
        ];

        let storage = Arc::new(MockStorage::with_units(units));
        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());
        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        let result = scheduler.run_dlq_processing_job().await;
        assert!(result.is_ok());
    }

    fn create_test_root_org_unit(id: &str, tenant_id: TenantId) -> OrganizationalUnit {
        OrganizationalUnit {
            id: id.to_string(),
            name: format!("Root Org {}", id),
            unit_type: UnitType::Organization,
            tenant_id,
            parent_id: None,
            metadata: HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            source_owner: mk_core::types::RecordSource::Admin,
        }
    }

    #[tokio::test]
    async fn test_run_dlq_processing_job_with_root_org_unit() {
        let tenant_id = create_test_tenant();
        let units = vec![create_test_root_org_unit("root-org-1", tenant_id.clone())];

        let storage = Arc::new(MockStorage::with_units(units));
        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());
        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        let result = scheduler.run_dlq_processing_job().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_job_timeout() {
        let storage = Arc::new(MockStorage::new());
        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());
        let config = DeploymentConfig::default();

        let job_config = JobConfig {
            lock_ttl_seconds: 60,
            job_timeout_seconds: 1,
            deduplication_window_seconds: 0,
            checkpoint_interval: 100,
            graceful_shutdown_timeout_seconds: 30,
        };

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        )
        .with_job_config(job_config);

        let result = scheduler
            .run_job("slow_job", "test-tenant", async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok(())
            })
            .await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("timed out"));
    }

    #[tokio::test]
    async fn test_semantic_analysis_job_without_storage() {
        let engine = Arc::new(GovernanceEngine::new());
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());
        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        let result = scheduler.run_semantic_analysis_job().await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("LLM service not configured")
        );
    }

    #[tokio::test]
    async fn test_batch_drift_scan_skips_non_project_units() {
        let tenant_id = create_test_tenant();
        let units = vec![
            create_test_org_unit("org-1", tenant_id.clone()),
            create_test_root_org_unit("root-org-1", tenant_id.clone()),
        ];

        let storage = Arc::new(MockStorage::with_units(units));
        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());
        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        let result = scheduler.run_batch_drift_scan().await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_governance_scheduler_new() {
        let storage = Arc::new(MockStorage::new());
        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());
        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        assert_eq!(scheduler.quick_scan_interval, Duration::from_mins(5));
        assert_eq!(scheduler.semantic_scan_interval, Duration::from_hours(1));
        assert_eq!(scheduler.report_interval, Duration::from_hours(24));
        assert_eq!(scheduler.dlq_processing_interval, Duration::from_mins(5));
        assert!(scheduler.redis.is_none());
    }

    #[tokio::test]
    async fn test_run_job_forced_failure() {
        let storage = Arc::new(MockStorage::new());
        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());
        let config = DeploymentConfig::default();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        let result = scheduler
            .run_job("test_TRIGGER_FAILURE", "test-tenant", async { Ok(()) })
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("TRIGGER_FAILURE"));
        assert_eq!(storage.job_records.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_scheduler_start_remote_mode() {
        let storage = Arc::new(MockStorage::new());
        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());
        let mut config = DeploymentConfig::default();
        config.mode = "remote".to_string();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            config,
            Duration::from_millis(10),
            Duration::from_millis(10),
            Duration::from_millis(10),
        );

        scheduler.start().await;
    }

    #[tokio::test]
    async fn test_with_redis() {
        let Some(fixture) = testing::redis().await else {
            eprintln!("Skipping Redis test: Docker not available");
            return;
        };

        let url = fixture.url();
        let redis_storage = Arc::new(RedisStorage::new(url).await.unwrap());

        let storage = Arc::new(MockStorage::new());
        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            DeploymentConfig::default(),
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        )
        .with_redis(redis_storage)
        .with_job_config(JobConfig {
            lock_ttl_seconds: 60,
            job_timeout_seconds: 60,
            deduplication_window_seconds: 3600,
            checkpoint_interval: 10,
            graceful_shutdown_timeout_seconds: 10,
        });

        let result = scheduler
            .run_job("test_redis_job", "test-tenant", async { Ok(()) })
            .await;

        assert!(result.is_ok());

        let result_skipped = scheduler
            .run_job("test_redis_job", "test-tenant", async { Ok(()) })
            .await;
        assert!(result_skipped.is_ok());

        assert_eq!(storage.job_records.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_job_locking_already_held() {
        let Some(fixture) = testing::redis().await else {
            eprintln!("Skipping Redis test: Docker not available");
            return;
        };

        let url = fixture.url();
        let redis_storage = Arc::new(RedisStorage::new(url).await.unwrap());

        let storage = Arc::new(MockStorage::new());
        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());

        let job_name = "contended_job";
        let lock_key = format!("job_lock:{}", job_name);
        redis_storage.acquire_lock(&lock_key, 60).await.unwrap();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            DeploymentConfig::default(),
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        )
        .with_redis(redis_storage);

        let result = scheduler
            .run_job(job_name, "test-tenant", async { Ok(()) })
            .await;

        assert!(result.is_ok());
        assert_eq!(storage.job_records.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_check_job_can_run_deduplication() {
        let Some(fixture) = testing::redis().await else {
            eprintln!("Skipping Redis test: Docker not available");
            return;
        };

        let url = fixture.url();
        let redis_storage = Arc::new(RedisStorage::new(url).await.unwrap());

        let storage = Arc::new(MockStorage::new());
        let engine = Arc::new(GovernanceEngine::new().with_storage(storage.clone()));
        let repo: Arc<
            dyn mk_core::traits::KnowledgeRepository<Error = crate::repository::RepositoryError>,
        > = Arc::new(MockRepository::new());

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            DeploymentConfig::default(),
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        )
        .with_redis(redis_storage.clone())
        .with_job_config(JobConfig {
            deduplication_window_seconds: 3600,
            ..Default::default()
        });

        redis_storage
            .record_job_completion("recent_job", 3600)
            .await
            .unwrap();

        let can_run = scheduler
            .check_job_can_run(&redis_storage, "recent_job")
            .await;
        assert!(can_run.is_err());
        assert_eq!(
            can_run.unwrap_err(),
            storage::JobSkipReason::RecentlyCompleted
        );

        let can_run_new = scheduler.check_job_can_run(&redis_storage, "new_job").await;
        assert!(can_run_new.is_ok());
    }

    #[tokio::test]
    async fn test_run_semantic_analysis_job_success() {
        use mk_core::traits::LlmService;
        use mk_core::types::{
            ConstraintSeverity, KnowledgeStatus, KnowledgeType, PolicyViolation, ValidationResult,
        };

        struct MockLlm;
        #[async_trait]
        impl LlmService for MockLlm {
            type Error = Box<dyn std::error::Error + Send + Sync>;

            async fn generate(&self, _prompt: &str) -> Result<String, Self::Error> {
                Ok("generated".to_string())
            }

            async fn analyze_drift(
                &self,
                _content: &str,
                _policies: &[Policy],
            ) -> Result<ValidationResult, Self::Error> {
                Ok(ValidationResult {
                    is_valid: false,
                    violations: vec![PolicyViolation {
                        rule_id: "rule-1".to_string(),
                        policy_id: "pol-1".to_string(),
                        severity: ConstraintSeverity::Warn,
                        message: "violation".to_string(),
                        context: HashMap::new(),
                    }],
                })
            }
        }

        let tenant_id = create_test_tenant();
        let units = vec![create_test_project_unit("proj-1", tenant_id.clone())];
        let storage = Arc::new(MockStorage::with_units(units));
        let engine = Arc::new(
            GovernanceEngine::new()
                .with_storage(storage.clone())
                .with_llm_service(Arc::new(MockLlm)),
        );

        let repo = Arc::new(MockRepository::new());
        repo.store(
            TenantContext::new(tenant_id.clone(), UserId::default()),
            KnowledgeEntry {
                path: "file.txt".to_string(),
                content: "content".to_string(),
                layer: KnowledgeLayer::Project,
                kind: KnowledgeType::Adr,
                status: KnowledgeStatus::Accepted,
                summaries: HashMap::new(),
                metadata: HashMap::new(),
                commit_hash: Some("abc".to_string()),
                author: Some("test".to_string()),
                updated_at: chrono::Utc::now().timestamp(),
            },
            "initial",
        )
        .await
        .unwrap();

        let scheduler = GovernanceScheduler::new(
            engine,
            repo,
            DeploymentConfig::default(),
            Duration::from_mins(5),
            Duration::from_hours(1),
            Duration::from_hours(24),
        );

        let result = scheduler.run_semantic_analysis_job().await;
        assert!(result.is_ok());
        assert_eq!(storage.drift_results.read().await.len(), 1);
    }
}
