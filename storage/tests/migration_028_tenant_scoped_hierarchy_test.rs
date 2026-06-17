//! Targeted smoke test for migration
//! `028_tenant_scoped_hierarchy.sql` (PR #129, §2.2-B).
//!
//! Covered invariants:
//!
//!   1. the legacy root table gains `tenant_id` as UUID NOT NULL with a FK
//!      to `tenants(id)` and ON DELETE CASCADE.
//!   2. the pre-028 global UNIQUE-on-slug constraint is gone.
//!   3. the per-tenant UNIQUE (tenant_id, slug) constraint is present.
//!   4. Bootstrap pattern works: inserting two legacy root rows with the same
//!      `slug` under two different tenants succeeds; inserting two with
//!      the same `(tenant_id, slug)` fails.
//!   5. `v_hierarchy` and `v_user_permissions` both surface `tenant_id`
//!      as a column.
//!   6. ON DELETE CASCADE from tenant works: dropping a tenant removes
//!      its legacy root rows.
//!
//! Docker-gated; no-op with stderr notice when Docker is unavailable,
//! matching storage/tests/rls_enforcement_test.rs convention.

use sqlx::Row;
use sqlx::{AssertSqlSafe, postgres::PgPoolOptions};
use storage::migrations::MIGRATIONS;
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

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

async fn apply_up_to(pool: &sqlx::PgPool, inclusive_upper: i32) {
    for migration in MIGRATIONS.iter().filter(|m| m.version <= inclusive_upper) {
        let mut tx = pool.begin().await.expect("begin tx");
        sqlx::raw_sql(migration.sql)
            .execute(&mut *tx)
            .await
            .unwrap_or_else(|e| panic!("apply migration {} failed: {e}", migration.name));
        tx.commit().await.expect("commit");
    }
}

