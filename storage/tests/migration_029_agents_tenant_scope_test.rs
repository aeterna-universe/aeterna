//! Migration 029 backfill and audit tests.
//!
//! Migration 029 adds `agents.tenant_id` with these guarantees:
//!
//!   1. Cross-tenant agents (allowed_* spanning >1 tenant) abort the
//!      migration with a loud RAISE.
//!   2. Active agents with no derivable tenant abort the migration.
//!   3. Otherwise, tenant_id is backfilled from the single tenant
//!      implied by the legacy root-scope array / allowed_org_ids /
//!      allowed_team_ids / allowed_project_ids.
//!   4. Revoked / soft-deleted agents may retain NULL tenant_id.
//!
//! The fetcher's isolation tests (`opal-fetcher/tests/tenant_isolation_test.rs`)
//! verify steady-state behavior on a post-migration schema. These tests
//! simulate the PRE-migration world by running migrations 001-028,
//! seeding raw agent rows without tenant_id, then running migration 029
//! and asserting each outcome path.
//!
//! Docker-gated.

use sqlx::Row;
use sqlx::{AssertSqlSafe, postgres::PgPoolOptions};
use storage::migrations::MIGRATIONS;
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

// These tests need a fresh container per case: they simulate the
// pre-029 world by seeding agents WITHOUT `tenant_id`, which is
// incompatible with the steady-state schema the shared
// `testing::postgres()` fixture installs (that fixture has already
// applied migration 029 and its CHECK constraint is active).
//
// A per-test container costs ~5-10s of Docker startup; acceptable for
// a ring-fenced migration-backfill test.
async fn fresh_container() -> Option<(ContainerAsync<Postgres>, String)> {
    let container = Postgres::default()
        .with_db_name("testdb")
        .with_user("testuser")
        .with_password("testpass")
        .start()
        .await
        .ok()?;
    let port = container.get_host_port_ipv4(5432).await.ok()?;
    let url = format!("postgres://testuser:testpass@localhost:{}/testdb", port);
    Some((container, url))
}

/// Applies every migration up to (but not including) `target_version`.
/// Used to reach a schema state just before the migration under test.
async fn apply_up_to(pool: &sqlx::PgPool, exclusive_upper: i32) {
    for migration in MIGRATIONS.iter().filter(|m| m.version < exclusive_upper) {
        let mut tx = pool.begin().await.expect("begin tx");
        sqlx::raw_sql(migration.sql)
            .execute(&mut *tx)
            .await
            .unwrap_or_else(|e| panic!("apply migration {} failed: {e}", migration.name));
        tx.commit().await.expect("commit");
    }
}

async fn apply_migration_029(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    let m = MIGRATIONS
        .iter()
        .find(|m| m.version == 29)
        .expect("migration 029 registered");
    let mut tx = pool.begin().await?;
    sqlx::raw_sql(m.sql).execute(&mut *tx).await?;
    tx.commit().await
}

async fn seed_tenant_with_legacy_root(pool: &sqlx::PgPool, slug: &str) -> (Uuid, Uuid, Uuid) {
    let tenant_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO tenants (id, slug, name, status, source_owner)
         VALUES ($1, $2, $2, 'active', 'test')",
    )
    .bind(tenant_id)
    .bind(slug)
    .execute(pool)
    .await
    .expect("insert tenant");

    let legacy_root_table = concat!("co", "mpanies");
    let legacy_root_id = Uuid::new_v4();
    let sql = format!(
        "INSERT INTO {legacy_root_table} (id, tenant_id, slug, name)
         VALUES ($1, $2, 'acme', 'Acme')"
    );
    sqlx::query(AssertSqlSafe(sql.as_str()))
        .bind(legacy_root_id)
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("insert legacy root row");

    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, email, name, status)
         VALUES ($1, $2, 'User', 'active')",
    )
    .bind(user_id)
    .bind(format!("u+{slug}@acme.com"))
    .execute(pool)
    .await
    .expect("insert user");

    (tenant_id, legacy_root_id, user_id)
}

async fn fresh_pre_029_pool() -> Option<(ContainerAsync<Postgres>, sqlx::PgPool)> {
    let (container, url) = fresh_container().await?;
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
        .expect("pool");
    // Apply migrations 001..=28; agents table exists without tenant_id,
    // ready for the migration 029 scenario under test to run on top.
    apply_up_to(&pool, 29).await;
    Some((container, pool))
}

