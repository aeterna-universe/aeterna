//! Embedded SQL migrations for the aeterna Postgres schema.
//!
//! This module is the single source of truth for:
//! - The list of migration files in `storage/migrations/` (embedded at build
//!   time via [`include_str!`]).
//! - The order in which they must be applied.
//!
//! Callers:
//! - `aeterna admin migrate` (prod / init container) — uses
//!   [`MIGRATIONS`] + its own checksum-verifying runner in
//!   `cli::commands::admin` for durable tracking via the
//!   `_aeterna_migrations` table.
//! - Test fixtures (`testing::postgres_with_migrations`) — use
//!   [`apply_all`] to bring a fresh testcontainer DB up to the same
//!   schema version production runs on, including tables only defined
//!   in migrations (e.g. `users`, `organizations`, `teams`, `agents`).
//!
//! The schema is produced by running, in order:
//!   1. `PostgresBackend::initialize_schema()` — inline `CREATE TABLE`s
//!      for the "core" tables operated on by `PostgresBackend` directly
//!      (including several tables only defined inline, such as
//!      `organizational_units`, `graph_nodes`, `sync_state`).
//!   2. [`apply_all`] (this module) — the 20 migrations that add the
//!      rest of the schema (`users`, referential integrity tables,
//!      governance workflow, codesearch, etc.) and run `ALTER TABLE`
//!      statements to extend the inline-created tables with additional
//!      columns.
//!
//! This mirrors exactly what happens in production: the app's init
//! container runs `aeterna admin migrate` (calling [`apply_all`]
//! transitively via the admin runner), and `PostgresBackend::new()`
//! calls `initialize_schema()` during normal startup.

use sqlx::PgPool;

/// A migration file embedded at compile time.
#[derive(Debug, Clone, Copy)]
pub struct EmbeddedMigration {
    pub version: i32,
    pub name: &'static str,
    pub sql: &'static str,
}

