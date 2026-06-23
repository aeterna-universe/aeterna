//! Regression test for issue #194 — migration 025 must succeed when run by a
//! non-superuser (the CNPG-managed `aeterna` app owner, which has CREATEROLE
//! but not SUPERUSER).
//!
//! Before the fix, migration 025 unconditionally issued
//! `CREATE ROLE … BYPASSRLS`, `ALTER ROLE … BYPASSRLS`, and
//! `ALTER ROLE … NOBYPASSRLS`, all of which require SUPERUSER. Every
//! greenfield / DB-wipe deploy where migrations ran as the app user stalled
//! at v24.
//!
//! The fix has two halves:
//!   1. Migration 025 is now superuser-aware: when `current_user` is not a
//!      superuser it skips the BYPASSRLS-touching statements with a WARNING
//!      (the roles are expected to be pre-provisioned by the prereqs layer).
//!   2. The prereqs layer (CNPG `managed.roles`) provisions the roles as
//!      superuser.
//!
//! This test simulates half (2): the superuser pre-provisions the roles,
//! then runs the FULL migration chain connected as a CREATEROLE-but-not-
//! SUPERUSER role and asserts it completes cleanly. It then also verifies
//! the superuser-run path still creates/repairs the roles.
//!
//! Docker-gated: falls back to a no-op notice when Docker is unavailable,
//! matching the project-wide convention.

use sqlx::Row;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

/// Start a fresh Postgres container (superuser = `testuser`).
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

/// `rolbypassrls` for the named role, queried via the superuser pool.
async fn role_bypassrls(pool: &sqlx::PgPool, role: &str) -> bool {
    sqlx::query("SELECT rolbypassrls FROM pg_roles WHERE rolname = $1")
        .bind(role)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("query rolbypassrls for {role}: {e}"))
        .get::<bool, _>("rolbypassrls")
}

/// Whether a role exists.
async fn role_exists(pool: &sqlx::PgPool, role: &str) -> bool {
    sqlx::query("SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname = $1) AS ok")
        .bind(role)
        .fetch_one(pool)
        .await
        .expect("role existence query")
        .get::<bool, _>("ok")
}

