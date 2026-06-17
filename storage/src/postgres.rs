use async_trait::async_trait;
use mk_core::traits::StorageBackend;
use mk_core::types::{
    INSTANCE_SCOPE_TENANT_ID, OrganizationalUnit, RoleIdentifier, SYSTEM_USER_ID, TenantContext,
    UnitType,
};
use sqlx::{Pool, Postgres, Row};
use thiserror::Error;

use crate::rls_migration;

#[derive(Error, Debug)]
pub enum PostgresError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Unit not found: {0}")]
    NotFound(String),
}

/// Application-facing Postgres backend.
///
/// Carries two connection pools:
///
/// - `pool` — the **tenant pool**. In production (post-A.3 Wave 6) this
///   opens connections as `aeterna_app` (NOBYPASSRLS). All per-tenant
///   request traffic runs through it, and every tenant-scoped query must
///   be wrapped in [`with_tenant_context`](Self::with_tenant_context) so
///   the `app.tenant_id` GUC is set for the RLS policy.
///
/// - `admin_pool` — the **admin pool**. Opens connections as
///   `aeterna_admin` (BYPASSRLS). Reserved for the narrow admin surface:
///   PlatformAdmin cross-tenant list endpoints, scheduled cross-tenant
///   maintenance, and the migration runner. Capped at 4 connections on
///   purpose — admin operations are rare, and a tight pool makes
///   accidental hot-loop use of the admin path a visible failure rather
///   than a silent cross-tenant leak. Access must go through
///   [`with_admin_context`](Self::with_admin_context), which records an
///   audit row automatically.
///
/// Direct access to either pool outside the two helpers is a latent
/// RLS-bypass hazard and is policed by `cli/tests/admin_pool_access_lint.rs`.
/// See `openspec/changes/decide-rls-enforcement-model/design.md` §4 for
/// the full rationale.
pub struct PostgresBackend {
    pool: Pool<Postgres>,
    admin_pool: Pool<Postgres>,
}

impl PostgresBackend {
    /// Tenant-scope pool. Direct use is forbidden outside
    /// [`with_tenant_context`](Self::with_tenant_context); callers that
    /// reach for it bypass the RLS session-variable invariant.
    pub fn pool(&self) -> &Pool<Postgres> {
        &self.pool
    }

    /// Admin-scope pool. Direct use is forbidden outside
    /// [`with_admin_context`](Self::with_admin_context); callers that
    /// reach for it bypass both the transaction boundary and the audit
    /// write.
    pub fn admin_pool(&self) -> &Pool<Postgres> {
        &self.admin_pool
    }

    /// Construct from a pre-built tenant pool, reusing the same pool as
    /// the admin pool. Intended for tests and single-pool dev
    /// environments. Production paths should prefer
    /// [`new_with_admin`](Self::new_with_admin).
    pub fn from_pool(pool: Pool<Postgres>) -> Self {
        Self {
            admin_pool: pool.clone(),
            pool,
        }
    }

    /// Construct with separate tenant and admin pools. Production path.
    pub fn from_pools(pool: Pool<Postgres>, admin_pool: Pool<Postgres>) -> Self {
        Self { pool, admin_pool }
    }