/// All embedded migrations, ordered by version.
///
/// Versions start at 3 for historical reasons (versions 1 and 2 were
/// consolidated into `PostgresBackend::initialize_schema()` before the
/// migration runner existed).
pub const MIGRATIONS: &[EmbeddedMigration] = &[
    EmbeddedMigration {
        version: 3,
        name: "003_create_memory_tables",
        sql: include_str!("../migrations/003_create_memory_tables.sql"),
    },
    EmbeddedMigration {
        version: 4,
        name: "004_enable_rls",
        sql: include_str!("../migrations/004_enable_rls.sql"),
    },
    EmbeddedMigration {
        version: 5,
        name: "005_drift_tuning",
        sql: include_str!("../migrations/005_drift_tuning.sql"),
    },
    EmbeddedMigration {
        version: 6,
        name: "006_event_streaming",
        sql: include_str!("../migrations/006_event_streaming.sql"),
    },
    EmbeddedMigration {
        version: 7,
        name: "007_cca_summaries",
        sql: include_str!("../migrations/007_cca_summaries.sql"),
    },
    EmbeddedMigration {
        version: 8,
        name: "008_hindsight_tables",
        sql: include_str!("../migrations/008_hindsight_tables.sql"),
    },
    EmbeddedMigration {
        version: 9,
        name: "009_organizational_referential",
        sql: include_str!("../migrations/009_organizational_referential.sql"),
    },
    EmbeddedMigration {
        version: 10,
        name: "010_governance_workflow",
        sql: include_str!("../migrations/010_governance_workflow.sql"),
    },
    EmbeddedMigration {
        version: 11,
        name: "011_meta_governance",
        sql: include_str!("../migrations/011_meta_governance.sql"),
    },
    EmbeddedMigration {
        version: 12,
        name: "012_decomposition_weights",
        sql: include_str!("../migrations/012_decomposition_weights.sql"),
    },
    EmbeddedMigration {
        version: 13,
        name: "013_referential_integrity",
        sql: include_str!("../migrations/013_referential_integrity.sql"),
    },
    EmbeddedMigration {
        version: 14,
        name: "014_grepai_repo_management",
        sql: include_str!("../migrations/014_grepai_repo_management.sql"),
    },
    EmbeddedMigration {
        version: 15,
        name: "015_add_device_id_to_memory",
        sql: include_str!("../migrations/015_add_device_id_to_memory.sql"),
    },
    EmbeddedMigration {
        version: 16,
        name: "016_governance_rls",
        sql: include_str!("../migrations/016_governance_rls.sql"),
    },
    EmbeddedMigration {
        version: 17,
        name: "017_tenants_tables",
        sql: include_str!("../migrations/017_tenants_tables.sql"),
    },
    EmbeddedMigration {
        version: 18,
        name: "018_add_last_accessed_at",
        sql: include_str!("../migrations/018_add_last_accessed_at.sql"),
    },
    EmbeddedMigration {
        version: 19,
        name: "019_day2_operations_tables",
        sql: include_str!("../migrations/019_day2_operations_tables.sql"),
    },
    EmbeddedMigration {
        version: 20,
        name: "020_fix_codesearch_views",
        sql: include_str!("../migrations/020_fix_codesearch_views.sql"),
    },
    EmbeddedMigration {
        version: 21,
        name: "021_add_user_idp_columns",
        sql: include_str!("../migrations/021_add_user_idp_columns.sql"),
    },
    EmbeddedMigration {
        version: 22,
        name: "022_drop_dead_vector_columns",
        sql: include_str!("../migrations/022_drop_dead_vector_columns.sql"),
    },
    EmbeddedMigration {
        version: 23,
        name: "023_platform_admin_impersonation",
        sql: include_str!("../migrations/023_platform_admin_impersonation.sql"),
    },
    EmbeddedMigration {
        version: 24,
        name: "024_normalize_rls_session_variables",
        sql: include_str!("../migrations/024_normalize_rls_session_variables.sql"),
    },
    EmbeddedMigration {
        version: 25,
        name: "025_add_app_roles",
        sql: include_str!("../migrations/025_add_app_roles.sql"),
    },
    EmbeddedMigration {
        version: 26,
        name: "026_tenant_secrets",
        sql: include_str!("../migrations/026_tenant_secrets.sql"),
    },
    EmbeddedMigration {
        version: 27,
        name: "027_tenant_manifest_state",
        sql: include_str!("../migrations/027_tenant_manifest_state.sql"),
    },
    EmbeddedMigration {
        version: 28,
        name: "028_tenant_scoped_hierarchy",
        sql: include_str!("../migrations/028_tenant_scoped_hierarchy.sql"),
    },
    EmbeddedMigration {
        version: 29,
        name: "029_agents_tenant_scope",
        sql: include_str!("../migrations/029_agents_tenant_scope.sql"),
    },
    EmbeddedMigration {
        version: 30,
        name: "030_audit_context",
        sql: include_str!("../migrations/030_audit_context.sql"),
    },
    EmbeddedMigration {
        version: 31,
        name: "031_provisioning_idempotency",
        sql: include_str!("../migrations/031_provisioning_idempotency.sql"),
    },
    EmbeddedMigration {
        version: 32,
        name: "032_tenant_delete_cascade",
        sql: include_str!("../migrations/032_tenant_delete_cascade.sql"),
    },
    EmbeddedMigration {
        version: 33,
        name: "033_tenant_legal_entity",
        sql: include_str!("../migrations/033_tenant_legal_entity.sql"),
    },
    EmbeddedMigration {
        version: 34,
        name: "034_graph_events",
        sql: include_str!("../migrations/034_graph_events.sql"),
    },
    EmbeddedMigration {
        version: 35,
        name: "035_organizational_units_timestamps",
        sql: include_str!("../migrations/035_organizational_units_timestamps.sql"),
    },
    EmbeddedMigration {
        version: 36,
        name: "036_accounts",
        sql: include_str!("../migrations/036_accounts.sql"),
    },
    EmbeddedMigration {
        version: 37,
        name: "037_organizations_tenant_root",
        sql: include_str!("../migrations/037_organizations_tenant_root.sql"),
    },
    EmbeddedMigration {
        version: 38,
        name: "038_tenant_root_views",
        sql: include_str!("../migrations/038_tenant_root_views.sql"),
    },
    EmbeddedMigration {
        version: 39,
        name: "039_drop_company_root",
        sql: include_str!("../migrations/039_drop_company_root.sql"),
    },
    EmbeddedMigration {
        version: 40,
        name: "040_finalize_tenant_schema",
        sql: include_str!("../migrations/040_finalize_tenant_schema.sql"),
    },
];

/// Apply every embedded migration in order, transactionally.
///
/// Intended for test setup (fresh PostgreSQL container). Every migration
/// file is idempotent (`CREATE TABLE IF NOT EXISTS`, `CREATE INDEX IF
/// NOT EXISTS`, `ADD COLUMN IF NOT EXISTS`, …), so repeated invocation
/// on the same database is safe — but the production code path
/// (`aeterna admin migrate`) uses the `_aeterna_migrations` tracking
/// table to skip already-applied migrations and verify checksums. Tests
/// don't need that machinery since they always start from a fresh DB.
pub async fn apply_all(pool: &PgPool) -> Result<(), sqlx::Error> {
    for migration in MIGRATIONS {
        let mut tx = pool.begin().await?;
        // `raw_sql` is required here because migration files contain
        // multiple statements; `sqlx::query()` sends a prepared statement
        // which allows only one command per call.
        sqlx::raw_sql(migration.sql).execute(&mut *tx).await?;
        tx.commit().await?;
    }
    Ok(())
}