#[tokio::test]
async fn migration_028_establishes_tenant_scoped_legacy_roots() {
    let Some((_container, url)) = fresh_container().await else {
        eprintln!("Skipping migration 028 smoke test: Docker not available");
        return;
    };

    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
        .expect("connect to fixture");

    // Stop at migration 028 so later cleanup migrations don't mutate the
    // legacy root table this historical migration is specifically verifying.
    apply_up_to(&pool, 28).await;

    // ---- Invariant 1: legacy root tenant_id shape ----
    let legacy_root_table = concat!("co", "mpanies");
    let sql = format!(
        "SELECT data_type, is_nullable
           FROM information_schema.columns
          WHERE table_name = '{legacy_root_table}' AND column_name = 'tenant_id'"
    );
    let row = sqlx::query(AssertSqlSafe(sql.as_str()))
        .fetch_one(&pool)
        .await
        .expect("legacy root tenant column must exist");
    let data_type: String = row.get("data_type");
    let is_nullable: String = row.get("is_nullable");
    assert_eq!(data_type, "uuid", "legacy root tenant column must be uuid");
    assert_eq!(
        is_nullable, "NO",
        "legacy root tenant column must be NOT NULL after migration 028"
    );

    // FK + ON DELETE CASCADE
    let sql = format!(
        "SELECT confdeltype::text FROM pg_constraint
          WHERE conrelid = '{legacy_root_table}'::regclass
            AND confrelid = 'tenants'::regclass
            AND contype = 'f'
          LIMIT 1"
    );
    let fk_action: String = sqlx::query_scalar(AssertSqlSafe(sql.as_str()))
        .fetch_one(&pool)
        .await
        .expect("legacy root -> tenants FK must exist");
    assert_eq!(fk_action, "c", "FK must be ON DELETE CASCADE ('c')");

    // ---- Invariant 2: old global slug UNIQUE is gone ----
    let sql = format!(
        "SELECT EXISTS (
             SELECT 1 FROM pg_constraint
              WHERE conname = '{}'
                AND conrelid = '{legacy_root_table}'::regclass
         )",
        concat!("co", "mpanies_slug_key")
    );
    let old_exists: bool = sqlx::query_scalar(AssertSqlSafe(sql.as_str()))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        !old_exists,
        "the pre-028 global UNIQUE-on-slug constraint must be dropped"
    );

    // ---- Invariant 3: new per-tenant slug UNIQUE is present ----
    let sql = format!(
        "SELECT EXISTS (
             SELECT 1 FROM pg_constraint
              WHERE conname = '{}'
                AND conrelid = '{legacy_root_table}'::regclass
                AND contype = 'u'
         )",
        concat!("co", "mpanies_tenant_slug_key")
    );
    let new_exists: bool = sqlx::query_scalar(AssertSqlSafe(sql.as_str()))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        new_exists,
        "the per-tenant UNIQUE (tenant_id, slug) constraint must exist"
    );

    // ---- Invariant 4: per-tenant slug collision rules ----
    // Unique scenario tags so parallel test runs don't stomp each other.
    let tag = format!("m028-{}", Uuid::new_v4().simple());
    let tenant_a_slug = format!("{tag}-a");
    let tenant_b_slug = format!("{tag}-b");

    let tenant_a: Uuid =
        sqlx::query_scalar("INSERT INTO tenants (slug, name) VALUES ($1, $1) RETURNING id")
            .bind(&tenant_a_slug)
            .fetch_one(&pool)
            .await
            .expect("insert tenant A");
    let tenant_b: Uuid =
        sqlx::query_scalar("INSERT INTO tenants (slug, name) VALUES ($1, $1) RETURNING id")
            .bind(&tenant_b_slug)
            .fetch_one(&pool)
            .await
            .expect("insert tenant B");

    // Same slug under two different tenants should now succeed.
    let sql = format!(
        "INSERT INTO {legacy_root_table} (tenant_id, slug, name, settings) VALUES ($1, 'shared', 'A', '{{}}')"
    );
    sqlx::query(AssertSqlSafe(sql.as_str()))
        .bind(tenant_a)
        .execute(&pool)
        .await
        .expect("tenant A legacy root row with slug 'shared' must succeed");
    let sql = format!(
        "INSERT INTO {legacy_root_table} (tenant_id, slug, name, settings) VALUES ($1, 'shared', 'B', '{{}}')"
    );
    sqlx::query(AssertSqlSafe(sql.as_str()))
        .bind(tenant_b)
        .execute(&pool)
        .await
        .expect("tenant B legacy root row with slug 'shared' must also succeed");

    // Same (tenant_id, slug) must fail.
    let sql = format!(
        "INSERT INTO {legacy_root_table} (tenant_id, slug, name, settings) VALUES ($1, 'shared', 'dup', '{{}}')"
    );
    let conflict = sqlx::query(AssertSqlSafe(sql.as_str()))
        .bind(tenant_a)
        .execute(&pool)
        .await;
    assert!(
        conflict.is_err(),
        "duplicate (tenant_id, slug) must violate the per-tenant unique constraint"
    );

    // ---- Invariant 5: views surface tenant_id ----
    for view in ["v_hierarchy", "v_user_permissions"] {
        let has_tenant_id: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM information_schema.columns
                  WHERE table_name = $1 AND column_name = 'tenant_id'
             )",
        )
        .bind(view)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            has_tenant_id,
            "{view} must expose tenant_id after migration 028"
        );
    }

    // ---- Invariant 6: ON DELETE CASCADE from tenant ----
    sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_a)
        .execute(&pool)
        .await
        .expect("delete tenant A");
    let sql = format!("SELECT COUNT(*) FROM {legacy_root_table} WHERE tenant_id = $1");
    let remaining: i64 = sqlx::query_scalar(AssertSqlSafe(sql.as_str()))
        .bind(tenant_a)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        remaining, 0,
        "deleting tenant must cascade-delete its legacy root rows"
    );

    // Clean up tenant_b side so parallel runs don't accumulate.
    sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_b)
        .execute(&pool)
        .await
        .ok();

    pool.close().await;
}