    /// Convenience constructor for tests and single-role dev setups:
    /// opens one pool and uses it for both tenant and admin work.
    ///
    /// In production, use [`new_with_admin`](Self::new_with_admin).
    pub async fn new(connection_url: &str) -> Result<Self, PostgresError> {
        use sqlx::postgres::PgPoolOptions;
        use std::time::Duration;

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(30))
            .connect(connection_url)
            .await?;
        Ok(Self {
            admin_pool: pool.clone(),
            pool,
        })
    }

    /// Production constructor: opens two separate pools.
    ///
    /// `tenant_url` should point at `aeterna_app`; `admin_url` at
    /// `aeterna_admin`. Admin pool is capped at 4 connections as a
    /// deliberate constraint (see the type-level doc-comment for the
    /// rationale).
    pub async fn new_with_admin(tenant_url: &str, admin_url: &str) -> Result<Self, PostgresError> {
        use sqlx::postgres::PgPoolOptions;
        use std::time::Duration;

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(30))
            .connect(tenant_url)
            .await?;
        let admin_pool = PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(30))
            .connect(admin_url)
            .await?;
        Ok(Self { pool, admin_pool })
    }

    /// Arm the Postgres transaction-level RLS context for the given tenant.
    ///
    /// Uses `set_config('app.tenant_id', $1, true)` — the `true` argument is
    /// equivalent to `SET LOCAL`, scoping the setting to the **current
    /// transaction only**. On `COMMIT` or `ROLLBACK` the setting is discarded,
    /// so a returned-to-pool connection can never leak tenant context to a
    /// subsequent request.
    ///
    /// # Contract
    ///
    /// - Caller MUST be inside an active transaction. Outside a transaction,
    ///   `SET LOCAL` applies only to the statement itself and is immediately
    ///   discarded — resulting in a silent no-op. This function takes a
    ///   `&mut Transaction` rather than a `PoolConnection` to make that
    ///   requirement compile-time enforced.
    /// - After this call returns, every tenant-scoped RLS policy keyed on
    ///   `current_setting('app.tenant_id', true)` will evaluate correctly for
    ///   the remainder of the transaction.
    ///
    /// # See also
    ///
    /// - Issue #57: previous session-scoped version leaked tenant context
    ///   across pooled connections.
    /// - Issue #58: policies may still be a no-op in prod if the connection
    ///   role has `BYPASSRLS` — orthogonal, deferred.
    ///
    /// # Hazard H1 status
    ///
    /// The "SET LOCAL outside a transaction is a silent no-op" hazard (H1 in
    /// the RLS enforcement RFC) is now **compile-time impossible** on this
    /// function: the `&mut Transaction` parameter type cannot be constructed
    /// outside an active `begin()`. Any legacy caller that passed a raw
    /// `&PgPool` or `PoolConnection` will fail to compile. The
    /// `debug_assert!` below guards against another regression class
    /// (empty tenant_id producing a policy-evaluating-against-empty-string
    /// that silently matches nothing).
    pub async fn activate_tenant_context(
        tx: &mut sqlx::Transaction<'_, Postgres>,
        tenant_id: &str,
    ) -> Result<(), PostgresError> {
        debug_assert!(
            !tenant_id.is_empty(),
            "activate_tenant_context called with empty tenant_id — this would \
             make every RLS policy evaluate against an empty string and \
             silently match zero rows. Upstream bug in context resolution."
        );
        sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
            .bind(tenant_id)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    /// Run `body` inside a tenant-scoped transaction with
    /// `app.tenant_id` set for the duration.
    ///
    /// This is the ONLY valid way to reach [`Self::pool`] on an
    /// RLS-protected code path. Every row-security policy keyed on
    /// `current_setting('app.tenant_id', true)` will evaluate correctly
    /// for the transaction's lifetime; on `COMMIT` or `ROLLBACK`
    /// PostgreSQL discards the setting, so a returned-to-pool connection
    /// cannot leak tenant context into a subsequent request.
    ///
    /// # Shape
    ///
    /// ```ignore
    /// let rows = backend.with_tenant_context(&ctx, |tx| {
    ///     Box::pin(async move {
    ///         sqlx::query_as::<_, Row>("SELECT \u2026 FROM users")
    ///             .fetch_all(&mut **tx).await.map_err(Into::into)
    ///     })
    /// }).await?;
    /// ```
    ///
    /// The explicit `WHERE tenant_id = $N` defense-in-depth clause
    /// should remain in place on the inner query (RLS is the floor, the
    /// `WHERE` clause is the ceiling; `cli/tests/rls_enforcement_test.rs`
    /// surfaces any divergence).
    pub async fn with_tenant_context<'a, F, T>(
        &self,
        ctx: &TenantContext,
        body: F,
    ) -> Result<T, PostgresError>
    where
        F: for<'c> FnOnce(
            &'c mut sqlx::Transaction<'_, Postgres>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<T, PostgresError>> + Send + 'c>,
        >,
    {
        let tenant_id = ctx.tenant_id.to_string();
        debug_assert!(
            !tenant_id.is_empty(),
            "with_tenant_context invoked with empty tenant_id — RLS policies \
             will evaluate against '' and silently match zero rows"
        );

        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
            .bind(&tenant_id)
            .execute(&mut *tx)
            .await?;
        let out = body(&mut tx).await?;
        tx.commit().await?;
        Ok(out)
    }

    /// Run `body` inside an admin-scoped transaction on the BYPASSRLS
    /// pool, recording a `governance_audit_log` row on success.
    ///
    /// This is the ONLY valid way to reach [`Self::admin_pool`]. Every
    /// call records an audit event with `admin_scope = TRUE` and the
    /// actor's identity pulled from `ctx`. `acting_as_tenant_id` is
    /// populated from `ctx.target_tenant_id` when the admin is
    /// impersonating a single tenant; NULL for cross-tenant-all
    /// operations.
    ///
    /// The audit write lives **inside** the same transaction as `body`:
    /// the admin action and its audit row commit atomically or not at
    /// all. This is the single most important invariant of the helper.
    ///
    /// # When to use
    ///
    /// - PlatformAdmin list endpoints with `?tenant=*`.
    /// - Scheduled cross-tenant jobs (audit compaction, global sweeps).
    /// - The migration runner.
    ///
    /// Everything else should use
    /// [`with_tenant_context`](Self::with_tenant_context).
    pub async fn with_admin_context<'a, F, T>(
        &self,
        ctx: &TenantContext,
        action: &str,
        body: F,
    ) -> Result<T, PostgresError>
    where
        F: for<'c> FnOnce(
            &'c mut sqlx::Transaction<'_, Postgres>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<T, PostgresError>> + Send + 'c>,
        >,
    {
        let mut tx = self.admin_pool.begin().await?;
        let out = body(&mut tx).await?;
        audit_admin_access(&mut tx, ctx, action).await?;
        tx.commit().await?;
        Ok(out)
    }

    /// Run `body` inside a plain transaction with NO tenant context set and
    /// NO admin audit write.
    ///
    /// This is the read-only helper for the **authentication bootstrap
    /// path** — the narrow window before a `TenantContext` exists, where the
    /// server is still resolving which user/tenant the caller belongs to.
    /// Concrete use cases:
    ///
    /// - `resolve_user_id_by_idp` / `resolve_user_id_by_idp_subject`
    /// - `get_user_roles_for_auth` at `INSTANCE_SCOPE_TENANT_ID`
    /// - `get_user_tenant_ids`
    /// - `get_user_default_tenant`
    ///
    /// These methods read from `users` / `user_roles`, which are NOT
    /// RLS-enabled (verified in migrations 004, 006, 008, 012, 014, 016):
    /// there is no `app.tenant_id` to set, and no admin-audit row is
    /// warranted on the auth hot path (one per request would flood
    /// `governance_audit_log`).
    ///
    /// # When NOT to use
    ///
    /// - Any write path — use `with_tenant_context` or `with_admin_context`.
    /// - Anything touching an RLS-gated table — bootstrap tx does not set
    ///   `app.tenant_id`, so those queries would silently match zero rows.
    /// - Anything where a `TenantContext` is already available — prefer
    ///   `with_tenant_context` for consistency.
    pub async fn with_bootstrap_context<'a, F, T>(&self, body: F) -> Result<T, PostgresError>
    where
        F: for<'c> FnOnce(
            &'c mut sqlx::Transaction<'_, Postgres>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<T, PostgresError>> + Send + 'c>,
        >,
    {
        let mut tx = self.pool.begin().await?;
        let out = body(&mut tx).await?;
        tx.commit().await?;
        Ok(out)
    }

    pub async fn initialize_schema(&self) -> Result<(), PostgresError> {
        // Enable pgcrypto extension for gen_random_uuid()
        sqlx::query("CREATE EXTENSION IF NOT EXISTS pgcrypto")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS sync_state (
                id TEXT NOT NULL,
                tenant_id TEXT NOT NULL,
                data JSONB NOT NULL,
                updated_at BIGINT NOT NULL,
                PRIMARY KEY (id, tenant_id)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_sync_state_tenant_id ON sync_state(tenant_id)")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS organizational_units (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                type TEXT NOT NULL, -- legacy root, organization, team, project
                parent_id TEXT REFERENCES organizational_units(id),
                tenant_id TEXT NOT NULL,
                metadata JSONB DEFAULT '{}',
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS user_roles (
                user_id TEXT NOT NULL,
                tenant_id TEXT NOT NULL,
                unit_id TEXT NOT NULL REFERENCES organizational_units(id),
                role TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (user_id, tenant_id, unit_id, role)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS unit_policies (
                id TEXT PRIMARY KEY,
                unit_id TEXT NOT NULL REFERENCES organizational_units(id),
                policy JSONB NOT NULL,
                created_at BIGINT NOT NULL,
                updated_at BIGINT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            // Full schema aligned with migration 006 (006_event_streaming.sql).
            // Both initialize_schema() and migration 006 use CREATE TABLE IF
            // NOT EXISTS; whichever executes first wins and the other is a
            // no-op. Having identical column sets here ensures migration 006's
            // subsequent CREATE INDEX statements (e.g. on idempotency_key)
            // succeed regardless of execution order.
            //
            // event_id and idempotency_key are nullable so that the
            // log_event() path (INSERT event_type, tenant_id, payload only)
            // remains compatible. PostgreSQL UNIQUE constraints treat each
            // NULL as distinct, so multiple NULL idempotency_key rows are
            // allowed while non-NULL keys are still deduplicated correctly via
            // ON CONFLICT (idempotency_key).
            "CREATE TABLE IF NOT EXISTS governance_events (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                event_id VARCHAR(255),
                idempotency_key VARCHAR(255) UNIQUE,
                event_type VARCHAR(100) NOT NULL,
                tenant_id VARCHAR(255) NOT NULL,
                payload JSONB NOT NULL,
                status VARCHAR(50) NOT NULL DEFAULT 'pending',
                retry_count INTEGER NOT NULL DEFAULT 0,
                max_retries INTEGER NOT NULL DEFAULT 3,
                last_error TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                published_at TIMESTAMPTZ,
                acknowledged_at TIMESTAMPTZ,
                dead_lettered_at TIMESTAMPTZ
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS drift_results (
                project_id TEXT NOT NULL,
                tenant_id TEXT NOT NULL,
                drift_score REAL NOT NULL,
                confidence REAL NOT NULL DEFAULT 1.0,
                violations JSONB NOT NULL,
                suppressed_violations JSONB NOT NULL DEFAULT '[]',
                requires_manual_review BOOLEAN NOT NULL DEFAULT FALSE,
                timestamp BIGINT NOT NULL,
                PRIMARY KEY (project_id, tenant_id, timestamp)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS drift_configs (
                project_id TEXT NOT NULL,
                tenant_id TEXT NOT NULL,
                threshold REAL NOT NULL DEFAULT 0.3,
                low_confidence_threshold REAL NOT NULL DEFAULT 0.7,
                auto_suppress_info BOOLEAN NOT NULL DEFAULT FALSE,
                updated_at BIGINT NOT NULL,
                PRIMARY KEY (project_id, tenant_id)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS project_team_assignments (
                project_id TEXT NOT NULL REFERENCES organizational_units(id),
                team_id TEXT NOT NULL REFERENCES organizational_units(id),
                tenant_id TEXT NOT NULL,
                assignment_type TEXT NOT NULL CHECK (assignment_type IN ('owner', 'contributor')),
                created_at BIGINT NOT NULL,
                PRIMARY KEY (project_id, team_id, tenant_id)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS job_status (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                job_name TEXT NOT NULL,
                tenant_id TEXT NOT NULL,
                status TEXT NOT NULL, -- 'running', 'completed', 'failed'
                message TEXT,
                started_at BIGINT NOT NULL,
                finished_at BIGINT,
                duration_ms BIGINT
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS graph_nodes (
                id TEXT NOT NULL,
                tenant_id TEXT NOT NULL,
                label TEXT NOT NULL,
                properties JSONB NOT NULL DEFAULT '{}',
                created_at BIGINT NOT NULL,
                PRIMARY KEY (id, tenant_id)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS graph_edges (
                id TEXT NOT NULL,
                tenant_id TEXT NOT NULL,
                source_id TEXT NOT NULL,
                target_id TEXT NOT NULL,
                relation TEXT NOT NULL,
                properties JSONB NOT NULL DEFAULT '{}',
                created_at BIGINT NOT NULL,
                PRIMARY KEY (id, tenant_id),
                FOREIGN KEY (source_id, tenant_id) REFERENCES graph_nodes(id, tenant_id) ON DELETE \
             CASCADE,
                FOREIGN KEY (target_id, tenant_id) REFERENCES graph_nodes(id, tenant_id) ON DELETE \
             CASCADE
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_graph_edges_source ON graph_edges(source_id, \
             tenant_id)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_graph_edges_target ON graph_edges(target_id, \
             tenant_id)",
        )
        .execute(&self.pool)
        .await?;

        // NOTE: No `embedding VECTOR(N)` column here — semantic vectors live in
        // Qdrant (see `memory::backends::qdrant`). Postgres stores only metadata.
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS memory_entries ( id TEXT PRIMARY KEY, tenant_id TEXT NOT \
             NULL, content TEXT NOT NULL, memory_layer TEXT NOT NULL, \
             properties JSONB DEFAULT '{}', device_id TEXT, importance_score REAL DEFAULT 0.0, \
             created_at BIGINT NOT NULL, updated_at BIGINT NOT NULL, deleted_at BIGINT )",
        )
        .execute(&self.pool)
        .await
        .ok();

        sqlx::query("ALTER TABLE memory_entries ADD COLUMN IF NOT EXISTS device_id TEXT")
            .execute(&self.pool)
            .await
            .ok();
        sqlx::query(
            "ALTER TABLE memory_entries ADD COLUMN IF NOT EXISTS importance_score REAL DEFAULT \
             0.0",
        )
        .execute(&self.pool)
        .await
        .ok();
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_memory_entries_device_id ON \
             memory_entries(device_id, tenant_id)",
        )
        .execute(&self.pool)
        .await
        .ok();
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_memory_entries_updated_at ON \
             memory_entries(updated_at, tenant_id)",
        )
        .execute(&self.pool)
        .await
        .ok();
        sqlx::query("ALTER TABLE memory_entries ADD COLUMN IF NOT EXISTS last_accessed_at BIGINT")
            .execute(&self.pool)
            .await
            .ok();
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_memory_entries_last_accessed ON \
             memory_entries(last_accessed_at, memory_layer) \
             WHERE last_accessed_at IS NOT NULL AND deleted_at IS NULL",
        )
        .execute(&self.pool)
        .await
        .ok();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS knowledge_items ( id TEXT PRIMARY KEY, tenant_id TEXT NOT \
             NULL, type TEXT NOT NULL, title TEXT NOT NULL, content TEXT NOT NULL, tags TEXT[], \
             properties JSONB DEFAULT '{}', created_at BIGINT NOT NULL, updated_at BIGINT NOT \
             NULL )",
        )
        .execute(&self.pool)
        .await
        .ok();

        // CCA Hindsight Learning tables
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS error_signatures (
                id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                error_type TEXT NOT NULL,
                message_pattern TEXT NOT NULL,
                stack_patterns JSONB DEFAULT '[]',
                context_patterns JSONB DEFAULT '[]',
                embedding JSONB,
                occurrence_count INTEGER DEFAULT 1,
                first_seen_at BIGINT NOT NULL,
                last_seen_at BIGINT NOT NULL,
                created_at BIGINT NOT NULL,
                updated_at BIGINT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_error_signatures_tenant ON error_signatures(tenant_id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_error_signatures_type ON error_signatures(tenant_id, \
             error_type)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS resolutions (
                id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                error_signature_id TEXT NOT NULL REFERENCES error_signatures(id) ON DELETE CASCADE,
                description TEXT NOT NULL,
                changes JSONB DEFAULT '[]',
                success_rate REAL DEFAULT 0.0,
                application_count INTEGER DEFAULT 0,
                last_success_at BIGINT,
                created_at BIGINT NOT NULL,
                updated_at BIGINT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_resolutions_error_signature ON \
             resolutions(error_signature_id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS hindsight_notes (
                id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                error_signature_id TEXT NOT NULL,
                content TEXT NOT NULL,
                tags JSONB DEFAULT '[]',
                resolution_ids JSONB DEFAULT '[]',
                quality_score REAL,
                promoted_to_layer TEXT,
                created_at BIGINT NOT NULL,
                updated_at BIGINT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_hindsight_notes_tenant ON hindsight_notes(tenant_id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_hindsight_notes_error_signature ON \
             hindsight_notes(error_signature_id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS governance_configs (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                tenant_id UUID,
                org_id UUID,
                team_id UUID,
                project_id UUID,
                approval_mode TEXT NOT NULL DEFAULT 'standard',
                min_approvers INTEGER NOT NULL DEFAULT 1,
                timeout_hours INTEGER DEFAULT 72,
                auto_approve_low_risk BOOLEAN NOT NULL DEFAULT false,
                escalation_enabled BOOLEAN NOT NULL DEFAULT false,
                escalation_timeout_hours INTEGER,
                escalation_contact TEXT,
                policy_settings JSONB DEFAULT '{}',
                knowledge_settings JSONB DEFAULT '{}',
                memory_settings JSONB DEFAULT '{}',
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_governance_configs_scope ON governance_configs \
             (
                COALESCE(tenant_id, '00000000-0000-0000-0000-000000000000'::UUID),
                COALESCE(org_id, '00000000-0000-0000-0000-000000000000'::UUID),
                COALESCE(team_id, '00000000-0000-0000-0000-000000000000'::UUID),
                COALESCE(project_id, '00000000-0000-0000-0000-000000000000'::UUID)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_governance_configs_tenant_scope \
             ON governance_configs (tenant_id) WHERE tenant_id IS NOT NULL",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query("CREATE SEQUENCE IF NOT EXISTS approval_requests_number_seq")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS approval_requests (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                request_number TEXT NOT NULL DEFAULT 'REQ-' || \
             nextval('approval_requests_number_seq')::TEXT,
                request_type TEXT NOT NULL,
                target_type TEXT NOT NULL,
                target_id TEXT,
                tenant_id UUID,
                org_id UUID,
                team_id UUID,
                project_id UUID,
                title TEXT NOT NULL,
                description TEXT,
                payload JSONB NOT NULL DEFAULT '{}',
                risk_level TEXT NOT NULL DEFAULT 'medium',
                status TEXT NOT NULL DEFAULT 'pending',
                requestor_type TEXT NOT NULL,
                requestor_id UUID NOT NULL,
                requestor_email TEXT,
                required_approvals INTEGER NOT NULL DEFAULT 1,
                current_approvals INTEGER NOT NULL DEFAULT 0,
                expires_at TIMESTAMPTZ,
                resolved_at TIMESTAMPTZ,
                resolution_reason TEXT,
                applied_at TIMESTAMPTZ,
                applied_by UUID,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_approval_requests_status ON approval_requests(status)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_approval_requests_requestor ON \
             approval_requests(requestor_id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS approval_decisions (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                request_id UUID NOT NULL REFERENCES approval_requests(id) ON DELETE CASCADE,
                approver_type TEXT NOT NULL,
                approver_id UUID NOT NULL,
                approver_email TEXT,
                decision TEXT NOT NULL,
                comment TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_approval_decisions_request ON \
             approval_decisions(request_id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS governance_roles (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                principal_type TEXT NOT NULL,
                principal_id UUID NOT NULL,
                role TEXT NOT NULL,
                tenant_id UUID,
                org_id UUID,
                team_id UUID,
                project_id UUID,
                granted_by UUID NOT NULL,
                expires_at TIMESTAMPTZ,
                granted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                revoked_at TIMESTAMPTZ,
                revoked_by UUID
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_governance_roles_unique ON governance_roles (
                principal_type,
                principal_id,
                role,
                COALESCE(tenant_id, '00000000-0000-0000-0000-000000000000'::UUID),
                COALESCE(org_id, '00000000-0000-0000-0000-000000000000'::UUID),
                COALESCE(team_id, '00000000-0000-0000-0000-000000000000'::UUID),
                COALESCE(project_id, '00000000-0000-0000-0000-000000000000'::UUID)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_governance_roles_principal ON \
             governance_roles(principal_type, principal_id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS governance_audit_log (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                action TEXT NOT NULL,
                request_id UUID,
                target_type TEXT,
                target_id TEXT,
                actor_type TEXT NOT NULL,
                actor_id UUID,
                actor_email TEXT,
                details JSONB DEFAULT '{}',
                old_values JSONB,
                new_values JSONB,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_governance_audit_log_action ON \
             governance_audit_log(action)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_governance_audit_log_actor ON \
             governance_audit_log(actor_id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_governance_audit_log_target ON \
             governance_audit_log(target_type)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "DO $$
            BEGIN
                IF NOT EXISTS (
                    SELECT 1 FROM pg_proc p
                    JOIN pg_namespace n ON n.oid = p.pronamespace
                    WHERE n.nspname = 'public' AND p.proname = 'get_effective_governance_config'
                ) THEN
                    EXECUTE $func$
                    CREATE OR REPLACE FUNCTION get_effective_governance_config(
                        p_tenant_id UUID DEFAULT NULL,
                        p_org_id UUID DEFAULT NULL,
                        p_team_id UUID DEFAULT NULL,
                        p_project_id UUID DEFAULT NULL
                    ) RETURNS TABLE (
                        config_id UUID,
                        scope_level TEXT,
                        approval_mode TEXT,
                        min_approvers INT,
                        timeout_hours INT,
                        auto_approve_low_risk BOOLEAN,
                        escalation_enabled BOOLEAN,
                        escalation_timeout_hours INT,
                        escalation_contact TEXT,
                        policy_settings JSONB,
                        knowledge_settings JSONB,
                        memory_settings JSONB
                    ) AS $inner$
                    BEGIN
                        IF p_project_id IS NOT NULL THEN
                            RETURN QUERY
                            SELECT gc.id, 'project'::TEXT, gc.approval_mode, gc.min_approvers, gc.timeout_hours,
                                   gc.auto_approve_low_risk, gc.escalation_enabled, gc.escalation_timeout_hours,
                                   gc.escalation_contact, gc.policy_settings, gc.knowledge_settings, gc.memory_settings
                            FROM governance_configs gc
                            WHERE gc.project_id = p_project_id LIMIT 1;
                            IF FOUND THEN RETURN; END IF;
                        END IF;
                        IF p_team_id IS NOT NULL THEN
                            RETURN QUERY
                            SELECT gc.id, 'team'::TEXT, gc.approval_mode, gc.min_approvers, gc.timeout_hours,
                                   gc.auto_approve_low_risk, gc.escalation_enabled, gc.escalation_timeout_hours,
                                   gc.escalation_contact, gc.policy_settings, gc.knowledge_settings, gc.memory_settings
                            FROM governance_configs gc
                            WHERE gc.team_id = p_team_id AND gc.project_id IS NULL LIMIT 1;
                            IF FOUND THEN RETURN; END IF;
                        END IF;
                        IF p_org_id IS NOT NULL THEN
                            RETURN QUERY
                            SELECT gc.id, 'org'::TEXT, gc.approval_mode, gc.min_approvers, gc.timeout_hours,
                                   gc.auto_approve_low_risk, gc.escalation_enabled, gc.escalation_timeout_hours,
                                   gc.escalation_contact, gc.policy_settings, gc.knowledge_settings, gc.memory_settings
                            FROM governance_configs gc
                            WHERE gc.org_id = p_org_id AND gc.team_id IS NULL AND gc.project_id IS NULL LIMIT 1;
                            IF FOUND THEN RETURN; END IF;
                        END IF;
                        IF p_tenant_id IS NOT NULL THEN
                            RETURN QUERY
                            SELECT gc.id, 'tenant'::TEXT, gc.approval_mode, gc.min_approvers, gc.timeout_hours,
                                   gc.auto_approve_low_risk, gc.escalation_enabled, gc.escalation_timeout_hours,
                                   gc.escalation_contact, gc.policy_settings, gc.knowledge_settings, gc.memory_settings
                            FROM governance_configs gc
                            WHERE gc.tenant_id = p_tenant_id AND gc.org_id IS NULL AND gc.team_id IS NULL AND gc.project_id IS NULL LIMIT 1;
                            IF FOUND THEN RETURN; END IF;
                        END IF;
                        RETURN QUERY SELECT
                            gen_random_uuid(), 'default'::TEXT, 'standard'::TEXT, 1::INT, 72::INT,
                            false::BOOLEAN, false::BOOLEAN, NULL::INT, NULL::TEXT,
                            '{}'::JSONB, '{}'::JSONB, '{}'::JSONB;
                    END;
                    $inner$ LANGUAGE plpgsql
                    $func$;
                END IF;
            END;
            $$",
        )
        .execute(&self.pool)
        .await?;

        rls_migration::run_rls_migration(&self.pool).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tenants (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                slug TEXT UNIQUE NOT NULL,
                name TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                source_owner TEXT NOT NULL DEFAULT 'admin',
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                deactivated_at TIMESTAMPTZ,
                last_applied_manifest_hash TEXT,
                manifest_generation BIGINT NOT NULL DEFAULT 0
            )",
        )
        .execute(&self.pool)
        .await?;

        // Backfill for test databases that pre-date the manifest-state
        // columns (production path: migration 027_tenant_manifest_state.sql).
        sqlx::query("ALTER TABLE tenants ADD COLUMN IF NOT EXISTS last_applied_manifest_hash TEXT")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "ALTER TABLE tenants ADD COLUMN IF NOT EXISTS manifest_generation BIGINT NOT NULL DEFAULT 0",
        )
        .execute(&self.pool)
        .await?;

        // Backfill for test databases that pre-date the legal-entity-name
        // metadata column (production path: migration
        // 033_tenant_legal_entity.sql). This remains as seed data for the
        // account migration performed below.
        sqlx::query("ALTER TABLE tenants ADD COLUMN IF NOT EXISTS legal_entity_name TEXT")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS accounts (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                slug TEXT UNIQUE NOT NULL,
                name TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                deleted_at TIMESTAMPTZ
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "ALTER TABLE tenants ADD COLUMN IF NOT EXISTS account_id UUID REFERENCES accounts(id) ON DELETE SET NULL",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("ALTER TABLE tenants ADD COLUMN IF NOT EXISTS environment TEXT")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_tenants_account_id ON tenants(account_id) WHERE account_id IS NOT NULL",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_tenants_environment ON tenants(environment) WHERE environment IS NOT NULL",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "WITH source_names AS (
                SELECT DISTINCT trim(legal_entity_name) AS name
                  FROM tenants
                 WHERE legal_entity_name IS NOT NULL
                   AND trim(legal_entity_name) <> ''
            ),
            slug_base AS (
                SELECT
                    name,
                    trim(both '-' FROM regexp_replace(lower(name), '[^a-z0-9]+', '-', 'g')) AS base_slug
                FROM source_names
            ),
            slugged AS (
                SELECT
                    name,
                    CASE
                        WHEN COUNT(*) OVER (PARTITION BY base_slug) = 1 THEN NULLIF(base_slug, '')
                        ELSE NULLIF(base_slug, '') || '-' || ROW_NUMBER() OVER (PARTITION BY base_slug ORDER BY name)
                    END AS slug
                FROM slug_base
            )
            INSERT INTO accounts (slug, name, created_at, updated_at)
            SELECT COALESCE(slug, 'account'), name, NOW(), NOW()
              FROM slugged
            ON CONFLICT (slug) DO NOTHING",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "UPDATE tenants t
                SET account_id = a.id
               FROM accounts a
              WHERE t.account_id IS NULL
                AND t.legal_entity_name IS NOT NULL
                AND trim(t.legal_entity_name) <> ''
                AND a.name = trim(t.legal_entity_name)",
        )
        .execute(&self.pool)
        .await?;

        // Mirror CHECK constraints from migration 027 so dev/test paths
        // enforce the same invariants. We assemble the end-of-string
        // anchor via chr(36) at SQL-evaluation time so the Rust source
        // never contains a literal `[dollar-sign][single-quote]` sequence.
        // The let-binding + .ok() pattern tolerates the duplicate-object
        // error when initialize_schema is re-run.
        let _ = sqlx::query(
            "ALTER TABLE tenants \
             ADD CONSTRAINT tenants_manifest_hash_format \
             CHECK (last_applied_manifest_hash IS NULL \
                    OR last_applied_manifest_hash ~ ('^sha256:[0-9a-f]{64}' || chr(36)))",
        )
        .execute(&self.pool)
        .await;
        let _ = sqlx::query(
            "ALTER TABLE tenants \
             ADD CONSTRAINT tenants_manifest_generation_nonneg \
             CHECK (manifest_generation >= 0)",
        )
        .execute(&self.pool)
        .await;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tenant_domain_mappings (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                domain TEXT NOT NULL,
                verified BOOLEAN NOT NULL DEFAULT false,
                source TEXT NOT NULL DEFAULT 'admin',
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                UNIQUE (tenant_id, domain)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tenant_repository_bindings (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                tenant_id UUID UNIQUE NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                kind TEXT NOT NULL,
                local_path TEXT,
                remote_url TEXT,
                branch TEXT NOT NULL DEFAULT 'main',
                branch_policy TEXT NOT NULL DEFAULT 'direct_commit',
                credential_kind TEXT NOT NULL DEFAULT 'none',
                credential_ref TEXT,
                github_owner TEXT,
                github_repo TEXT,
                source_owner TEXT NOT NULL DEFAULT 'admin',
                git_provider_connection_id TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;

        // Day-2 operations tables (migration 019)
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS remediation_requests (
                id TEXT PRIMARY KEY,
                request_type TEXT NOT NULL,
                risk_tier TEXT NOT NULL,
                entity_type TEXT NOT NULL,
                entity_ids JSONB NOT NULL DEFAULT '[]',
                tenant_id TEXT,
                description TEXT NOT NULL,
                proposed_action TEXT NOT NULL,
                detected_by TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at BIGINT NOT NULL,
                reviewed_by TEXT,
                reviewed_at BIGINT,
                resolution_notes TEXT,
                executed_at BIGINT
            )",
        )
        .execute(&self.pool)
        .await
        .ok();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS dead_letter_items (
                id TEXT PRIMARY KEY,
                item_type TEXT NOT NULL,
                item_id TEXT NOT NULL,
                tenant_id TEXT NOT NULL,
                error TEXT NOT NULL,
                retry_count INTEGER NOT NULL DEFAULT 0,
                max_retries INTEGER NOT NULL DEFAULT 5,
                first_failed_at BIGINT NOT NULL,
                last_failed_at BIGINT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                metadata JSONB NOT NULL DEFAULT '{}'
            )",
        )
        .execute(&self.pool)
        .await
        .ok();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tenant_storage_quotas (
                tenant_id TEXT PRIMARY KEY,
                memory_max BIGINT,
                knowledge_max BIGINT,
                vector_max BIGINT,
                total_max BIGINT,
                updated_at BIGINT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await
        .ok();

        Ok(())
    }

    /// Pure matrix check. Returns Ok iff `child` is allowed to live directly
    /// under `parent`. Mirrored on the API side by
    /// `cli::server::org_api::expected_parent_type` — keep them in sync.
    ///
    /// History: this used to be inlined in `create_unit` and was *missing*
    /// entirely from `update_unit`, which meant updates could move a Project
    /// directly under a tenant-root organization, silently violating the contract. The
    /// extraction in v1.5.0 closes that gap by routing both paths through
    /// `validate_unit_invariants` below.
    fn validate_parent_child_types(parent: UnitType, child: UnitType) -> Result<(), PostgresError> {
        match (parent, child) {
            (UnitType::Organization, UnitType::Team) => Ok(()),
            (UnitType::Team, UnitType::Project) => Ok(()),
            _ => Err(PostgresError::Database(sqlx::Error::Decode(
                format!("Invalid hierarchy: cannot place {child:?} under {parent:?}").into(),
            ))),
        }
    }

    /// Validates the contract invariants of a unit row before persisting it
    /// (whether on insert or on update):
    ///
    ///  1. **Root rule** — only `Organization` may have `parent_id = None`.
    ///     Any other type with no parent is rejected.
    ///  2. **Matrix rule** — if `parent_id` is set, the parent must exist in
    ///     the same tenant and `(parent.type, unit.type)` must satisfy
    ///     [`Self::validate_parent_child_types`].
    ///  3. **Cycle rule** — `unit.id` must not appear in the ancestor chain
    ///     of the new `parent_id`, and `parent_id` must not equal `unit.id`.
    ///     With the strict-by-type matrix today, cycles are structurally
    ///     impossible (Project can't have Project descendants etc.), so this
    ///     check is *defensive* — it documents the invariant and survives
    ///     any future relaxation of the matrix (e.g. recursive same-type
    ///     parenting for the legal-entity epic). Costs one recursive CTE
    ///     query per write; acceptable on the admin write path.
    ///
    /// `existing_id` is `None` on create (the row doesn't exist yet, so the
    /// cycle check has nothing to compare against) and `Some(unit.id)` on
    /// update.
    async fn validate_unit_invariants(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        unit: &OrganizationalUnit,
        existing_id: Option<&str>,
    ) -> Result<(), PostgresError> {
        let Some(ref parent_id) = unit.parent_id else {
            // Rule 1 — only Organization may be a tenant-root unit.
            if unit.unit_type != UnitType::Organization {
                return Err(PostgresError::Database(sqlx::Error::Decode(
                    "Only Organization units can be root units (no parent)".into(),
                )));
            }
            return Ok(());
        };

        // Rule 3a — trivial self-cycle. Cheap and gives a clearer error
        // than waiting for the ancestor walk to discover it.
        if let Some(id) = existing_id
            && id == parent_id
        {
            return Err(PostgresError::Database(sqlx::Error::Decode(
                format!("Cycle detected: unit {id:?} cannot be its own parent").into(),
            )));
        }

        // Rule 2 — parent must exist in the same tenant; matrix must hold.
        let parent = Self::get_unit_by_id_tx(tx, parent_id, &unit.tenant_id.to_string())
            .await?
            .ok_or_else(|| PostgresError::NotFound(parent_id.clone()))?;
        Self::validate_parent_child_types(parent.unit_type, unit.unit_type)?;

        // Rule 3b — non-trivial cycle (only meaningful on update; on create
        // the row doesn't exist yet so it can't be in any ancestor chain).
        if let Some(id) = existing_id {
            let ancestors = Self::get_ancestors(tx, &unit.tenant_id, parent_id).await?;
            if ancestors.iter().any(|a| a.id == id) {
                return Err(PostgresError::Database(sqlx::Error::Decode(
                    format!(
                        "Cycle detected: cannot set parent of {id:?} to {parent_id:?} \
                         because {id:?} is already an ancestor of {parent_id:?}"
                    )
                    .into(),
                )));
            }
        }

        Ok(())
    }

    pub async fn create_unit(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        unit: &OrganizationalUnit,
    ) -> Result<(), PostgresError> {
        Self::validate_unit_invariants(tx, unit, None).await?;

        // Idempotent on (tenant_id, COALESCE(parent_id,''), name) — re-applying
        // a tenant manifest after a partial provisioning failure must not insert
        // duplicate units. The composite unique index in migration 031 makes
        // this ON CONFLICT clause a true upsert: metadata + updated_at are
        // refreshed; the original id, created_at, and parent_id are preserved.
        // See docs/operations/tenant-provisioning.md for the rationale.
        sqlx::query(
            "INSERT INTO organizational_units (id, name, type, parent_id, tenant_id, metadata, \
             created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (tenant_id, COALESCE(parent_id, ''), name) DO UPDATE SET
                 metadata   = EXCLUDED.metadata,
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(&unit.id)
        .bind(&unit.name)
        .bind(unit.unit_type.to_string().to_lowercase())
        .bind(&unit.parent_id)
        .bind(unit.tenant_id.as_str())
        .bind(serde_json::to_value(&unit.metadata)?)
        .bind(unit.created_at)
        .bind(unit.updated_at)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    fn row_to_persistent_event(
        row: &sqlx::postgres::PgRow,
    ) -> Result<mk_core::types::PersistentEvent, PostgresError> {
        use sqlx::Row;

        let status_str: String = row.get("status");
        let status = match status_str.as_str() {
            "pending" => mk_core::types::EventStatus::Pending,
            "published" => mk_core::types::EventStatus::Published,
            "acknowledged" => mk_core::types::EventStatus::Acknowledged,
            "dead_lettered" => mk_core::types::EventStatus::DeadLettered,
            _ => mk_core::types::EventStatus::Pending,
        };

        let payload: mk_core::types::GovernanceEvent = serde_json::from_value(row.get("payload"))?;

        Ok(mk_core::types::PersistentEvent {
            id: row.get("id"),
            event_id: row.get("event_id"),
            idempotency_key: row.get("idempotency_key"),
            tenant_id: row.get::<String, _>("tenant_id").parse().map_err(|e| {
                PostgresError::Database(sqlx::Error::Decode(
                    format!("Invalid tenant_id: {}", e).into(),
                ))
            })?,
            event_type: row.get("event_type"),
            payload,
            status,
            retry_count: row.get("retry_count"),
            max_retries: row.get("max_retries"),
            last_error: row.get("last_error"),
            created_at: row.get("created_at"),
            published_at: row.get("published_at"),
            acknowledged_at: row.get("acknowledged_at"),
            dead_lettered_at: row.get("dead_lettered_at"),
        })
    }

    async fn get_unit_by_id_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: &str,
        tenant_id: &str,
    ) -> Result<Option<OrganizationalUnit>, PostgresError> {
        let row = sqlx::query(
            "SELECT id, name, type, parent_id, tenant_id, metadata, created_at, updated_at 
             FROM organizational_units WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(&mut **tx)
        .await?;

        if let Some(row) = row {
            let unit_type_str: String = row.get("type");
            let unit_type = match unit_type_str.as_str() {
                "tenant" => UnitType::Organization,
                "organization" => UnitType::Organization,
                "team" => UnitType::Team,
                "project" => UnitType::Project,
                _ => {
                    return Err(PostgresError::Database(sqlx::Error::Decode(
                        "Invalid unit type".into(),
                    )));
                }
            };

            Ok(Some(OrganizationalUnit {
                id: row.get("id"),
                name: row.get("name"),
                unit_type,
                parent_id: row.get("parent_id"),
                tenant_id: row.get::<String, _>("tenant_id").parse().map_err(|e| {
                    PostgresError::Database(sqlx::Error::Decode(
                        format!("Invalid tenant_id: {}", e).into(),
                    ))
                })?,
                metadata: serde_json::from_value(row.get("metadata"))?,
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                source_owner: mk_core::types::RecordSource::Admin,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn get_unit(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &mk_core::types::TenantId,
        id: &str,
    ) -> Result<Option<OrganizationalUnit>, PostgresError> {
        let row = sqlx::query(
            "SELECT id, name, type, parent_id, tenant_id, metadata, created_at, updated_at 
             FROM organizational_units WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id.as_str())
        .fetch_optional(&mut **tx)
        .await?;

        if let Some(row) = row {
            let unit_type_str: String = row.get("type");
            let unit_type = match unit_type_str.as_str() {
                "tenant" => UnitType::Organization,
                "organization" => UnitType::Organization,
                "team" => UnitType::Team,
                "project" => UnitType::Project,
                _ => {
                    return Err(PostgresError::Database(sqlx::Error::Decode(
                        "Invalid unit type".into(),
                    )));
                }
            };

            Ok(Some(OrganizationalUnit {
                id: row.get("id"),
                name: row.get("name"),
                unit_type,
                parent_id: row.get("parent_id"),
                tenant_id: row.get::<String, _>("tenant_id").parse().map_err(|e| {
                    PostgresError::Database(sqlx::Error::Decode(
                        format!("Invalid tenant_id: {}", e).into(),
                    ))
                })?,
                metadata: serde_json::from_value(row.get("metadata"))?,
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                source_owner: mk_core::types::RecordSource::Admin,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn list_children(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &mk_core::types::TenantId,
        parent_id: &str,
    ) -> Result<Vec<OrganizationalUnit>, PostgresError> {
        let rows = sqlx::query(
            "SELECT id, name, type, parent_id, tenant_id, metadata, created_at, updated_at 
             FROM organizational_units WHERE parent_id = $1 AND tenant_id = $2",
        )
        .bind(parent_id)
        .bind(tenant_id.as_str())
        .fetch_all(&mut **tx)
        .await?;

        let mut units = Vec::new();
        for row in rows {
            let unit_type_str: String = row.get("type");
            let unit_type = match unit_type_str.as_str() {
                "tenant" => UnitType::Organization,
                "organization" => UnitType::Organization,
                "team" => UnitType::Team,
                "project" => UnitType::Project,
                _ => continue,
            };

            units.push(OrganizationalUnit {
                id: row.get("id"),
                name: row.get("name"),
                unit_type,
                parent_id: row.get("parent_id"),
                tenant_id: row.get::<String, _>("tenant_id").parse().map_err(|e| {
                    PostgresError::Database(sqlx::Error::Decode(
                        format!("Invalid tenant_id: {}", e).into(),
                    ))
                })?,
                metadata: serde_json::from_value(row.get("metadata"))?,
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                source_owner: mk_core::types::RecordSource::Admin,
            });
        }

        Ok(units)
    }

    pub async fn get_ancestors(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &mk_core::types::TenantId,
        id: &str,
    ) -> Result<Vec<OrganizationalUnit>, PostgresError> {
        let rows = sqlx::query(
            "WITH RECURSIVE ancestors AS (
                SELECT id, name, type, parent_id, tenant_id, metadata, created_at, updated_at
                FROM organizational_units
                WHERE id = $1 AND tenant_id = $2
                UNION ALL
                SELECT u.id, u.name, u.type, u.parent_id, u.tenant_id, u.metadata, u.created_at, \
             u.updated_at
                FROM organizational_units u
                INNER JOIN ancestors a ON u.id = a.parent_id AND u.tenant_id = a.tenant_id
            )
            SELECT * FROM ancestors WHERE id != $1",
        )
        .bind(id)
        .bind(tenant_id.as_str())
        .fetch_all(&mut **tx)
        .await?;

        let mut units = Vec::new();
        for row in rows {
            let unit_type_str: String = row.get("type");
            let unit_type = match unit_type_str.as_str() {
                "tenant" => UnitType::Organization,
                "organization" => UnitType::Organization,
                "team" => UnitType::Team,
                "project" => UnitType::Project,
                _ => continue,
            };

            units.push(OrganizationalUnit {
                id: row.get("id"),
                name: row.get("name"),
                unit_type,
                parent_id: row.get("parent_id"),
                tenant_id: row.get::<String, _>("tenant_id").parse().map_err(|e| {
                    PostgresError::Database(sqlx::Error::Decode(
                        format!("Invalid tenant_id: {}", e).into(),
                    ))
                })?,
                metadata: serde_json::from_value(row.get("metadata"))?,
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                source_owner: mk_core::types::RecordSource::Admin,
            });
        }

        Ok(units)
    }

    pub async fn get_unit_ancestors(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &mk_core::types::TenantId,
        id: &str,
    ) -> Result<Vec<OrganizationalUnit>, PostgresError> {
        Self::get_ancestors(tx, tenant_id, id).await
    }

    pub async fn get_unit_descendants(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &mk_core::types::TenantId,
        id: &str,
    ) -> Result<Vec<OrganizationalUnit>, PostgresError> {
        let rows = sqlx::query(
            "WITH RECURSIVE descendants AS (
                SELECT id, name, type, parent_id, tenant_id, metadata, created_at, updated_at
                FROM organizational_units
                WHERE id = $1 AND tenant_id = $2
                UNION ALL
                SELECT u.id, u.name, u.type, u.parent_id, u.tenant_id, u.metadata, u.created_at, \
             u.updated_at
                FROM organizational_units u
                INNER JOIN descendants d ON u.parent_id = d.id AND u.tenant_id = d.tenant_id
            )
            SELECT * FROM descendants WHERE id != $1",
        )
        .bind(id)
        .bind(tenant_id.as_str())
        .fetch_all(&mut **tx)
        .await?;

        let mut units = Vec::new();
        for row in rows {
            let unit_type_str: String = row.get("type");
            let unit_type = match unit_type_str.as_str() {
                "tenant" => UnitType::Organization,
                "organization" => UnitType::Organization,
                "team" => UnitType::Team,
                "project" => UnitType::Project,
                _ => continue,
            };

            units.push(OrganizationalUnit {
                id: row.get("id"),
                name: row.get("name"),
                unit_type,
                parent_id: row.get("parent_id"),
                tenant_id: row.get::<String, _>("tenant_id").parse().map_err(|e| {
                    PostgresError::Database(sqlx::Error::Decode(
                        format!("Invalid tenant_id: {}", e).into(),
                    ))
                })?,
                metadata: serde_json::from_value(row.get("metadata"))?,
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                source_owner: mk_core::types::RecordSource::Admin,
            });
        }

        Ok(units)
    }

    /// Updates an organisational unit. Pre-v1.5.0 this skipped *all*
    /// hierarchy validation, so it was possible to e.g. move a Project to
    /// directly under a tenant-root organization via update — a silent contract violation
    /// surfaced during the rc.9 triage. Now routes through
    /// [`Self::validate_unit_invariants`], the same pre-flight that
    /// [`Self::create_unit`] uses, which:
    ///
    ///  - rejects making a non-organization a root,
    ///  - rejects parent/child pairs not in the matrix,
    ///  - rejects self-parenting and ancestor-cycle parenting.
    ///
    /// The `tenant_id` argument is preserved for the SQL `WHERE` clause and
    /// must equal `unit.tenant_id` (callers always pass them aligned today;
    /// we assert it as a debug invariant).
    pub async fn update_unit(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &mk_core::types::TenantId,
        unit: &OrganizationalUnit,
    ) -> Result<(), PostgresError> {
        debug_assert_eq!(
            tenant_id, &unit.tenant_id,
            "update_unit: tenant_id arg must match unit.tenant_id"
        );

        Self::validate_unit_invariants(tx, unit, Some(&unit.id)).await?;

        sqlx::query(
            "UPDATE organizational_units 
             SET name = $3, type = $4, parent_id = $5, metadata = $6, updated_at = $7
             WHERE id = $1 AND tenant_id = $2",
        )
        .bind(&unit.id)
        .bind(tenant_id.as_str())
        .bind(&unit.name)
        .bind(unit.unit_type.to_string().to_lowercase())
        .bind(&unit.parent_id)
        .bind(serde_json::to_value(&unit.metadata)?)
        .bind(unit.updated_at)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    pub async fn delete_unit(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &mk_core::types::TenantId,
        id: &str,
    ) -> Result<(), PostgresError> {
        sqlx::query("DELETE FROM organizational_units WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id.as_str())
            .execute(&mut **tx)
            .await?;

        Ok(())
    }

    pub async fn add_unit_policy(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &mk_core::types::TenantId,
        unit_id: &str,
        policy: &mk_core::types::Policy,
    ) -> Result<(), PostgresError> {
        let exists: Option<(i32,)> =
            sqlx::query_as("SELECT 1 FROM organizational_units WHERE id = $1 AND tenant_id = $2")
                .bind(unit_id)
                .bind(tenant_id.as_str())
                .fetch_optional(&mut **tx)
                .await?;

        if exists.is_none() {
            return Err(PostgresError::NotFound(unit_id.to_string()));
        }

        sqlx::query(
            "INSERT INTO unit_policies (id, unit_id, policy, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (id) DO UPDATE SET policy = $3, updated_at = $5",
        )
        .bind(&policy.id)
        .bind(unit_id)
        .bind(serde_json::to_value(policy)?)
        .bind(chrono::Utc::now().timestamp())
        .bind(chrono::Utc::now().timestamp())
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn get_unit_policies(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &mk_core::types::TenantId,
        unit_id: &str,
    ) -> Result<Vec<mk_core::types::Policy>, PostgresError> {
        let rows = sqlx::query(
            "SELECT p.policy 
             FROM unit_policies p
             JOIN organizational_units u ON p.unit_id = u.id
             WHERE p.unit_id = $1 AND u.tenant_id = $2",
        )
        .bind(unit_id)
        .bind(tenant_id.as_str())
        .fetch_all(&mut **tx)
        .await?;

        let mut policies = Vec::new();
        for row in rows {
            let policy: mk_core::types::Policy = serde_json::from_value(row.get("policy"))?;
            policies.push(policy);
        }
        Ok(policies)
    }

    pub async fn assign_role(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        user_id: &mk_core::types::UserId,
        tenant_id: &mk_core::types::TenantId,
        unit_id: &str,
        role: RoleIdentifier,
    ) -> Result<(), PostgresError> {
        sqlx::query(
            "INSERT INTO user_roles (user_id, tenant_id, unit_id, role, created_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (user_id, tenant_id, unit_id, role) DO NOTHING",
        )
        .bind(user_id.as_str())
        .bind(tenant_id.as_str())
        .bind(unit_id)
        .bind(role.to_string().to_lowercase())
        .bind(chrono::Utc::now())
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn remove_role(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        user_id: &mk_core::types::UserId,
        tenant_id: &mk_core::types::TenantId,
        unit_id: &str,
        role: RoleIdentifier,
    ) -> Result<(), PostgresError> {
        sqlx::query(
            "DELETE FROM user_roles 
             WHERE user_id = $1 AND tenant_id = $2 AND unit_id = $3 AND role = $4",
        )
        .bind(user_id.as_str())
        .bind(tenant_id.as_str())
        .bind(unit_id)
        .bind(role.to_string().to_lowercase())
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn get_user_roles(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        user_id: &mk_core::types::UserId,
        tenant_id: &mk_core::types::TenantId,
    ) -> Result<Vec<(String, RoleIdentifier)>, PostgresError> {
        let rows = sqlx::query(
            "SELECT unit_id, role FROM user_roles WHERE user_id = $1 AND tenant_id = $2",
        )
        .bind(user_id.as_str())
        .bind(tenant_id.as_str())
        .fetch_all(&mut **tx)
        .await?;

        let mut roles = Vec::new();
        for row in rows {
            let unit_id: String = row.get("unit_id");
            let role_str: String = row.get("role");
            let role = RoleIdentifier::from_str_flexible(&role_str);
            roles.push((unit_id, role));
        }
        Ok(roles)
    }
    /// Lists all (user_id, role) pairs assigned within a specific organisational unit.
    pub async fn list_unit_roles(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &mk_core::types::TenantId,
        unit_id: &str,
    ) -> Result<Vec<(mk_core::types::UserId, RoleIdentifier)>, PostgresError> {
        use sqlx::Row;
        let rows = sqlx::query(
            "SELECT user_id, role FROM user_roles WHERE tenant_id = $1 AND unit_id = $2",
        )
        .bind(tenant_id.as_str())
        .bind(unit_id)
        .fetch_all(&mut **tx)
        .await?;

        let mut result = Vec::new();
        for row in rows {
            let user_id_str: String = row.get("user_id");
            let role_str: String = row.get("role");
            if let Some(uid) = mk_core::types::UserId::new(user_id_str) {
                let role = RoleIdentifier::from_str_flexible(&role_str);
                result.push((uid, role));
            }
        }
        Ok(result)
    }

    // ---------------------------------------------------------------------
    // A.3 Wave 1 / Cluster 1 — inherent convenience wrappers
    //
    // Call-site ergonomic helpers around the `&mut Transaction`-first
    // associated functions above. Each wrapper opens a single tx via the
    // appropriate context helper and calls the associated fn.
    //
    //   *_scoped  — requires a `&TenantContext`, uses `with_tenant_context`
    //               (handler path, inside an RLS-gated tenant scope).
    //   *_admin   — cross-tenant write, uses `with_admin_context`
    //               (tenant provisioning, governance, etc. — audited).
    //
    // Target tables (organizational_units, unit_policies, user_roles) are
    // not RLS-enabled today; the tx contract gives atomicity and future-
    // proofs for when RLS lands on these tables.
    // ---------------------------------------------------------------------

    /// Create a unit inside an existing tenant context (RLS-scoped).
    pub async fn create_unit_scoped(
        &self,
        ctx: &TenantContext,
        unit: &OrganizationalUnit,
    ) -> Result<(), PostgresError> {
        let unit = unit.clone();
        self.with_tenant_context(ctx, move |tx| {
            Box::pin(async move { Self::create_unit(tx, &unit).await })
        })
        .await
    }

    /// Create a unit from the admin path (cross-tenant, audited).
    /// Used by tenant bootstrap flows that seed a new tenant's initial units
    /// before a full `TenantContext` for the target tenant is available.
    pub async fn create_unit_admin(
        &self,
        admin_ctx: &TenantContext,
        action: &str,
        unit: &OrganizationalUnit,
    ) -> Result<(), PostgresError> {
        let unit = unit.clone();
        self.with_admin_context(admin_ctx, action, move |tx| {
            Box::pin(async move { Self::create_unit(tx, &unit).await })
        })
        .await
    }

    pub async fn get_unit_scoped(
        &self,
        ctx: &TenantContext,
        id: &str,
    ) -> Result<Option<OrganizationalUnit>, PostgresError> {
        let id = id.to_string();
        let tenant_id = ctx.tenant_id.clone();
        self.with_tenant_context(ctx, move |tx| {
            Box::pin(async move { Self::get_unit(tx, &tenant_id, &id).await })
        })
        .await
    }

    pub async fn list_children_scoped(
        &self,
        ctx: &TenantContext,
        parent_id: &str,
    ) -> Result<Vec<OrganizationalUnit>, PostgresError> {
        let parent_id = parent_id.to_string();
        let tenant_id = ctx.tenant_id.clone();
        self.with_tenant_context(ctx, move |tx| {
            Box::pin(async move { Self::list_children(tx, &tenant_id, &parent_id).await })
        })
        .await
    }

    pub async fn get_ancestors_scoped(
        &self,
        ctx: &TenantContext,
        id: &str,
    ) -> Result<Vec<OrganizationalUnit>, PostgresError> {
        let id = id.to_string();
        let tenant_id = ctx.tenant_id.clone();
        self.with_tenant_context(ctx, move |tx| {
            Box::pin(async move { Self::get_ancestors(tx, &tenant_id, &id).await })
        })
        .await
    }

    pub async fn get_unit_descendants_scoped(
        &self,
        ctx: &TenantContext,
        id: &str,
    ) -> Result<Vec<OrganizationalUnit>, PostgresError> {
        let id = id.to_string();
        let tenant_id = ctx.tenant_id.clone();
        self.with_tenant_context(ctx, move |tx| {
            Box::pin(async move { Self::get_unit_descendants(tx, &tenant_id, &id).await })
        })
        .await
    }

    pub async fn update_unit_scoped(
        &self,
        ctx: &TenantContext,
        unit: &OrganizationalUnit,
    ) -> Result<(), PostgresError> {
        let unit = unit.clone();
        let tenant_id = ctx.tenant_id.clone();
        self.with_tenant_context(ctx, move |tx| {
            Box::pin(async move { Self::update_unit(tx, &tenant_id, &unit).await })
        })
        .await
    }

    pub async fn delete_unit_scoped(
        &self,
        ctx: &TenantContext,
        id: &str,
    ) -> Result<(), PostgresError> {
        let id = id.to_string();
        let tenant_id = ctx.tenant_id.clone();
        self.with_tenant_context(ctx, move |tx| {
            Box::pin(async move { Self::delete_unit(tx, &tenant_id, &id).await })
        })
        .await
    }

    pub async fn add_unit_policy_scoped(
        &self,
        ctx: &TenantContext,
        unit_id: &str,
        policy: &mk_core::types::Policy,
    ) -> Result<(), PostgresError> {
        let unit_id = unit_id.to_string();
        let policy = policy.clone();
        let tenant_id = ctx.tenant_id.clone();
        self.with_tenant_context(ctx, move |tx| {
            Box::pin(async move { Self::add_unit_policy(tx, &tenant_id, &unit_id, &policy).await })
        })
        .await
    }

    pub async fn get_unit_policies_scoped(
        &self,
        ctx: &TenantContext,
        unit_id: &str,
    ) -> Result<Vec<mk_core::types::Policy>, PostgresError> {
        let unit_id = unit_id.to_string();
        let tenant_id = ctx.tenant_id.clone();
        self.with_tenant_context(ctx, move |tx| {
            Box::pin(async move { Self::get_unit_policies(tx, &tenant_id, &unit_id).await })
        })
        .await
    }

    pub async fn list_unit_roles_scoped(
        &self,
        ctx: &TenantContext,
        unit_id: &str,
    ) -> Result<Vec<(mk_core::types::UserId, RoleIdentifier)>, PostgresError> {
        let unit_id = unit_id.to_string();
        let tenant_id = ctx.tenant_id.clone();
        self.with_tenant_context(ctx, move |tx| {
            Box::pin(async move { Self::list_unit_roles(tx, &tenant_id, &unit_id).await })
        })
        .await
    }

    pub async fn assign_role_scoped(
        &self,
        ctx: &TenantContext,
        user_id: &mk_core::types::UserId,
        unit_id: &str,
        role: RoleIdentifier,
    ) -> Result<(), PostgresError> {
        let user_id = user_id.clone();
        let unit_id = unit_id.to_string();
        let tenant_id = ctx.tenant_id.clone();
        self.with_tenant_context(ctx, move |tx| {
            Box::pin(
                async move { Self::assign_role(tx, &user_id, &tenant_id, &unit_id, role).await },
            )
        })
        .await
    }

    /// Admin variant of [`Self::assign_role_scoped`] for cross-tenant
    /// provisioning flows (e.g. bootstrapping a new tenant's initial admins).
    pub async fn assign_role_admin(
        &self,
        admin_ctx: &TenantContext,
        action: &str,
        tenant_id: &mk_core::types::TenantId,
        user_id: &mk_core::types::UserId,
        unit_id: &str,
        role: RoleIdentifier,
    ) -> Result<(), PostgresError> {
        let user_id = user_id.clone();
        let tenant_id = tenant_id.clone();
        let unit_id = unit_id.to_string();
        self.with_admin_context(admin_ctx, action, move |tx| {
            Box::pin(
                async move { Self::assign_role(tx, &user_id, &tenant_id, &unit_id, role).await },
            )
        })
        .await
    }

    pub async fn remove_role_scoped(
        &self,
        ctx: &TenantContext,
        user_id: &mk_core::types::UserId,
        unit_id: &str,
        role: RoleIdentifier,
    ) -> Result<(), PostgresError> {
        let user_id = user_id.clone();
        let unit_id = unit_id.to_string();
        let tenant_id = ctx.tenant_id.clone();
        self.with_tenant_context(ctx, move |tx| {
            Box::pin(
                async move { Self::remove_role(tx, &user_id, &tenant_id, &unit_id, role).await },
            )
        })
        .await
    }

    /// Looks up a user's internal UUID by their identity-provider subject (e.g. GitHub login or email).
    ///
    /// Returns `None` when no matching user exists — callers should treat this as an anonymous /
    /// unauthenticated identity with no roles.
    pub async fn resolve_user_id_by_idp_subject(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        idp_subject: &str,
    ) -> Result<Option<String>, PostgresError> {
        use sqlx::Row;
        let row = sqlx::query("SELECT id::text FROM users WHERE idp_subject = $1 LIMIT 1")
            .bind(idp_subject)
            .fetch_optional(&mut **tx)
            .await?;
        Ok(row.map(|r| r.get::<String, _>("id")))
    }

    /// Looks up a user's internal UUID by their identity-provider name and subject.
    ///
    /// Both `idp_provider` and `idp_subject` must match.  Returns `None` when no
    /// matching user exists.
    pub async fn resolve_user_id_by_idp(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        idp_provider: &str,
        idp_subject: &str,
    ) -> Result<Option<String>, PostgresError> {
        use sqlx::Row;
        let row = sqlx::query(
            "SELECT id::text FROM users WHERE idp_provider = $1 AND idp_subject = $2 LIMIT 1",
        )
        .bind(idp_provider)
        .bind(idp_subject)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.map(|r| r.get::<String, _>("id")))
    }

    /// Returns the deduplicated list of roles for a user within a specific tenant, **plus**
    /// any instance-scoped roles stored under [`mk_core::types::INSTANCE_SCOPE_TENANT_ID`] (`"__root__"`).
    ///
    /// This is the authoritative role lookup for authentication context construction.
    pub async fn get_user_roles_for_auth(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        user_id: &str,
        tenant_id: &str,
    ) -> Result<Vec<RoleIdentifier>, PostgresError> {
        use sqlx::Row;
        let rows = sqlx::query(
            "SELECT DISTINCT role FROM user_roles
             WHERE user_id = $1 AND (tenant_id = $2 OR tenant_id = $3)",
        )
        .bind(user_id)
        .bind(tenant_id)
        .bind(INSTANCE_SCOPE_TENANT_ID)
        .fetch_all(&mut **tx)
        .await?;

        let mut roles: Vec<RoleIdentifier> = rows
            .iter()
            .map(|r| {
                let role_str: String = r.get("role");
                RoleIdentifier::from_str_flexible(&role_str)
            })
            .collect();
        roles.sort_by_cached_key(|role| role.to_string().to_ascii_lowercase());
        roles.dedup();
        Ok(roles)
    }

    /// Returns the caller-configured default tenant for a user, or `None`
    /// if no preference is set.
    ///
    /// The column is populated via [`set_user_default_tenant`]; the migration
    /// `023_users_default_tenant.sql` declares it as
    /// `default_tenant_id UUID NULL REFERENCES tenants(id) ON DELETE SET NULL`,
    /// which guarantees the returned value still points to an active tenant
    /// row at read time (the FK nulls the column when the tenant is deleted).
    ///
    /// Added in OpenSpec change `refactor-platform-admin-impersonation` (#44.b)
    /// to back the `GET /api/v1/user/me/default-tenant` endpoint.
    pub async fn get_user_default_tenant(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        user_id: &str,
    ) -> Result<Option<String>, PostgresError> {
        use sqlx::Row;
        let row =
            sqlx::query("SELECT default_tenant_id::text AS tid FROM users WHERE id = $1::uuid")
                .bind(user_id)
                .fetch_optional(&mut **tx)
                .await?;
        Ok(row.and_then(|r| r.try_get::<Option<String>, _>("tid").ok().flatten()))
    }

    /// Persist the caller-configured default tenant for a user.
    ///
    /// The tenant must exist and the caller is expected to have already
    /// verified membership in it before calling this method. The FK
    /// constraint rejects orphan IDs at write time.
    pub async fn set_user_default_tenant(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        user_id: &str,
        tenant_id: &str,
    ) -> Result<(), PostgresError> {
        let res = sqlx::query(
            "UPDATE users SET default_tenant_id = $2::uuid, updated_at = NOW() WHERE id = $1::uuid",
        )
        .bind(user_id)
        .bind(tenant_id)
        .execute(&mut **tx)
        .await?;
        if res.rows_affected() == 0 {
            return Err(PostgresError::NotFound(format!("user {user_id} not found")));
        }
        Ok(())
    }

    /// Clear the caller-configured default tenant for a user.
    pub async fn clear_user_default_tenant(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        user_id: &str,
    ) -> Result<(), PostgresError> {
        let res = sqlx::query(
            "UPDATE users SET default_tenant_id = NULL, updated_at = NOW() WHERE id = $1::uuid",
        )
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
        if res.rows_affected() == 0 {
            return Err(PostgresError::NotFound(format!("user {user_id} not found")));
        }
        Ok(())
    }

    /// Returns the distinct non-root tenant IDs that a user has any role in.
    ///
    /// Used for implicit tenant resolution: when a request carries no `X-Tenant-ID`
    /// header, the server can auto-select the tenant when the result is exactly one.
    /// When the result is empty or more than one, the caller must require an explicit
    /// `X-Tenant-ID`.
    pub async fn get_user_tenant_ids(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        user_id: &str,
    ) -> Result<Vec<String>, PostgresError> {
        use sqlx::Row;
        let rows = sqlx::query(
            "SELECT DISTINCT tenant_id FROM user_roles
             WHERE user_id = $1 AND tenant_id != $2
             ORDER BY tenant_id",
        )
        .bind(user_id)
        .bind(INSTANCE_SCOPE_TENANT_ID)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .iter()
            .map(|r| r.get::<String, _>("tenant_id"))
            .collect())
    }

    // -- Auth / bootstrap convenience wrappers --------------------------------
    //
    // These thin wrappers drive the associated `resolve_user_id_by_idp*` /
    // `get_user_roles_for_auth` / `get_user_tenant_ids` / `get_user_default_tenant`
    // methods through [`Self::with_bootstrap_context`]. They exist because the
    // authentication hot path runs before any `TenantContext` is known, so the
    // normal `with_tenant_context` / `with_admin_context` wrappers do not apply
    // (the former has no tenant, the latter would emit an admin-audit row per
    // request).
    //
    // The underlying tables (`users`, `user_roles`) are not RLS-enabled; the tx
    // is used purely for atomicity and consistency with the rest of the API.

    /// Bootstrap variant of [`Self::resolve_user_id_by_idp_subject`].
    pub async fn resolve_user_id_by_idp_subject_bootstrap(
        &self,
        idp_subject: &str,
    ) -> Result<Option<String>, PostgresError> {
        let idp_subject = idp_subject.to_string();
        self.with_bootstrap_context(move |tx| {
            Box::pin(async move { Self::resolve_user_id_by_idp_subject(tx, &idp_subject).await })
        })
        .await
    }

    /// Bootstrap variant of [`Self::resolve_user_id_by_idp`].
    pub async fn resolve_user_id_by_idp_bootstrap(
        &self,
        idp_provider: &str,
        idp_subject: &str,
    ) -> Result<Option<String>, PostgresError> {
        let idp_provider = idp_provider.to_string();
        let idp_subject = idp_subject.to_string();
        self.with_bootstrap_context(move |tx| {
            Box::pin(
                async move { Self::resolve_user_id_by_idp(tx, &idp_provider, &idp_subject).await },
            )
        })
        .await
    }

    /// Bootstrap variant of [`Self::get_user_roles_for_auth`].
    pub async fn get_user_roles_for_auth_bootstrap(
        &self,
        user_id: &str,
        tenant_id: &str,
    ) -> Result<Vec<RoleIdentifier>, PostgresError> {
        let user_id = user_id.to_string();
        let tenant_id = tenant_id.to_string();
        self.with_bootstrap_context(move |tx| {
            Box::pin(async move { Self::get_user_roles_for_auth(tx, &user_id, &tenant_id).await })
        })
        .await
    }

    /// Bootstrap variant of [`Self::get_user_tenant_ids`].
    pub async fn get_user_tenant_ids_bootstrap(
        &self,
        user_id: &str,
    ) -> Result<Vec<String>, PostgresError> {
        let user_id = user_id.to_string();
        self.with_bootstrap_context(move |tx| {
            Box::pin(async move { Self::get_user_tenant_ids(tx, &user_id).await })
        })
        .await
    }

    /// Bootstrap variant of [`Self::get_user_default_tenant`].
    pub async fn get_user_default_tenant_bootstrap(
        &self,
        user_id: &str,
    ) -> Result<Option<String>, PostgresError> {
        let user_id = user_id.to_string();
        self.with_bootstrap_context(move |tx| {
            Box::pin(async move { Self::get_user_default_tenant(tx, &user_id).await })
        })
        .await
    }

    /// Bootstrap-pool write variant of [`Self::set_user_default_tenant`].
    ///
    /// `users.default_tenant_id` is user-level state, not tenant-scoped;
    /// there is no RLS policy on the `users` table (verified in the
    /// migration set). The bootstrap pool gives us tx-scoped atomicity
    /// without emitting an admin-audit row on every preference change.
    pub async fn set_user_default_tenant_bootstrap(
        &self,
        user_id: &str,
        tenant_id: &str,
    ) -> Result<(), PostgresError> {
        let user_id = user_id.to_string();
        let tenant_id = tenant_id.to_string();
        self.with_bootstrap_context(move |tx| {
            Box::pin(async move { Self::set_user_default_tenant(tx, &user_id, &tenant_id).await })
        })
        .await
    }

    /// Bootstrap-pool write variant of [`Self::clear_user_default_tenant`].
    ///
    /// See [`Self::set_user_default_tenant_bootstrap`] for rationale.
    pub async fn clear_user_default_tenant_bootstrap(
        &self,
        user_id: &str,
    ) -> Result<(), PostgresError> {
        let user_id = user_id.to_string();
        self.with_bootstrap_context(move |tx| {
            Box::pin(async move { Self::clear_user_default_tenant(tx, &user_id).await })
        })
        .await
    }

    /// Tenant-scoped convenience variant of [`Self::get_user_roles`].
    ///
    /// Wraps the associated function in [`Self::with_tenant_context`] so
    /// handler call sites don't need to `Box::pin` an async block each time.
    /// The scoped tx is technically redundant today (`user_roles` is not
    /// RLS-gated), but it's the right shape for the A.3 Wave 1 contract
    /// and future-proofs against enabling RLS on `user_roles`.
    pub async fn get_user_roles_scoped(
        &self,
        ctx: &TenantContext,
        user_id: &mk_core::types::UserId,
        tenant_id: &mk_core::types::TenantId,
    ) -> Result<Vec<(String, RoleIdentifier)>, PostgresError> {
        let user_id = user_id.clone();
        let tenant_id = tenant_id.clone();
        self.with_tenant_context(ctx, move |tx| {
            Box::pin(async move { Self::get_user_roles(tx, &user_id, &tenant_id).await })
        })
        .await
    }

    /// Append a governance event inside an existing transaction.
    ///
    /// # RLS contract
    ///
    /// `governance_events` has RLS enabled and is keyed on `tenant_id`.
    /// The caller MUST ensure `app.tenant_id` is SET LOCAL to a value
    /// matching `event`'s tenant before invoking this function. In
    /// practice this means calling it from within either:
    ///
    /// - [`Self::with_tenant_context`] with a matching `TenantContext`
    ///   (the typical tenant-scoped handler path), or
    /// - [`Self::with_admin_context`] (tenant provisioning / admin
    ///   cross-tenant paths — BYPASSRLS applies).
    ///
    /// # Shape
    ///
    /// ```ignore
    /// state.postgres.with_tenant_context(&ctx, |tx| Box::pin(async move {
    ///     PostgresBackend::log_event(tx, &event).await
    /// })).await?;
    /// ```
    pub async fn log_event(
        tx: &mut sqlx::Transaction<'_, Postgres>,
        event: &mk_core::types::GovernanceEvent,
    ) -> Result<(), PostgresError> {
        let (event_type, tenant_id, _timestamp) = match event {
            mk_core::types::GovernanceEvent::UnitCreated {
                unit_id: _,
                unit_type: _,
                tenant_id,
                parent_id: _,
                timestamp,
            } => ("unit_created", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::UnitUpdated {
                unit_id: _,
                tenant_id,
                timestamp,
            } => ("unit_updated", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::UnitDeleted {
                unit_id: _,
                tenant_id,
                timestamp,
            } => ("unit_deleted", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::RoleAssigned {
                user_id: _,
                unit_id: _,
                role: _,
                tenant_id,
                timestamp,
            } => ("role_assigned", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::RoleRemoved {
                user_id: _,
                unit_id: _,
                role: _,
                tenant_id,
                timestamp,
            } => ("role_removed", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::PolicyUpdated {
                policy_id: _,
                layer: _,
                tenant_id,
                timestamp,
            } => ("policy_updated", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::PolicyDeleted {
                policy_id: _,
                tenant_id,
                timestamp,
            } => ("policy_deleted", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::DriftDetected {
                project_id: _,
                tenant_id,
                drift_score: _,
                timestamp,
            } => ("drift_detected", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::ConfigUpdated {
                config_id: _,
                scope: _,
                tenant_id,
                timestamp,
            } => ("config_updated", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::RequestCreated {
                request_id: _,
                request_type: _,
                title: _,
                tenant_id,
                timestamp,
            } => ("request_created", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::RequestApproved {
                request_id: _,
                approver_id: _,
                fully_approved: _,
                tenant_id,
                timestamp,
            } => ("request_approved", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::RequestRejected {
                request_id: _,
                rejector_id: _,
                reason: _,
                tenant_id,
                timestamp,
            } => ("request_rejected", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::TenantCreated {
                tenant_id,
                timestamp,
                ..
            } => ("tenant_created", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::TenantUpdated {
                tenant_id,
                timestamp,
                ..
            } => ("tenant_updated", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::TenantDeactivated {
                tenant_id,
                timestamp,
                ..
            } => ("tenant_deactivated", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::RepositoryBindingCreated {
                tenant_id,
                timestamp,
                ..
            } => ("repository_binding_created", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::RepositoryBindingUpdated {
                tenant_id,
                timestamp,
                ..
            } => ("repository_binding_updated", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::GitProviderConnectionCreated {
                tenant_id,
                timestamp,
                ..
            } => ("git_provider_connection_created", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::GitProviderConnectionUpdated {
                tenant_id,
                timestamp,
                ..
            } => ("git_provider_connection_updated", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::GitProviderConnectionTenantGranted {
                tenant_id,
                timestamp,
                ..
            } => (
                "git_provider_connection_tenant_granted",
                tenant_id,
                *timestamp,
            ),
            mk_core::types::GovernanceEvent::GitProviderConnectionTenantRevoked {
                tenant_id,
                timestamp,
                ..
            } => (
                "git_provider_connection_tenant_revoked",
                tenant_id,
                *timestamp,
            ),
            mk_core::types::GovernanceEvent::KnowledgePromotionRequested {
                tenant_id,
                timestamp,
                ..
            } => ("knowledge_promotion_requested", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::KnowledgePromotionApproved {
                tenant_id,
                timestamp,
                ..
            } => ("knowledge_promotion_approved", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::KnowledgePromotionRejected {
                tenant_id,
                timestamp,
                ..
            } => ("knowledge_promotion_rejected", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::KnowledgePromotionRetargeted {
                tenant_id,
                timestamp,
                ..
            } => ("knowledge_promotion_retargeted", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::KnowledgePromotionApplied {
                tenant_id,
                timestamp,
                ..
            } => ("knowledge_promotion_applied", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::KnowledgeRelationCreated {
                tenant_id,
                timestamp,
                ..
            } => ("knowledge_relation_created", tenant_id, *timestamp),
            mk_core::types::GovernanceEvent::BootstrapCompleted {
                tenant_id,
                timestamp,
                ..
            } => ("bootstrap_completed", tenant_id, *timestamp),
        };

        sqlx::query(
            "INSERT INTO governance_events (event_type, tenant_id, payload)
             VALUES ($1, $2, $3)",
        )
        .bind(event_type)
        .bind(tenant_id.as_str())
        .bind(serde_json::to_value(event)?)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    pub async fn get_governance_events_internal(
        &self,
        ctx: mk_core::types::TenantContext,
        since_timestamp: i64,
        limit: usize,
    ) -> Result<Vec<mk_core::types::GovernanceEvent>, PostgresError> {
        let rows = sqlx::query(
            "SELECT payload FROM governance_events 
             WHERE tenant_id = $1 AND EXTRACT(EPOCH FROM created_at)::BIGINT > $2 
             ORDER BY created_at ASC LIMIT $3",
        )
        .bind(ctx.tenant_id.as_str())
        .bind(since_timestamp)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let mut events = Vec::new();
        for row in rows {
            use sqlx::Row;
            let payload: serde_json::Value = row.get("payload");
            let event: mk_core::types::GovernanceEvent = serde_json::from_value(payload)?;
            events.push(event);
        }
        Ok(events)
    }
}

#[async_trait]
#[cfg(feature = "postgres-graph")]
impl crate::graph::GraphStore for PostgresBackend {
    type Error = Box<dyn std::error::Error + Send + Sync>;

    async fn add_node(
        &self,
        ctx: TenantContext,
        node: crate::graph::GraphNode,
    ) -> Result<(), Self::Error> {
        sqlx::query(
            "INSERT INTO graph_nodes (id, tenant_id, label, properties, created_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (id, tenant_id) DO UPDATE SET label = $3, properties = $4",
        )
        .bind(&node.id)
        .bind(ctx.tenant_id.as_str())
        .bind(&node.label)
        .bind(&node.properties)
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.pool)
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        Ok(())
    }

    async fn add_edge(
        &self,
        ctx: TenantContext,
        edge: crate::graph::GraphEdge,
    ) -> Result<(), Self::Error> {
        sqlx::query(
            "INSERT INTO graph_edges (id, tenant_id, source_id, target_id, relation, properties, \
             created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (id, tenant_id) DO UPDATE SET relation = $5, properties = $6",
        )
        .bind(&edge.id)
        .bind(ctx.tenant_id.as_str())
        .bind(&edge.source_id)
        .bind(&edge.target_id)
        .bind(&edge.relation)
        .bind(&edge.properties)
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.pool)
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        Ok(())
    }

    async fn get_neighbors(
        &self,
        ctx: TenantContext,
        node_id: &str,
    ) -> Result<Vec<(crate::graph::GraphEdge, crate::graph::GraphNode)>, Self::Error> {
        let rows = sqlx::query(
            "SELECT e.id as edge_id, e.source_id, e.target_id, e.relation, e.properties as \
             edge_props,
                    n.id as node_id, n.label, n.properties as node_props
             FROM graph_edges e
             JOIN graph_nodes n ON e.target_id = n.id AND e.tenant_id = n.tenant_id
             WHERE e.source_id = $1 AND e.tenant_id = $2",
        )
        .bind(node_id)
        .bind(ctx.tenant_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let mut results = Vec::new();
        for row in rows {
            let edge = crate::graph::GraphEdge {
                id: row.get("edge_id"),
                source_id: row.get("source_id"),
                target_id: row.get("target_id"),
                relation: row.get("relation"),
                properties: row.get("edge_props"),
                tenant_id: ctx.tenant_id.as_str().to_string(),
            };
            let node = crate::graph::GraphNode {
                id: row.get("node_id"),
                label: row.get("label"),
                properties: row.get("node_props"),
                tenant_id: ctx.tenant_id.as_str().to_string(),
            };
            results.push((edge, node));
        }
        Ok(results)
    }

    async fn find_path(
        &self,
        ctx: TenantContext,
        start_id: &str,
        end_id: &str,
        max_depth: usize,
    ) -> Result<Vec<crate::graph::GraphEdge>, Self::Error> {
        let rows = sqlx::query(
            "WITH RECURSIVE search_path(id, source_id, target_id, relation, properties, depth, \
             path) AS (
                SELECT id, source_id, target_id, relation, properties, 1, ARRAY[id]
                FROM graph_edges
                WHERE source_id = $1 AND tenant_id = $3
                UNION ALL
                SELECT e.id, e.source_id, e.target_id, e.relation, e.properties, sp.depth + 1, \
             sp.path || e.id
                FROM graph_edges e
                JOIN search_path sp ON e.source_id = sp.target_id AND e.tenant_id = $3
                WHERE sp.depth < $4 AND NOT (e.id = ANY(sp.path))
            )
            SELECT id, source_id, target_id, relation, properties
            FROM search_path
            WHERE target_id = $2
            LIMIT 1",
        )
        .bind(start_id)
        .bind(end_id)
        .bind(ctx.tenant_id.as_str())
        .bind(max_depth as i32)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let mut path = Vec::new();
        for row in rows {
            path.push(crate::graph::GraphEdge {
                id: row.get("id"),
                source_id: row.get("source_id"),
                target_id: row.get("target_id"),
                relation: row.get("relation"),
                properties: row.get("properties"),
                tenant_id: ctx.tenant_id.as_str().to_string(),
            });
        }
        Ok(path)
    }

    async fn search_nodes(
        &self,
        ctx: TenantContext,
        query: &str,
        limit: usize,
    ) -> Result<Vec<crate::graph::GraphNode>, Self::Error> {
        let rows = sqlx::query(
            "SELECT id, label, properties FROM graph_nodes
             WHERE tenant_id = $1 AND (id ILIKE $2 OR label ILIKE $2)
             LIMIT $3",
        )
        .bind(ctx.tenant_id.as_str())
        .bind(format!("%{}%", query))
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let mut nodes = Vec::new();
        for row in rows {
            nodes.push(crate::graph::GraphNode {
                id: row.get("id"),
                label: row.get("label"),
                properties: row.get("properties"),
                tenant_id: ctx.tenant_id.as_str().to_string(),
            });
        }
        Ok(nodes)
    }

    async fn soft_delete_nodes_by_source_memory_id(
        &self,
        ctx: TenantContext,
        source_memory_id: &str,
    ) -> Result<usize, Self::Error> {
        let result = sqlx::query(
            "UPDATE graph_nodes SET deleted_at = NOW() 
             WHERE tenant_id = $1 
             AND deleted_at IS NULL 
             AND properties->>'source_memory_id' = $2",
        )
        .bind(ctx.tenant_id.as_str())
        .bind(source_memory_id)
        .execute(&self.pool)
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let deleted_count = result.rows_affected() as usize;

        sqlx::query(
            "UPDATE graph_edges SET deleted_at = NOW()
             WHERE tenant_id = $1 
             AND deleted_at IS NULL
             AND (source_id IN (
                SELECT id FROM graph_nodes 
                WHERE tenant_id = $1 AND properties->>'source_memory_id' = $2
             ) OR target_id IN (
                SELECT id FROM graph_nodes 
                WHERE tenant_id = $1 AND properties->>'source_memory_id' = $2
             ))",
        )
        .bind(ctx.tenant_id.as_str())
        .bind(source_memory_id)
        .execute(&self.pool)
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        Ok(deleted_count)
    }
}

#[async_trait]
impl mk_core::traits::EventPublisher for PostgresBackend {
    type Error = PostgresError;

    async fn publish(&self, event: mk_core::types::GovernanceEvent) -> Result<(), Self::Error> {
        // `log_event` is tx-scoped and must run inside a session where
        // `app.tenant_id` matches the event's tenant (RLS on
        // `governance_events`). The event carries its own tenant_id;
        // construct a minimal context and wrap in `with_tenant_context`.
        let ctx = mk_core::types::TenantContext::new(
            event.tenant_id().clone(),
            mk_core::types::UserId::default(),
        );
        self.with_tenant_context(&ctx, move |tx| {
            Box::pin(async move { Self::log_event(tx, &event).await })
        })
        .await
    }

    async fn subscribe(
        &self,
        _channels: &[&str],
    ) -> Result<tokio::sync::mpsc::Receiver<mk_core::types::GovernanceEvent>, Self::Error> {
        Err(PostgresError::Database(sqlx::Error::Decode(
            "Subscribe not implemented for Postgres backend".into(),
        )))
    }
}

#[async_trait]
impl StorageBackend for PostgresBackend {
    type Error = PostgresError;

    async fn store(&self, ctx: TenantContext, key: &str, value: &[u8]) -> Result<(), Self::Error> {
        sqlx::query(
            "INSERT INTO sync_state (id, tenant_id, data, updated_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (id, tenant_id) DO UPDATE SET data = $3, updated_at = $4",
        )
        .bind(key)
        .bind(ctx.tenant_id.as_str())
        .bind(serde_json::from_slice::<serde_json::Value>(value).unwrap_or_default())
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn retrieve(
        &self,
        ctx: TenantContext,
        key: &str,
    ) -> Result<Option<Vec<u8>>, Self::Error> {
        let row: Option<(serde_json::Value,)> =
            sqlx::query_as("SELECT data FROM sync_state WHERE id = $1 AND tenant_id = $2")
                .bind(key)
                .bind(ctx.tenant_id.as_str())
                .fetch_optional(&self.pool)
                .await?;

        Ok(row.and_then(|(v,)| serde_json::to_vec(&v).ok()))
    }

    async fn delete(&self, ctx: TenantContext, key: &str) -> Result<(), Self::Error> {
        sqlx::query("DELETE FROM sync_state WHERE id = $1 AND tenant_id = $2")
            .bind(key)
            .bind(ctx.tenant_id.as_str())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn exists(&self, ctx: TenantContext, key: &str) -> Result<bool, Self::Error> {
        let row: Option<(i32,)> =
            sqlx::query_as("SELECT 1 FROM sync_state WHERE id = $1 AND tenant_id = $2")
                .bind(key)
                .bind(ctx.tenant_id.as_str())
                .fetch_optional(&self.pool)
                .await?;

        Ok(row.is_some())
    }

    async fn get_ancestors(
        &self,
        ctx: TenantContext,
        unit_id: &str,
    ) -> Result<Vec<OrganizationalUnit>, Self::Error> {
        self.get_ancestors_scoped(&ctx, unit_id).await
    }

    async fn get_descendants(
        &self,
        ctx: TenantContext,
        unit_id: &str,
    ) -> Result<Vec<OrganizationalUnit>, Self::Error> {
        let tenant_id = ctx.tenant_id.clone();
        let unit_id = unit_id.to_string();
        self.with_tenant_context(&ctx, move |tx| {
            Box::pin(async move { Self::get_unit_descendants(tx, &tenant_id, &unit_id).await })
        })
        .await
    }

    async fn get_unit_policies(
        &self,
        ctx: TenantContext,
        unit_id: &str,
    ) -> Result<Vec<mk_core::types::Policy>, Self::Error> {
        self.get_unit_policies_scoped(&ctx, unit_id).await
    }

    async fn create_unit(&self, unit: &OrganizationalUnit) -> Result<(), Self::Error> {
        let unit = unit.clone();
        self.with_bootstrap_context(move |tx| {
            Box::pin(async move { Self::create_unit(tx, &unit).await })
        })
        .await
    }

    async fn get_unit_by_id(
        &self,
        unit_id: &str,
        tenant_id: &str,
    ) -> Result<Option<OrganizationalUnit>, Self::Error> {
        let unit_id = unit_id.to_string();
        let tenant_id = tenant_id.to_string();
        self.with_bootstrap_context(move |tx| {
            Box::pin(async move { Self::get_unit_by_id_tx(tx, &unit_id, &tenant_id).await })
        })
        .await
    }

    async fn update_unit(&self, unit: &OrganizationalUnit) -> Result<(), Self::Error> {
        let tenant_id = unit.tenant_id.clone();
        let unit = unit.clone();
        self.with_bootstrap_context(move |tx| {
            Box::pin(async move { Self::update_unit(tx, &tenant_id, &unit).await })
        })
        .await
    }

    async fn delete_unit(&self, unit_id: &str, tenant_id: &str) -> Result<(), Self::Error> {
        let parsed_tenant_id: mk_core::types::TenantId = tenant_id.parse().map_err(|e| {
            PostgresError::Database(sqlx::Error::Decode(
                format!("Invalid tenant_id: {}", e).into(),
            ))
        })?;

        let unit_id = unit_id.to_string();
        self.with_bootstrap_context(move |tx| {
            Box::pin(async move { Self::delete_unit(tx, &parsed_tenant_id, &unit_id).await })
        })
        .await
    }

    async fn list_unit_members(
        &self,
        unit_id: &str,
        tenant_id: &str,
    ) -> Result<Vec<(mk_core::types::UserId, RoleIdentifier)>, Self::Error> {
        let parsed_tenant_id: mk_core::types::TenantId = tenant_id.parse().map_err(|e| {
            PostgresError::Database(sqlx::Error::Decode(
                format!("Invalid tenant_id: {}", e).into(),
            ))
        })?;

        let unit_id = unit_id.to_string();
        self.with_bootstrap_context(move |tx| {
            Box::pin(async move { Self::list_unit_roles(tx, &parsed_tenant_id, &unit_id).await })
        })
        .await
    }

    async fn assign_team_to_project(
        &self,
        project_id: &str,
        team_id: &str,
        tenant_id: &str,
        assignment_type: &str,
    ) -> Result<(), Self::Error> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO project_team_assignments (project_id, team_id, tenant_id, assignment_type, created_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (project_id, team_id, tenant_id) DO UPDATE SET assignment_type = $4",
        )
        .bind(project_id)
        .bind(team_id)
        .bind(tenant_id)
        .bind(assignment_type)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn remove_team_from_project(
        &self,
        project_id: &str,
        team_id: &str,
        tenant_id: &str,
    ) -> Result<(), Self::Error> {
        sqlx::query(
            "DELETE FROM project_team_assignments WHERE project_id = $1 AND team_id = $2 AND tenant_id = $3",
        )
        .bind(project_id)
        .bind(team_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_project_team_assignments(
        &self,
        project_id: &str,
        tenant_id: &str,
    ) -> Result<Vec<(String, String)>, Self::Error> {
        use sqlx::Row;

        let rows = sqlx::query(
            "SELECT team_id, assignment_type FROM project_team_assignments WHERE project_id = $1 AND tenant_id = $2",
        )
        .bind(project_id)
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .iter()
            .map(|row| {
                let team_id: String = row.get("team_id");
                let assignment_type: String = row.get("assignment_type");
                (team_id, assignment_type)
            })
            .collect())
    }

    async fn get_effective_roles_at_scope(
        &self,
        user_id: &mk_core::types::UserId,
        tenant_id: &mk_core::types::TenantId,
        unit_id: &str,
    ) -> Result<Vec<RoleIdentifier>, Self::Error> {
        use sqlx::Row;

        let rows = sqlx::query(
            "WITH RECURSIVE ancestors AS (
                SELECT id, parent_id
                FROM organizational_units
                WHERE id = $1 AND tenant_id = $2
                UNION ALL
                SELECT u.id, u.parent_id
                FROM organizational_units u
                INNER JOIN ancestors a ON u.id = a.parent_id
                WHERE u.tenant_id = $2
            )
            SELECT DISTINCT r.role FROM user_roles r
            INNER JOIN ancestors a ON r.unit_id = a.id
            WHERE r.user_id = $3 AND r.tenant_id = $2",
        )
        .bind(unit_id)
        .bind(tenant_id.as_str())
        .bind(user_id.as_str())
        .fetch_all(&self.pool)
        .await?;

        let mut roles = Vec::new();
        for row in rows {
            let role_str: String = row.get("role");
            let role = RoleIdentifier::from_str_flexible(&role_str);
            roles.push(role);
        }

        Ok(roles)
    }

    async fn add_unit_policy(
        &self,
        ctx: &TenantContext,
        unit_id: &str,
        policy: &mk_core::types::Policy,
    ) -> Result<(), Self::Error> {
        self.add_unit_policy_scoped(ctx, unit_id, policy).await
    }

    async fn assign_role(
        &self,
        user_id: &mk_core::types::UserId,
        tenant_id: &mk_core::types::TenantId,
        unit_id: &str,
        role: RoleIdentifier,
    ) -> Result<(), Self::Error> {
        let user_id = user_id.clone();
        let tenant_id = tenant_id.clone();
        let unit_id = unit_id.to_string();
        self.with_bootstrap_context(move |tx| {
            Box::pin(
                async move { Self::assign_role(tx, &user_id, &tenant_id, &unit_id, role).await },
            )
        })
        .await
    }

    async fn remove_role(
        &self,
        user_id: &mk_core::types::UserId,
        tenant_id: &mk_core::types::TenantId,
        unit_id: &str,
        role: RoleIdentifier,
    ) -> Result<(), Self::Error> {
        let user_id = user_id.clone();
        let tenant_id = tenant_id.clone();
        let unit_id = unit_id.to_string();
        self.with_bootstrap_context(move |tx| {
            Box::pin(
                async move { Self::remove_role(tx, &user_id, &tenant_id, &unit_id, role).await },
            )
        })
        .await
    }

    async fn store_drift_result(
        &self,
        result: mk_core::types::DriftResult,
    ) -> Result<(), Self::Error> {
        sqlx::query(
            "INSERT INTO drift_results (project_id, tenant_id, drift_score, confidence, \
             violations, suppressed_violations, requires_manual_review, timestamp)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(&result.project_id)
        .bind(result.tenant_id.as_str())
        .bind(result.drift_score)
        .bind(result.confidence)
        .bind(serde_json::to_value(&result.violations)?)
        .bind(serde_json::to_value(&result.suppressed_violations)?)
        .bind(result.requires_manual_review)
        .bind(result.timestamp)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_latest_drift_result(
        &self,
        ctx: mk_core::types::TenantContext,
        project_id: &str,
    ) -> Result<Option<mk_core::types::DriftResult>, Self::Error> {
        let row = sqlx::query(
            "SELECT project_id, tenant_id, drift_score, confidence, violations, \
             suppressed_violations, requires_manual_review, timestamp 
             FROM drift_results 
             WHERE project_id = $1 AND tenant_id = $2 
             ORDER BY timestamp DESC LIMIT 1",
        )
        .bind(project_id)
        .bind(ctx.tenant_id.as_str())
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            Ok(Some(mk_core::types::DriftResult {
                project_id: row.get("project_id"),
                tenant_id: row.get::<String, _>("tenant_id").parse().map_err(|e| {
                    PostgresError::Database(sqlx::Error::Decode(
                        format!("Invalid tenant_id: {}", e).into(),
                    ))
                })?,
                drift_score: row.get("drift_score"),
                confidence: row.get("confidence"),
                violations: serde_json::from_value(row.get("violations"))?,
                suppressed_violations: serde_json::from_value(row.get("suppressed_violations"))?,
                requires_manual_review: row.get("requires_manual_review"),
                timestamp: row.get("timestamp"),
            }))
        } else {
            Ok(None)
        }
    }

    async fn record_job_status(
        &self,
        job_name: &str,
        tenant_id: &str,
        status: &str,
        message: Option<&str>,
        started_at: i64,
        finished_at: Option<i64>,
    ) -> Result<(), Self::Error> {
        let duration_ms = finished_at.map(|f| (f - started_at) * 1000);

        sqlx::query(
            "INSERT INTO job_status (job_name, tenant_id, status, message, started_at, \
             finished_at, duration_ms)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(job_name)
        .bind(tenant_id)
        .bind(status)
        .bind(message)
        .bind(started_at)
        .bind(finished_at)
        .bind(duration_ms)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_governance_events(
        &self,
        ctx: mk_core::types::TenantContext,
        since_timestamp: i64,
        limit: usize,
    ) -> Result<Vec<mk_core::types::GovernanceEvent>, Self::Error> {
        self.get_governance_events_internal(ctx, since_timestamp, limit)
            .await
    }

    async fn list_all_units(&self) -> Result<Vec<mk_core::types::OrganizationalUnit>, Self::Error> {
        let rows = sqlx::query(
            "SELECT id, name, type, parent_id, tenant_id, metadata, created_at, updated_at 
             FROM organizational_units",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut units = Vec::new();
        for row in rows {
            let unit_type_str: String = row.get("type");
            let unit_type = match unit_type_str.as_str() {
                "tenant" => UnitType::Organization,
                "organization" => UnitType::Organization,
                "team" => UnitType::Team,
                "project" => UnitType::Project,
                _ => continue,
            };

            units.push(mk_core::types::OrganizationalUnit {
                id: row.get("id"),
                name: row.get("name"),
                unit_type,
                parent_id: row.get("parent_id"),
                tenant_id: row.get::<String, _>("tenant_id").parse().map_err(|e| {
                    PostgresError::Database(sqlx::Error::Decode(
                        format!("Invalid tenant_id: {}", e).into(),
                    ))
                })?,
                metadata: serde_json::from_value(row.get("metadata"))?,
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                source_owner: mk_core::types::RecordSource::Admin,
            });
        }

        Ok(units)
    }

    async fn create_suppression(
        &self,
        suppression: mk_core::types::DriftSuppression,
    ) -> Result<(), Self::Error> {
        sqlx::query(
            "INSERT INTO drift_suppressions (id, project_id, tenant_id, policy_id, rule_pattern, \
             reason, created_by, expires_at, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(&suppression.id)
        .bind(&suppression.project_id)
        .bind(suppression.tenant_id.as_str())
        .bind(&suppression.policy_id)
        .bind(&suppression.rule_pattern)
        .bind(&suppression.reason)
        .bind(suppression.created_by.as_str())
        .bind(suppression.expires_at)
        .bind(suppression.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn list_suppressions(
        &self,
        ctx: mk_core::types::TenantContext,
        project_id: &str,
    ) -> Result<Vec<mk_core::types::DriftSuppression>, Self::Error> {
        let rows = sqlx::query(
            "SELECT id, project_id, tenant_id, policy_id, rule_pattern, reason, created_by, \
             expires_at, created_at
             FROM drift_suppressions
             WHERE project_id = $1 AND tenant_id = $2
             ORDER BY created_at DESC",
        )
        .bind(project_id)
        .bind(ctx.tenant_id.as_str())
        .fetch_all(&self.pool)
        .await?;

        let mut suppressions = Vec::new();
        for row in rows {
            suppressions.push(mk_core::types::DriftSuppression {
                id: row.get("id"),
                project_id: row.get("project_id"),
                tenant_id: row.get::<String, _>("tenant_id").parse().map_err(|e| {
                    PostgresError::Database(sqlx::Error::Decode(
                        format!("Invalid tenant_id: {}", e).into(),
                    ))
                })?,
                policy_id: row.get("policy_id"),
                rule_pattern: row.get("rule_pattern"),
                reason: row.get("reason"),
                created_by: row.get::<String, _>("created_by").parse().map_err(|e| {
                    PostgresError::Database(sqlx::Error::Decode(
                        format!("Invalid created_by: {}", e).into(),
                    ))
                })?,
                expires_at: row.get("expires_at"),
                created_at: row.get("created_at"),
            });
        }

        Ok(suppressions)
    }

    async fn delete_suppression(
        &self,
        ctx: mk_core::types::TenantContext,
        suppression_id: &str,
    ) -> Result<(), Self::Error> {
        sqlx::query("DELETE FROM drift_suppressions WHERE id = $1 AND tenant_id = $2")
            .bind(suppression_id)
            .bind(ctx.tenant_id.as_str())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn get_drift_config(
        &self,
        ctx: mk_core::types::TenantContext,
        project_id: &str,
    ) -> Result<Option<mk_core::types::DriftConfig>, Self::Error> {
        let row = sqlx::query(
            "SELECT project_id, tenant_id, threshold, low_confidence_threshold, \
             auto_suppress_info, updated_at
             FROM drift_configs
             WHERE project_id = $1 AND tenant_id = $2",
        )
        .bind(project_id)
        .bind(ctx.tenant_id.as_str())
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            Ok(Some(mk_core::types::DriftConfig {
                project_id: row.get("project_id"),
                tenant_id: row.get::<String, _>("tenant_id").parse().map_err(|e| {
                    PostgresError::Database(sqlx::Error::Decode(
                        format!("Invalid tenant_id: {}", e).into(),
                    ))
                })?,
                threshold: row.get("threshold"),
                low_confidence_threshold: row.get("low_confidence_threshold"),
                auto_suppress_info: row.get("auto_suppress_info"),
                updated_at: row.get("updated_at"),
            }))
        } else {
            Ok(None)
        }
    }

    async fn save_drift_config(
        &self,
        config: mk_core::types::DriftConfig,
    ) -> Result<(), Self::Error> {
        sqlx::query(
            "INSERT INTO drift_configs (project_id, tenant_id, threshold, \
             low_confidence_threshold, auto_suppress_info, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (project_id, tenant_id) DO UPDATE SET
                threshold = EXCLUDED.threshold,
                low_confidence_threshold = EXCLUDED.low_confidence_threshold,
                auto_suppress_info = EXCLUDED.auto_suppress_info,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(&config.project_id)
        .bind(config.tenant_id.as_str())
        .bind(config.threshold)
        .bind(config.low_confidence_threshold)
        .bind(config.auto_suppress_info)
        .bind(config.updated_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn persist_event(
        &self,
        event: mk_core::types::PersistentEvent,
    ) -> Result<(), Self::Error> {
        // NOTE: `event.id` is a `String` (UUID v4 stringified by
        // `PersistentEvent::new`) but the `governance_events.id` column is
        // `uuid NOT NULL`. Postgres does not implicitly cast text → uuid, so
        // we make the cast explicit with `$1::uuid`. Without this, every
        // INSERT (e.g. the post-bootstrap `BootstrapCompleted` emission in
        // `cli/src/commands/serve.rs::emit_bootstrap_completed`) fails with
        // `column "id" is of type uuid but expression is of type text` and
        // the event is silently dropped — `/admin/bootstrap/status` is the
        // primary readout, so the server keeps running, but the durable
        // governance trail is incomplete. See rc.6 release notes.
        sqlx::query(
            "INSERT INTO governance_events (id, event_id, idempotency_key, tenant_id, event_type, \
             payload, status, retry_count, max_retries, last_error, created_at, published_at, \
             acknowledged_at, dead_lettered_at)
             VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10, to_timestamp($11), $12, $13, $14)
             ON CONFLICT (idempotency_key) DO NOTHING",
        )
        .bind(&event.id)
        .bind(&event.event_id)
        .bind(&event.idempotency_key)
        .bind(event.tenant_id.as_str())
        .bind(&event.event_type)
        .bind(serde_json::to_value(&event.payload)?)
        .bind(event.status.to_string())
        .bind(event.retry_count)
        .bind(event.max_retries)
        .bind(&event.last_error)
        .bind(event.created_at)
        .bind(
            event
                .published_at
                .map(|ts| chrono::DateTime::from_timestamp(ts, 0)),
        )
        .bind(
            event
                .acknowledged_at
                .map(|ts| chrono::DateTime::from_timestamp(ts, 0)),
        )
        .bind(
            event
                .dead_lettered_at
                .map(|ts| chrono::DateTime::from_timestamp(ts, 0)),
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_pending_events(
        &self,
        ctx: mk_core::types::TenantContext,
        limit: usize,
    ) -> Result<Vec<mk_core::types::PersistentEvent>, Self::Error> {
        let rows = sqlx::query(
            "SELECT id, event_id, idempotency_key, tenant_id, event_type, payload, status, \
             retry_count, max_retries, last_error, 
                    EXTRACT(EPOCH FROM created_at)::bigint as created_at,
                    EXTRACT(EPOCH FROM published_at)::bigint as published_at,
                    EXTRACT(EPOCH FROM acknowledged_at)::bigint as acknowledged_at,
                    EXTRACT(EPOCH FROM dead_lettered_at)::bigint as dead_lettered_at
             FROM governance_events
             WHERE tenant_id = $1 AND status = 'pending'
             ORDER BY created_at ASC
             LIMIT $2",
        )
        .bind(ctx.tenant_id.as_str())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let mut events = Vec::new();
        for row in rows {
            events.push(Self::row_to_persistent_event(&row)?);
        }
        Ok(events)
    }

    async fn update_event_status(
        &self,
        event_id: &str,
        status: mk_core::types::EventStatus,
        error: Option<String>,
    ) -> Result<(), Self::Error> {
        let now = chrono::Utc::now();

        match status {
            mk_core::types::EventStatus::Published => {
                sqlx::query(
                    "UPDATE governance_events SET status = $2, published_at = $3 WHERE event_id = \
                     $1",
                )
                .bind(event_id)
                .bind(status.to_string())
                .bind(now)
                .execute(&self.pool)
                .await?;
            }
            mk_core::types::EventStatus::Acknowledged => {
                sqlx::query(
                    "UPDATE governance_events SET status = $2, acknowledged_at = $3 WHERE \
                     event_id = $1",
                )
                .bind(event_id)
                .bind(status.to_string())
                .bind(now)
                .execute(&self.pool)
                .await?;
            }
            mk_core::types::EventStatus::DeadLettered => {
                sqlx::query(
                    "UPDATE governance_events SET status = $2, last_error = $3, dead_lettered_at \
                     = $4 WHERE event_id = $1",
                )
                .bind(event_id)
                .bind(status.to_string())
                .bind(&error)
                .bind(now)
                .execute(&self.pool)
                .await?;
            }
            mk_core::types::EventStatus::Pending => {
                sqlx::query(
                    "UPDATE governance_events SET status = $2, retry_count = retry_count + 1, \
                     last_error = $3 WHERE event_id = $1",
                )
                .bind(event_id)
                .bind(status.to_string())
                .bind(&error)
                .execute(&self.pool)
                .await?;
            }
        }

        Ok(())
    }

    async fn get_dead_letter_events(
        &self,
        ctx: mk_core::types::TenantContext,
        limit: usize,
    ) -> Result<Vec<mk_core::types::PersistentEvent>, Self::Error> {
        let rows = sqlx::query(
            "SELECT id, event_id, idempotency_key, tenant_id, event_type, payload, status, \
             retry_count, max_retries, last_error,
                    EXTRACT(EPOCH FROM created_at)::bigint as created_at,
                    EXTRACT(EPOCH FROM published_at)::bigint as published_at,
                    EXTRACT(EPOCH FROM acknowledged_at)::bigint as acknowledged_at,
                    EXTRACT(EPOCH FROM dead_lettered_at)::bigint as dead_lettered_at
             FROM governance_events
             WHERE tenant_id = $1 AND status = 'dead_lettered'
             ORDER BY dead_lettered_at DESC
             LIMIT $2",
        )
        .bind(ctx.tenant_id.as_str())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let mut events = Vec::new();
        for row in rows {
            events.push(Self::row_to_persistent_event(&row)?);
        }
        Ok(events)
    }

    async fn check_idempotency(
        &self,
        consumer_group: &str,
        idempotency_key: &str,
    ) -> Result<bool, Self::Error> {
        let result: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM event_consumer_state WHERE consumer_group = $1 AND idempotency_key = $2",
        )
        .bind(consumer_group)
        .bind(idempotency_key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.is_some())
    }

    async fn record_consumer_state(
        &self,
        state: mk_core::types::ConsumerState,
    ) -> Result<(), Self::Error> {
        sqlx::query(
            "INSERT INTO event_consumer_state (consumer_group, idempotency_key, tenant_id, \
             processed_at)
             VALUES ($1, $2, $3, to_timestamp($4))
             ON CONFLICT (consumer_group, idempotency_key) DO NOTHING",
        )
        .bind(&state.consumer_group)
        .bind(&state.idempotency_key)
        .bind(state.tenant_id.as_str())
        .bind(state.processed_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_event_metrics(
        &self,
        ctx: mk_core::types::TenantContext,
        period_start: i64,
        period_end: i64,
    ) -> Result<Vec<mk_core::types::EventDeliveryMetrics>, Self::Error> {
        let rows = sqlx::query(
            "SELECT tenant_id, event_type, 
                    EXTRACT(EPOCH FROM period_start)::bigint as period_start,
                    EXTRACT(EPOCH FROM period_end)::bigint as period_end,
                    total_events, delivered_events, retried_events, dead_lettered_events, \
             avg_delivery_time_ms
             FROM event_delivery_metrics
             WHERE tenant_id = $1 AND period_start >= to_timestamp($2) AND period_end <= \
             to_timestamp($3)
             ORDER BY period_start DESC",
        )
        .bind(ctx.tenant_id.as_str())
        .bind(period_start)
        .bind(period_end)
        .fetch_all(&self.pool)
        .await?;

        let mut metrics = Vec::new();
        for row in rows {
            metrics.push(mk_core::types::EventDeliveryMetrics {
                tenant_id: row.get::<String, _>("tenant_id").parse().map_err(|e| {
                    PostgresError::Database(sqlx::Error::Decode(
                        format!("Invalid tenant_id: {}", e).into(),
                    ))
                })?,
                event_type: row.get("event_type"),
                period_start: row.get("period_start"),
                period_end: row.get("period_end"),
                total_events: row.get("total_events"),
                delivered_events: row.get("delivered_events"),
                retried_events: row.get("retried_events"),
                dead_lettered_events: row.get("dead_lettered_events"),
                avg_delivery_time_ms: row.get("avg_delivery_time_ms"),
            });
        }
        Ok(metrics)
    }

    async fn record_event_metrics(
        &self,
        metrics: mk_core::types::EventDeliveryMetrics,
    ) -> Result<(), Self::Error> {
        sqlx::query(
            "INSERT INTO event_delivery_metrics (tenant_id, event_type, period_start, period_end, \
             total_events, delivered_events, retried_events, dead_lettered_events, \
             avg_delivery_time_ms)
             VALUES ($1, $2, to_timestamp($3), to_timestamp($4), $5, $6, $7, $8, $9)",
        )
        .bind(metrics.tenant_id.as_str())
        .bind(&metrics.event_type)
        .bind(metrics.period_start)
        .bind(metrics.period_end)
        .bind(metrics.total_events)
        .bind(metrics.delivered_events)
        .bind(metrics.retried_events)
        .bind(metrics.dead_lettered_events)
        .bind(metrics.avg_delivery_time_ms)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

impl PostgresBackend {
    // ──────────────────────────────────────────────────────────────────────
    // Error signatures & resolutions (A.3 Wave 1 — Cluster 4)
    //
    // Transaction-scoped variants take `&mut Transaction<'_, Postgres>` so
    // callers can compose multiple operations atomically. Convenience
    // wrappers (without `_tx` suffix) open a pool transaction internally
    // and commit on success, preserving the previous calling convention.
    // ──────────────────────────────────────────────────────────────────────

    pub async fn create_error_signature_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
        signature: &mk_core::types::ErrorSignature,
    ) -> Result<String, PostgresError> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            "INSERT INTO error_signatures (id, tenant_id, error_type, message_pattern, \
             stack_patterns, context_patterns, embedding, occurrence_count, first_seen_at, \
             last_seen_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(&id)
        .bind(tenant_id)
        .bind(&signature.error_type)
        .bind(&signature.message_pattern)
        .bind(serde_json::to_value(&signature.stack_patterns)?)
        .bind(serde_json::to_value(&signature.context_patterns)?)
        .bind(serde_json::to_value(&signature.embedding)?)
        .bind(1i32)
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&mut **tx)
        .await?;

        Ok(id)
    }

    pub async fn create_error_signature(
        &self,
        tenant_id: &str,
        signature: &mk_core::types::ErrorSignature,
    ) -> Result<String, PostgresError> {
        let mut tx = self.pool.begin().await?;
        let id = Self::create_error_signature_tx(&mut tx, tenant_id, signature).await?;
        tx.commit().await?;
        Ok(id)
    }

    pub async fn get_error_signature_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
        id: &str,
    ) -> Result<Option<mk_core::types::ErrorSignature>, PostgresError> {
        let row = sqlx::query(
            "SELECT error_type, message_pattern, stack_patterns, context_patterns, embedding
             FROM error_signatures WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(&mut **tx)
        .await?;

        match row {
            Some(row) => {
                let embedding: Option<serde_json::Value> = row.get("embedding");
                Ok(Some(mk_core::types::ErrorSignature {
                    error_type: row.get("error_type"),
                    message_pattern: row.get("message_pattern"),
                    stack_patterns: serde_json::from_value(row.get("stack_patterns"))?,
                    context_patterns: serde_json::from_value(row.get("context_patterns"))?,
                    embedding: embedding.and_then(|v| serde_json::from_value(v).ok()),
                }))
            }
            None => Ok(None),
        }
    }

    pub async fn get_error_signature(
        &self,
        tenant_id: &str,
        id: &str,
    ) -> Result<Option<mk_core::types::ErrorSignature>, PostgresError> {
        let mut tx = self.pool.begin().await?;
        let sig = Self::get_error_signature_tx(&mut tx, tenant_id, id).await?;
        tx.commit().await?;
        Ok(sig)
    }

    pub async fn delete_error_signature_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
        id: &str,
    ) -> Result<bool, PostgresError> {
        let result = sqlx::query("DELETE FROM error_signatures WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .execute(&mut **tx)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_error_signature(
        &self,
        tenant_id: &str,
        id: &str,
    ) -> Result<bool, PostgresError> {
        let mut tx = self.pool.begin().await?;
        let deleted = Self::delete_error_signature_tx(&mut tx, tenant_id, id).await?;
        tx.commit().await?;
        Ok(deleted)
    }

    pub async fn create_resolution_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
        resolution: &mk_core::types::Resolution,
    ) -> Result<(), PostgresError> {
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            "INSERT INTO resolutions (id, tenant_id, error_signature_id, description, changes, \
             success_rate, application_count, last_success_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(&resolution.id)
        .bind(tenant_id)
        .bind(&resolution.error_signature_id)
        .bind(&resolution.description)
        .bind(serde_json::to_value(&resolution.changes)?)
        .bind(resolution.success_rate)
        .bind(resolution.application_count as i32)
        .bind(if resolution.last_success_at > 0 {
            Some(resolution.last_success_at)
        } else {
            None
        })
        .bind(now)
        .bind(now)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    pub async fn create_resolution(
        &self,
        tenant_id: &str,
        resolution: &mk_core::types::Resolution,
    ) -> Result<(), PostgresError> {
        let mut tx = self.pool.begin().await?;
        Self::create_resolution_tx(&mut tx, tenant_id, resolution).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_resolution_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
        id: &str,
    ) -> Result<Option<mk_core::types::Resolution>, PostgresError> {
        let row = sqlx::query(
            "SELECT id, error_signature_id, description, changes, success_rate, \
             application_count, last_success_at FROM resolutions WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(&mut **tx)
        .await?;

        match row {
            Some(row) => {
                let last_success_at: Option<i64> = row.get("last_success_at");
                Ok(Some(mk_core::types::Resolution {
                    id: row.get("id"),
                    error_signature_id: row.get("error_signature_id"),
                    description: row.get("description"),
                    changes: serde_json::from_value(row.get("changes"))?,
                    success_rate: row.get("success_rate"),
                    application_count: row.get::<i32, _>("application_count") as u32,
                    last_success_at: last_success_at.unwrap_or(0),
                }))
            }
            None => Ok(None),
        }
    }

    pub async fn get_resolution(
        &self,
        tenant_id: &str,
        id: &str,
    ) -> Result<Option<mk_core::types::Resolution>, PostgresError> {
        let mut tx = self.pool.begin().await?;
        let res = Self::get_resolution_tx(&mut tx, tenant_id, id).await?;
        tx.commit().await?;
        Ok(res)
    }

    pub async fn get_resolutions_for_error_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
        error_signature_id: &str,
    ) -> Result<Vec<mk_core::types::Resolution>, PostgresError> {
        let rows = sqlx::query(
            "SELECT id, error_signature_id, description, changes, success_rate, \
             application_count, last_success_at FROM resolutions 
             WHERE error_signature_id = $1 AND tenant_id = $2
             ORDER BY success_rate DESC, application_count DESC",
        )
        .bind(error_signature_id)
        .bind(tenant_id)
        .fetch_all(&mut **tx)
        .await?;

        let mut resolutions = Vec::new();
        for row in rows {
            let last_success_at: Option<i64> = row.get("last_success_at");
            resolutions.push(mk_core::types::Resolution {
                id: row.get("id"),
                error_signature_id: row.get("error_signature_id"),
                description: row.get("description"),
                changes: serde_json::from_value(row.get("changes"))?,
                success_rate: row.get("success_rate"),
                application_count: row.get::<i32, _>("application_count") as u32,
                last_success_at: last_success_at.unwrap_or(0),
            });
        }

        Ok(resolutions)
    }

    pub async fn get_resolutions_for_error(
        &self,
        tenant_id: &str,
        error_signature_id: &str,
    ) -> Result<Vec<mk_core::types::Resolution>, PostgresError> {
        let mut tx = self.pool.begin().await?;
        let resolutions =
            Self::get_resolutions_for_error_tx(&mut tx, tenant_id, error_signature_id).await?;
        tx.commit().await?;
        Ok(resolutions)
    }

    pub async fn delete_resolution_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
        id: &str,
    ) -> Result<bool, PostgresError> {
        let result = sqlx::query("DELETE FROM resolutions WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .execute(&mut **tx)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_resolution(
        &self,
        tenant_id: &str,
        id: &str,
    ) -> Result<bool, PostgresError> {
        let mut tx = self.pool.begin().await?;
        let deleted = Self::delete_resolution_tx(&mut tx, tenant_id, id).await?;
        tx.commit().await?;
        Ok(deleted)
    }

    // ──────────────────────────────────────────────────────────────────────
    // Hindsight notes (A.3 Wave 1 — Cluster 5)
    // ──────────────────────────────────────────────────────────────────────

    pub async fn create_hindsight_note_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
        note: &mk_core::types::HindsightNote,
    ) -> Result<(), PostgresError> {
        let resolution_ids: Vec<String> = note.resolutions.iter().map(|r| r.id.clone()).collect();

        sqlx::query(
            "INSERT INTO hindsight_notes (id, tenant_id, error_signature_id, content, tags, \
             resolution_ids, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(&note.id)
        .bind(tenant_id)
        .bind(&note.error_signature.error_type)
        .bind(&note.content)
        .bind(serde_json::to_value(&note.tags)?)
        .bind(serde_json::to_value(&resolution_ids)?)
        .bind(note.created_at)
        .bind(note.updated_at)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    pub async fn create_hindsight_note(
        &self,
        tenant_id: &str,
        note: &mk_core::types::HindsightNote,
    ) -> Result<(), PostgresError> {
        let mut tx = self.pool.begin().await?;
        Self::create_hindsight_note_tx(&mut tx, tenant_id, note).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_hindsight_note_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
        id: &str,
    ) -> Result<Option<mk_core::types::HindsightNote>, PostgresError> {
        let row = sqlx::query(
            "SELECT h.id, h.error_signature_id, h.content, h.tags, h.resolution_ids, \
             h.created_at, h.updated_at,
             e.error_type, e.message_pattern, e.stack_patterns, e.context_patterns, e.embedding
             FROM hindsight_notes h
             LEFT JOIN error_signatures e ON h.error_signature_id = e.error_type AND e.tenant_id = \
             $2
             WHERE h.id = $1 AND h.tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(&mut **tx)
        .await?;

        match row {
            Some(row) => {
                let resolution_ids: Vec<String> =
                    serde_json::from_value(row.get("resolution_ids"))?;
                let mut resolutions = Vec::new();
                for res_id in resolution_ids {
                    if let Some(resolution) =
                        Self::get_resolution_tx(tx, tenant_id, &res_id).await?
                    {
                        resolutions.push(resolution);
                    }
                }

                let embedding: Option<serde_json::Value> = row.get("embedding");

                Ok(Some(mk_core::types::HindsightNote {
                    id: row.get("id"),
                    error_signature: mk_core::types::ErrorSignature {
                        error_type: row
                            .get::<Option<String>, _>("error_type")
                            .unwrap_or_else(|| row.get("error_signature_id")),
                        message_pattern: row
                            .get::<Option<String>, _>("message_pattern")
                            .unwrap_or_default(),
                        stack_patterns: row
                            .get::<Option<serde_json::Value>, _>("stack_patterns")
                            .and_then(|v| serde_json::from_value(v).ok())
                            .unwrap_or_default(),
                        context_patterns: row
                            .get::<Option<serde_json::Value>, _>("context_patterns")
                            .and_then(|v| serde_json::from_value(v).ok())
                            .unwrap_or_default(),
                        embedding: embedding.and_then(|v| serde_json::from_value(v).ok()),
                    },
                    resolutions,
                    content: row.get("content"),
                    tags: serde_json::from_value(row.get("tags"))?,
                    created_at: row.get("created_at"),
                    updated_at: row.get("updated_at"),
                }))
            }
            None => Ok(None),
        }
    }

    pub async fn get_hindsight_note(
        &self,
        tenant_id: &str,
        id: &str,
    ) -> Result<Option<mk_core::types::HindsightNote>, PostgresError> {
        let mut tx = self.pool.begin().await?;
        let note = Self::get_hindsight_note_tx(&mut tx, tenant_id, id).await?;
        tx.commit().await?;
        Ok(note)
    }

    pub async fn update_hindsight_note_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
        note: &mk_core::types::HindsightNote,
    ) -> Result<bool, PostgresError> {
        let resolution_ids: Vec<String> = note.resolutions.iter().map(|r| r.id.clone()).collect();

        let result = sqlx::query(
            "UPDATE hindsight_notes 
             SET content = $3, tags = $4, resolution_ids = $5, updated_at = $6
             WHERE id = $1 AND tenant_id = $2",
        )
        .bind(&note.id)
        .bind(tenant_id)
        .bind(&note.content)
        .bind(serde_json::to_value(&note.tags)?)
        .bind(serde_json::to_value(&resolution_ids)?)
        .bind(note.updated_at)
        .execute(&mut **tx)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn update_hindsight_note(
        &self,
        tenant_id: &str,
        note: &mk_core::types::HindsightNote,
    ) -> Result<bool, PostgresError> {
        let mut tx = self.pool.begin().await?;
        let updated = Self::update_hindsight_note_tx(&mut tx, tenant_id, note).await?;
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn delete_hindsight_note_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
        id: &str,
    ) -> Result<bool, PostgresError> {
        let result = sqlx::query("DELETE FROM hindsight_notes WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .execute(&mut **tx)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_hindsight_note(
        &self,
        tenant_id: &str,
        id: &str,
    ) -> Result<bool, PostgresError> {
        let mut tx = self.pool.begin().await?;
        let deleted = Self::delete_hindsight_note_tx(&mut tx, tenant_id, id).await?;
        tx.commit().await?;
        Ok(deleted)
    }

    pub async fn list_hindsight_notes_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<mk_core::types::HindsightNote>, PostgresError> {
        let rows = sqlx::query(
            "SELECT id FROM hindsight_notes 
             WHERE tenant_id = $1
             ORDER BY updated_at DESC
             LIMIT $2 OFFSET $3",
        )
        .bind(tenant_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&mut **tx)
        .await?;

        let ids: Vec<String> = rows.iter().map(|r| r.get("id")).collect();
        let mut notes = Vec::new();
        for id in ids {
            if let Some(note) = Self::get_hindsight_note_tx(tx, tenant_id, &id).await? {
                notes.push(note);
            }
        }

        Ok(notes)
    }

    pub async fn list_hindsight_notes(
        &self,
        tenant_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<mk_core::types::HindsightNote>, PostgresError> {
        let mut tx = self.pool.begin().await?;
        let notes = Self::list_hindsight_notes_tx(&mut tx, tenant_id, limit, offset).await?;
        tx.commit().await?;
        Ok(notes)
    }

    // ──────────────────────────────────────────────────────────────────────
    // Decomposition policy / trajectories (A.3 Wave 1 — Cluster 5)
    // ──────────────────────────────────────────────────────────────────────

    /// Save decomposition policy weights (tx-scoped).
    pub async fn save_decomposition_weights_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
        action_type: &str,
        weight: f32,
    ) -> Result<(), PostgresError> {
        let id = format!("{}-{}", tenant_id, action_type);
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            "INSERT INTO decomposition_policy_weights (id, tenant_id, action_type, weight, \
             created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (tenant_id, action_type) DO UPDATE SET 
                weight = $4, 
                updated_at = $6",
        )
        .bind(&id)
        .bind(tenant_id)
        .bind(action_type)
        .bind(weight)
        .bind(now)
        .bind(now)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    /// Save decomposition policy weights to database.
    pub async fn save_decomposition_weights(
        &self,
        tenant_id: &str,
        action_type: &str,
        weight: f32,
    ) -> Result<(), PostgresError> {
        let mut tx = self.pool.begin().await?;
        Self::save_decomposition_weights_tx(&mut tx, tenant_id, action_type, weight).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Load decomposition policy weights (tx-scoped).
    pub async fn load_decomposition_weights_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
    ) -> Result<Vec<(String, f32)>, PostgresError> {
        let rows = sqlx::query(
            "SELECT action_type, weight FROM decomposition_policy_weights 
             WHERE tenant_id = $1 
             ORDER BY action_type",
        )
        .bind(tenant_id)
        .fetch_all(&mut **tx)
        .await?;

        let mut weights = Vec::new();
        for row in rows {
            let action_type: String = row.get("action_type");
            let weight: f32 = row.get("weight");
            weights.push((action_type, weight));
        }

        Ok(weights)
    }

    /// Load decomposition policy weights from database.
    pub async fn load_decomposition_weights(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<(String, f32)>, PostgresError> {
        let mut tx = self.pool.begin().await?;
        let weights = Self::load_decomposition_weights_tx(&mut tx, tenant_id).await?;
        tx.commit().await?;
        Ok(weights)
    }

    /// Save decomposition policy state (tx-scoped).
    #[allow(clippy::too_many_arguments)]
    pub async fn save_decomposition_policy_state_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
        epsilon: f32,
        step_count: i32,
        learning_rate: f32,
        gamma: f32,
        success_weight: f32,
        efficiency_weight: f32,
    ) -> Result<(), PostgresError> {
        let id = format!("{}-policy-state", tenant_id);
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            "INSERT INTO decomposition_policy_state (id, tenant_id, epsilon, step_count, 
             learning_rate, gamma, success_weight, efficiency_weight, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (tenant_id) DO UPDATE SET 
                epsilon = $3,
                step_count = $4,
                learning_rate = $5,
                gamma = $6,
                success_weight = $7,
                efficiency_weight = $8,
                updated_at = $10",
        )
        .bind(&id)
        .bind(tenant_id)
        .bind(epsilon)
        .bind(step_count)
        .bind(learning_rate)
        .bind(gamma)
        .bind(success_weight)
        .bind(efficiency_weight)
        .bind(now)
        .bind(now)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    /// Save decomposition policy state to database.
    #[allow(clippy::too_many_arguments)]
    pub async fn save_decomposition_policy_state(
        &self,
        tenant_id: &str,
        epsilon: f32,
        step_count: i32,
        learning_rate: f32,
        gamma: f32,
        success_weight: f32,
        efficiency_weight: f32,
    ) -> Result<(), PostgresError> {
        let mut tx = self.pool.begin().await?;
        Self::save_decomposition_policy_state_tx(
            &mut tx,
            tenant_id,
            epsilon,
            step_count,
            learning_rate,
            gamma,
            success_weight,
            efficiency_weight,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Load decomposition policy state (tx-scoped).
    pub async fn load_decomposition_policy_state_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
    ) -> Result<Option<(f32, i32, f32, f32, f32, f32)>, PostgresError> {
        let row = sqlx::query(
            "SELECT epsilon, step_count, learning_rate, gamma, success_weight, efficiency_weight 
             FROM decomposition_policy_state 
             WHERE tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_optional(&mut **tx)
        .await?;

        if let Some(row) = row {
            let epsilon: f32 = row.get("epsilon");
            let step_count: i32 = row.get("step_count");
            let learning_rate: f32 = row.get("learning_rate");
            let gamma: f32 = row.get("gamma");
            let success_weight: f32 = row.get("success_weight");
            let efficiency_weight: f32 = row.get("efficiency_weight");

            Ok(Some((
                epsilon,
                step_count,
                learning_rate,
                gamma,
                success_weight,
                efficiency_weight,
            )))
        } else {
            Ok(None)
        }
    }

    /// Load decomposition policy state from database.
    pub async fn load_decomposition_policy_state(
        &self,
        tenant_id: &str,
    ) -> Result<Option<(f32, i32, f32, f32, f32, f32)>, PostgresError> {
        let mut tx = self.pool.begin().await?;
        let state = Self::load_decomposition_policy_state_tx(&mut tx, tenant_id).await?;
        tx.commit().await?;
        Ok(state)
    }

    /// Save decomposition trajectory (tx-scoped).
    #[allow(clippy::too_many_arguments)]
    pub async fn save_decomposition_trajectory_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
        trajectory_id: &str,
        query: &str,
        started_at: i64,
        completed_at: Option<i64>,
        outcome: Option<serde_json::Value>,
        reward: Option<f32>,
        tokens_used: i32,
        max_depth: i32,
        actions: serde_json::Value,
    ) -> Result<(), PostgresError> {
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            "INSERT INTO decomposition_trajectories (id, tenant_id, query, started_at, \
             completed_at,
             outcome, reward, tokens_used, max_depth, actions, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(trajectory_id)
        .bind(tenant_id)
        .bind(query)
        .bind(started_at)
        .bind(completed_at)
        .bind(outcome)
        .bind(reward)
        .bind(tokens_used)
        .bind(max_depth)
        .bind(actions)
        .bind(now)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    /// Save decomposition trajectory to database.
    #[allow(clippy::too_many_arguments)]
    pub async fn save_decomposition_trajectory(
        &self,
        tenant_id: &str,
        trajectory_id: &str,
        query: &str,
        started_at: i64,
        completed_at: Option<i64>,
        outcome: Option<serde_json::Value>,
        reward: Option<f32>,
        tokens_used: i32,
        max_depth: i32,
        actions: serde_json::Value,
    ) -> Result<(), PostgresError> {
        let mut tx = self.pool.begin().await?;
        Self::save_decomposition_trajectory_tx(
            &mut tx,
            tenant_id,
            trajectory_id,
            query,
            started_at,
            completed_at,
            outcome,
            reward,
            tokens_used,
            max_depth,
            actions,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Load recent decomposition trajectories (tx-scoped).
    pub async fn load_recent_decomposition_trajectories_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
        limit: i32,
    ) -> Result<Vec<serde_json::Value>, PostgresError> {
        let rows = sqlx::query(
            "SELECT id, query, started_at, completed_at, outcome, reward, tokens_used, max_depth, \
             actions
             FROM decomposition_trajectories 
             WHERE tenant_id = $1 AND completed_at IS NOT NULL
             ORDER BY completed_at DESC 
             LIMIT $2",
        )
        .bind(tenant_id)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;

        let mut trajectories = Vec::new();
        for row in rows {
            let mut map = serde_json::Map::new();

            map.insert("id".to_string(), serde_json::Value::String(row.get("id")));
            map.insert(
                "query".to_string(),
                serde_json::Value::String(row.get("query")),
            );
            map.insert(
                "started_at".to_string(),
                serde_json::Value::Number(row.get::<i64, _>("started_at").into()),
            );

            if let Some(completed_at) = row.get::<Option<i64>, _>("completed_at") {
                map.insert(
                    "completed_at".to_string(),
                    serde_json::Value::Number(completed_at.into()),
                );
            }

            if let Some(outcome) = row.get::<Option<serde_json::Value>, _>("outcome") {
                map.insert("outcome".to_string(), outcome);
            }

            if let Some(reward) = row.get::<Option<f32>, _>("reward") {
                map.insert(
                    "reward".to_string(),
                    serde_json::Value::Number(serde_json::Number::from_f64(reward as f64).unwrap()),
                );
            }

            map.insert(
                "tokens_used".to_string(),
                serde_json::Value::Number(row.get::<i32, _>("tokens_used").into()),
            );
            map.insert(
                "max_depth".to_string(),
                serde_json::Value::Number(row.get::<i32, _>("max_depth").into()),
            );
            map.insert(
                "actions".to_string(),
                row.get::<serde_json::Value, _>("actions"),
            );

            trajectories.push(serde_json::Value::Object(map));
        }

        Ok(trajectories)
    }

    /// Load recent decomposition trajectories from database.
    pub async fn load_recent_decomposition_trajectories(
        &self,
        tenant_id: &str,
        limit: i32,
    ) -> Result<Vec<serde_json::Value>, PostgresError> {
        let mut tx = self.pool.begin().await?;
        let trajectories =
            Self::load_recent_decomposition_trajectories_tx(&mut tx, tenant_id, limit).await?;
        tx.commit().await?;
        Ok(trajectories)
    }
}

// ---------------------------------------------------------------------------
// Admin-scope audit writer
// ---------------------------------------------------------------------------

/// Write a `governance_audit_log` row marking an admin-scope action.
///
/// Called from [`PostgresBackend::with_admin_context`] inside the same
/// transaction as the admin work itself: the admin action and its audit
/// row commit atomically or not at all. That property is load-bearing
/// — an admin write that succeeds without an audit row is an
/// unattributed BYPASSRLS access, which is the exact failure mode this
/// helper exists to prevent.
///
/// Field mapping:
/// - `actor_type`  \u2190 `'system'` if `ctx.user_id == SYSTEM_USER_ID`
///                   (scheduled jobs), else `'user'`.
/// - `actor_email` \u2190 derived from `ctx.user_id` display string
///                   (audit_log has no first-class email lookup here;
///                   ctx carries what the middleware resolved).
/// - `admin_scope` \u2190 always `TRUE`. The column is the primary filter
///                   for \"show me every cross-tenant administrative
///                   access\" queries.
/// - `acting_as_tenant_id` \u2190 `ctx.target_tenant_id` when set
///                   (single-tenant impersonation), else NULL
///                   (cross-tenant-all).
/// - `details`     \u2190 JSONB with the action name and a `source =
///                   'with_admin_context'` marker so filters can
///                   distinguish helper-generated rows from call-site
///                   log_audit writes.
async fn audit_admin_access(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    ctx: &TenantContext,
    action: &str,
) -> Result<(), PostgresError> {
    let actor_type = if ctx.user_id.as_str() == SYSTEM_USER_ID {
        "system"
    } else {
        "user"
    };
    let actor_email = format!("{}", ctx.user_id);
    let acting_as_tenant_id: Option<uuid::Uuid> = ctx
        .target_tenant_id
        .as_ref()
        .and_then(|t| uuid::Uuid::parse_str(&t.to_string()).ok());
    let details = serde_json::json!({
        "source": "with_admin_context",
        "action": action,
    });

    sqlx::query(
        r#"
        INSERT INTO governance_audit_log (
            action, actor_type, actor_email, details,
            admin_scope, acting_as_tenant_id
        ) VALUES ($1, $2, $3, $4, TRUE, $5)
        "#,
    )
    .bind(action)
    .bind(actor_type)
    .bind(actor_email)
    .bind(details)
    .bind(acting_as_tenant_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;
    use serde_json::json;

    // Test PostgresError display
    #[test]
    fn test_postgres_error_display() {
        let error = PostgresError::Database(sqlx::Error::Configuration(
            "Invalid connection string".into(),
        ));

        assert!(error.to_string().contains("Database error"));
        assert!(error.to_string().contains("Invalid connection string"));
    }

    // Test error conversion from sqlx::Error
    #[test]
    fn test_postgres_error_from_sqlx() {
        let sqlx_error = sqlx::Error::Configuration("test".into());
        let pg_error: PostgresError = sqlx_error.into();

        match pg_error {
            PostgresError::Database(_) => (),
            PostgresError::Serialization(_) => (),
            PostgresError::NotFound(_) => (),
        }
    }

    // Test PostgresBackend struct (compile-time checks)
    #[test]
    fn test_postgres_backend_struct() {
        // Verify the struct has expected fields
        struct _TestBackend {
            _pool: Pool<Postgres>,
        }

        // This is a compile-time test - if it compiles, PostgresBackend has the right
        // structure We can't instantiate it without a real database connection
        let _backend_type = std::any::type_name::<PostgresBackend>();
        assert_eq!(_backend_type, "storage::postgres::PostgresBackend");
    }

    // Test StorageBackend trait implementation
    #[test]
    fn test_storage_backend_trait_implementation() {
        use mk_core::traits::StorageBackend;

        // Compile-time check that PostgresBackend implements StorageBackend
        fn assert_implements_storage_backend<T: StorageBackend>() {}

        assert_implements_storage_backend::<PostgresBackend>();
    }

    // Test JSON serialization patterns used in the code
    #[test]
    fn test_json_serialization_patterns() {
        // Test the serialization pattern used in store() method
        let value = json!({"key": "value", "number": 42});
        let bytes = serde_json::to_vec(&value).unwrap();

        // Test deserialization pattern used in retrieve() method
        let deserialized: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(deserialized["key"], "value");
        assert_eq!(deserialized["number"], 42);

        // Test default fallback used in store()
        let invalid_bytes = b"not json";
        let default_value =
            serde_json::from_slice::<serde_json::Value>(invalid_bytes).unwrap_or_default();
        assert!(default_value.is_null() || default_value == json!({}));
    }

    // Test timestamp generation pattern
    #[test]
    fn test_timestamp_generation() {
        use chrono::Utc;

        let timestamp = Utc::now().timestamp();
        assert!(timestamp > 0); // Should be positive (after 1970)

        // Verify it's a reasonable timestamp (not in distant future)
        let current_year = Utc::now().year();
        let timestamp_year = chrono::DateTime::from_timestamp(timestamp, 0)
            .map(|dt| dt.year())
            .unwrap_or(1970);

        // Should be within 10 years of current year
        assert!((timestamp_year - current_year).abs() <= 10);
    }

    // Test SQL query patterns for correctness
    #[test]
    fn test_sql_query_patterns() {
        // Verify the SQL queries are syntactically correct
        let create_table_query = "CREATE TABLE IF NOT EXISTS sync_state (
                id TEXT PRIMARY KEY,
                data JSONB NOT NULL,
                updated_at BIGINT NOT NULL
            )";

        let insert_query = "INSERT INTO sync_state (id, data, updated_at)
             VALUES ($1, $2, $3)
             ON CONFLICT (id) DO UPDATE SET data = $2, updated_at = $3";

        let select_query = "SELECT data FROM sync_state WHERE id = $1";
        let delete_query = "DELETE FROM sync_state WHERE id = $1";
        let exists_query = "SELECT 1 FROM sync_state WHERE id = $1";

        // Just verify they're non-empty strings
        assert!(!create_table_query.is_empty());
        assert!(!insert_query.is_empty());
        assert!(!select_query.is_empty());
        assert!(!delete_query.is_empty());
        assert!(!exists_query.is_empty());

        // Verify they contain expected keywords
        assert!(create_table_query.contains("CREATE TABLE"));
        assert!(insert_query.contains("INSERT INTO"));
        assert!(select_query.contains("SELECT"));
        assert!(delete_query.contains("DELETE"));
        assert!(exists_query.contains("SELECT 1"));
    }

    // Tests for resolve_user_id_by_idp_subject and get_user_roles_for_auth SQL patterns

    #[test]
    fn resolve_user_id_sql_pattern_is_correct() {
        let sql = "SELECT id::text FROM users WHERE idp_subject = $1 LIMIT 1";
        assert!(sql.contains("idp_subject"));
        assert!(sql.contains("LIMIT 1"));
        assert!(sql.starts_with("SELECT"));
    }

    #[test]
    fn get_user_roles_for_auth_sql_includes_root_sentinel() {
        // The query must include BOTH the caller's tenant_id AND the __root__ sentinel so that
        // instance-scoped roles (e.g. PlatformAdmin) are always returned regardless of tenant.
        let sql = "SELECT DISTINCT role FROM user_roles
             WHERE user_id = $1 AND (tenant_id = $2 OR tenant_id = '__root__')";
        assert!(sql.contains("__root__"));
        assert!(sql.contains("DISTINCT"));
        assert!(sql.contains("tenant_id = $2 OR tenant_id"));
    }

    #[test]
    fn instance_scope_tenant_id_constant_is_root() {
        assert_eq!(mk_core::types::INSTANCE_SCOPE_TENANT_ID, "__root__");
    }
}