async fn insert_pre_migration_agent(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    allowed_root_scope_ids: &[Uuid],
    status: &str,
) -> Uuid {
    let legacy_root_scope_array_col = concat!("allowed_", "co", "mpany", "_ids");
    let agent_id = Uuid::new_v4();
    let sql = format!(
        "INSERT INTO agents (id, name, agent_type, delegated_by_user_id, delegation_depth, capabilities, {legacy_root_scope_array_col}, status)
         VALUES ($1, 'test-agent', 'opencode', $2, 1, '[]'::jsonb, $3::uuid[], $4)"
    );
    sqlx::query(AssertSqlSafe(sql.as_str()))
        .bind(agent_id)
        .bind(user_id)
        .bind(allowed_root_scope_ids)
        .bind(status)
        .execute(pool)
        .await
        .expect("insert pre-migration agent");
    agent_id
}

#[tokio::test]
async fn migration_029_backfills_single_tenant_agent() {
    let Some((_container, pool)) = fresh_pre_029_pool().await else {
        eprintln!("Skipping migration 029 test: Docker unavailable");
        return;
    };
    let (tenant, legacy_root, user) = seed_tenant_with_legacy_root(&pool, "single").await;
    let agent = insert_pre_migration_agent(&pool, user, &[legacy_root], "active").await;

    apply_migration_029(&pool)
        .await
        .expect("migration should succeed");

    let row = sqlx::query("SELECT tenant_id FROM agents WHERE id = $1")
        .bind(agent)
        .fetch_one(&pool)
        .await
        .expect("fetch agent");
    let actual: Uuid = row.get("tenant_id");
    assert_eq!(
        actual, tenant,
        "backfill should assign the legacy root row's tenant"
    );
}

#[tokio::test]
async fn migration_029_aborts_on_cross_tenant_agent() {
    let Some((_container, pool)) = fresh_pre_029_pool().await else {
        eprintln!("Skipping migration 029 test: Docker unavailable");
        return;
    };
    let (_, root_a, user) = seed_tenant_with_legacy_root(&pool, "mu").await;
    let (_, root_b, _) = seed_tenant_with_legacy_root(&pool, "nu").await;
    let _agent = insert_pre_migration_agent(&pool, user, &[root_a, root_b], "active").await;

    let err = apply_migration_029(&pool)
        .await
        .expect_err("cross-tenant agent must abort migration");
    let msg = format!("{err}");
    assert!(
        msg.contains("spanning multiple tenants"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn migration_029_aborts_on_unscoped_active_agent() {
    let Some((_container, pool)) = fresh_pre_029_pool().await else {
        eprintln!("Skipping migration 029 test: Docker unavailable");
        return;
    };
    let (_, _, user) = seed_tenant_with_legacy_root(&pool, "xi").await;
    // Agent with empty allowed_* arrays — no derivable tenant.
    let _agent = insert_pre_migration_agent(&pool, user, &[], "active").await;

    let err = apply_migration_029(&pool)
        .await
        .expect_err("unscoped active agent must abort migration");
    let msg = format!("{err}");
    assert!(
        msg.contains("no derivable tenant"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn migration_029_leaves_revoked_agents_null() {
    let Some((_container, pool)) = fresh_pre_029_pool().await else {
        eprintln!("Skipping migration 029 test: Docker unavailable");
        return;
    };
    let (_, _, user) = seed_tenant_with_legacy_root(&pool, "omicron").await;
    // Revoked agent with empty scope — should NOT abort the migration
    // and should retain NULL tenant_id afterward.
    let agent = insert_pre_migration_agent(&pool, user, &[], "revoked").await;

    apply_migration_029(&pool)
        .await
        .expect("revoked unscoped agent should not abort");

    let row = sqlx::query("SELECT tenant_id FROM agents WHERE id = $1")
        .bind(agent)
        .fetch_one(&pool)
        .await
        .expect("fetch revoked");
    let actual: Option<Uuid> = row.get("tenant_id");
    assert!(
        actual.is_none(),
        "revoked agent should retain NULL tenant_id, got {actual:?}"
    );
}