/// Run the full migration chain as a **non-superuser** that has CREATEROLE
/// (mirrors the CNPG-managed `aeterna` owner). The login roles are
/// pre-provisioned by the superuser, as the CNPG `managed.roles` layer would.
#[tokio::test]
async fn migration_025_runs_as_non_superuser() {
    let Some((container, super_url)) = fresh_container().await else {
        eprintln!("Skipping non-superuser migration test: Docker not available");
        return;
    };
    let _container = container; // keep alive

    let super_pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&super_url)
        .await
        .expect("superuser pool");

    // --- Simulate the CNPG prereqs layer (runs as SUPERUSER) ---------------
    //
    // 1. Create the app-owner role `aeterna` with CREATEROLE but NOT
    //    SUPERUSER — exactly what CNPG assigns to the `bootstrap.initdb.owner`.
    sqlx::query("CREATE ROLE aeterna LOGIN CREATEROLE PASSWORD 'apppw'")
        .execute(&super_pool)
        .await
        .expect("create aeterna owner role");

    // Transfer ownership of the test DB and its objects to `aeterna`, and
    // grant the privileges it needs to run migrations (create objects, grant
    // on schema, set defaults). This mirrors what CNPG does for the owner.
    sqlx::query("ALTER DATABASE testdb OWNER TO aeterna")
        .execute(&super_pool)
        .await
        .expect("transfer db ownership");
    sqlx::query("GRANT ALL ON SCHEMA public TO aeterna")
        .execute(&super_pool)
        .await
        .expect("grant schema public");

    // pgcrypto extension (needed by initialize_schema for gen_random_uuid).
    // Must be created by the superuser — non-superusers can't CREATE EXTENSION.
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pgcrypto")
        .execute(&super_pool)
        .await
        .expect("create pgcrypto extension");

    // Existing objects (none yet, but be safe): transfer ownership.
    sqlx::query(
        "DO $g$ BEGIN \
         EXECUTE format('ALTER TABLE IF EXISTS %I OWNER TO aeterna', t.tablename) \
         FROM (SELECT tablename FROM pg_tables WHERE schemaname='public') AS t; \
         END $g$;",
    )
    .execute(&super_pool)
    .await
    .ok();

    // 2. Pre-provision the application roles with their BYPASSRLS attributes,
    //    exactly as the CNPG `managed.roles` block in
    //    charts/aeterna-prereqs/templates/cloudnativepg-cluster.yaml does.
    sqlx::query("CREATE ROLE aeterna_app LOGIN NOBYPASSRLS PASSWORD NULL")
        .execute(&super_pool)
        .await
        .expect("pre-provision aeterna_app");
    sqlx::query("CREATE ROLE aeterna_admin LOGIN BYPASSRLS PASSWORD NULL")
        .execute(&super_pool)
        .await
        .expect("pre-provision aeterna_admin");

    // Sanity: the app owner is NOT a superuser.
    let owner_is_super: bool =
        sqlx::query("SELECT rolsuper FROM pg_roles WHERE rolname = 'aeterna'")
            .fetch_one(&super_pool)
            .await
            .expect("query owner rolsuper")
            .get("rolsuper");
    assert!(
        !owner_is_super,
        "precondition: the `aeterna` migration role must NOT be a superuser"
    );

    // --- Run the FULL migration chain as the non-superuser `aeterna` ------
    //
    // Parse the superuser URL and swap credentials to connect as `aeterna`.
    let parsed = url::Url::parse(&super_url).expect("valid url");
    let host = parsed.host_str().unwrap_or("localhost");
    let port = parsed.port().unwrap_or(5432);
    let db = parsed.path().trim_start_matches('/');
    let nonsuper_url = format!("postgres://aeterna:apppw@{host}:{port}/{db}");

    let nonsuper_pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&nonsuper_url)
        .await
        .expect("connect as non-superuser aeterna");

    // Core inline schema (mirrors the `testing::postgres()` fixture setup,
    // which calls PostgresBackend::initialize_schema before migrations).
    let backend = storage::postgres::PostgresBackend::new(&nonsuper_url)
        .await
        .expect("PostgresBackend as non-superuser");
    backend
        .initialize_schema()
        .await
        .expect("initialize_schema as non-superuser");

    // THE assertion: the full migration chain, including 025, must succeed
    // when run by a CREATEROLE-but-not-SUPERUSER role with the login roles
    // pre-provisioned. Before the fix this panicked at migration 025.
    storage::migrations::apply_all(&nonsuper_pool)
        .await
        .expect("apply_all migrations as non-superuser should succeed");

    // --- Verify the roles still have their correct attributes -------------
    assert!(
        role_exists(&super_pool, "aeterna_app").await,
        "aeterna_app must exist after non-superuser migration"
    );
    assert!(
        !role_bypassrls(&super_pool, "aeterna_app").await,
        "aeterna_app must be NOBYPASSRLS after non-superuser migration"
    );
    assert!(
        role_exists(&super_pool, "aeterna_admin").await,
        "aeterna_admin must exist after non-superuser migration"
    );
    assert!(
        role_bypassrls(&super_pool, "aeterna_admin").await,
        "aeterna_admin must be BYPASSRLS after non-superuser migration"
    );

    // Verify the non-role parts of migration 025 still applied: the
    // governance_audit_log.admin_scope column and its partial index.
    let has_admin_scope: bool = sqlx::query(
        "SELECT EXISTS (\
            SELECT 1 FROM information_schema.columns \
            WHERE table_schema='public' AND table_name='governance_audit_log' \
            AND column_name='admin_scope') AS ok",
    )
    .fetch_one(&super_pool)
    .await
    .expect("query admin_scope column")
    .get("ok");
    assert!(
        has_admin_scope,
        "migration 025's admin_scope column must be present after non-superuser run"
    );

    let has_index: bool = sqlx::query(
        "SELECT EXISTS (SELECT 1 FROM pg_indexes \
         WHERE schemaname='public' AND indexname='idx_governance_audit_log_admin_scope') AS ok",
    )
    .fetch_one(&super_pool)
    .await
    .expect("query admin_scope index")
    .get("ok");
    assert!(
        has_index,
        "migration 025's admin_scope partial index must be present after non-superuser run"
    );

    nonsuper_pool.close().await;
    super_pool.close().await;
}

/// The superuser path still works and creates the roles itself when they
/// don't already exist (local dev with the `postgres` superuser).
#[tokio::test]
async fn migration_025_creates_roles_as_superuser() {
    let Some((container, super_url)) = fresh_container().await else {
        eprintln!("Skipping superuser migration test: Docker not available");
        return;
    };
    let _container = container;

    let super_pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&super_url)
        .await
        .expect("superuser pool");

    // Migration 025 references `ALTER DEFAULT PRIVILEGES FOR ROLE aeterna`.
    // The testcontainers default user is `testuser`, so pre-create `aeterna`
    // to match the migration's expectation.
    sqlx::query("CREATE ROLE aeterna LOGIN PASSWORD 'testpass'")
        .execute(&super_pool)
        .await
        .ok();

    let backend = storage::postgres::PostgresBackend::new(&super_url)
        .await
        .expect("PostgresBackend as superuser");
    backend
        .initialize_schema()
        .await
        .expect("initialize_schema as superuser");

    // Roles must NOT exist yet.
    assert!(!role_exists(&super_pool, "aeterna_app").await);
    assert!(!role_exists(&super_pool, "aeterna_admin").await);

    storage::migrations::apply_all(&super_pool)
        .await
        .expect("apply_all migrations as superuser");

    // The superuser path must have created them with the right attributes.
    assert!(role_exists(&super_pool, "aeterna_app").await);
    assert!(!role_bypassrls(&super_pool, "aeterna_app").await);
    assert!(role_exists(&super_pool, "aeterna_admin").await);
    assert!(role_bypassrls(&super_pool, "aeterna_admin").await);

    // Re-running (idempotent path) must not error and must keep attributes.
    storage::migrations::apply_all(&super_pool)
        .await
        .expect("re-apply migrations as superuser (idempotent)");
    assert!(!role_bypassrls(&super_pool, "aeterna_app").await);
    assert!(role_bypassrls(&super_pool, "aeterna_admin").await);

    super_pool.close().await;
}
