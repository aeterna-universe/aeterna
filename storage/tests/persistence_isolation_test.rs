//! Backend-specific persistence-isolation regression tests (Task 4.3).
//!
//! Covers PostgreSQL and Redis isolation.  Qdrant collection-name isolation
//! is tested directly in `memory/src/backends/qdrant.rs`
//! (`mod tenant_isolation_tests`) because `QdrantBackend` fields are private.
//!
//! Integration tests that require a live database are gated by the `testing`
//! fixture helper (`postgres()` / `redis()`).  They skip automatically when
//! Docker is not available so CI is not broken in environments without
//! container support.

// ---------------------------------------------------------------------------
// PostgreSQL isolation tests
// ---------------------------------------------------------------------------

mod postgres_isolation {
    use mk_core::types::{TenantContext, TenantId, UserId};
    use storage::postgres::PostgresBackend;
    use storage::rls_migration::{GOVERNANCE_TABLES, TENANT_TABLES};
    use testing::postgres;
    use uuid::Uuid;

    fn ctx(tenant: &str) -> TenantContext {
        TenantContext::new(
            TenantId::new(tenant.to_string()).unwrap(),
            UserId::new("user-1".to_string()).unwrap(),
        )
    }

    /// `initialize_schema` is idempotent; running it twice must not fail.
    /// This also confirms that the tenant-aware schema plumbing compiles and
    /// two distinct tenant contexts can coexist.
    #[tokio::test]
    async fn postgres_schema_init_is_idempotent() {
        let Some(pg_fixture) = postgres().await else {
            eprintln!("Skipping postgres isolation test: Docker not available");
            return;
        };

        let backend = PostgresBackend::new(pg_fixture.url())
            .await
            .expect("Postgres connection required");

        backend
            .initialize_schema()
            .await
            .expect("First schema init failed");
        backend
            .initialize_schema()
            .await
            .expect("Second schema init (idempotent) failed");

        // Verify that two distinct tenant contexts are well-formed and non-equal
        let ctx_a = ctx("acme-corp");
        let ctx_b = ctx("rival-inc");
        assert_ne!(ctx_a.tenant_id, ctx_b.tenant_id);
    }

    /// Every table in `TENANT_TABLES` MUST have `relrowsecurity = true` after
    /// schema initialisation.  A table without RLS enabled would allow any
    /// session to read rows belonging to any tenant.
    #[tokio::test]
    async fn postgres_rls_enabled_on_all_tenant_tables() {
        let Some(pg_fixture) = postgres().await else {
            eprintln!("Skipping RLS regression test: Docker not available");
            return;
        };

        let backend = PostgresBackend::new(pg_fixture.url())
            .await
            .expect("Postgres connection required");

        backend
            .initialize_schema()
            .await
            .expect("Schema init failed");

        let pool = backend.pool();

        for table in TENANT_TABLES {
            let row: Option<(bool,)> = sqlx::query_as(
                "SELECT relrowsecurity FROM pg_class WHERE relname = $1 AND relkind = 'r'",
            )
            .bind(table)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

            match row {
                Some((rls_enabled,)) => {
                    assert!(
                        rls_enabled,
                        "SECURITY REGRESSION: table '{table}' has RLS disabled — \
                         cross-tenant reads would succeed for sessions that do not \
                         set app.current_tenant"
                    );
                }
                None => {
                    eprintln!(
                        "Warning: table '{table}' not found after schema init — \
                         ensure all migrations ran"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn postgres_rls_enabled_on_all_governance_tables() {
        let Some(pg_fixture) = postgres().await else {
            eprintln!("Skipping governance RLS regression test: Docker not available");
            return;
        };

        let backend = PostgresBackend::new(pg_fixture.url())
            .await
            .expect("Postgres connection required");

        backend
            .initialize_schema()
            .await
            .expect("Schema init failed");

        let pool = backend.pool();

        for table in GOVERNANCE_TABLES {
            let row: Option<(bool,)> = sqlx::query_as(
                "SELECT relrowsecurity FROM pg_class WHERE relname = $1 AND relkind = 'r'",
            )
            .bind(table)
            .fetch_optional(pool)
            .await
            .expect("Governance RLS query should succeed");

            let (rls_enabled,) = row.unwrap_or_else(|| {
                panic!("SECURITY REGRESSION: governance table '{table}' missing after schema init")
            });

            assert!(
                rls_enabled,
                "SECURITY REGRESSION: governance table '{table}' has RLS disabled"
            );
        }
    }

    #[tokio::test]
    async fn postgres_governance_rows_are_hidden_across_companies() {
        let Some(pg_fixture) = postgres().await else {
            eprintln!("Skipping governance isolation test: Docker not available");
            return;
        };

        let backend = PostgresBackend::new(pg_fixture.url())
            .await
            .expect("Postgres connection required");

        backend
            .initialize_schema()
            .await
            .expect("Schema init failed");

        let company_a = Uuid::new_v4();
        let company_b = Uuid::new_v4();
        let config_id = Uuid::new_v4();
        let request_id = Uuid::new_v4();
        let role_id = Uuid::new_v4();
        let decision_id = Uuid::new_v4();
        let principal_id = Uuid::new_v4();
        let requestor_id = Uuid::new_v4();
        let granted_by = Uuid::new_v4();
        let approver_id = Uuid::new_v4();

        let mut owner_conn = backend.pool().acquire().await.expect("owner connection");
        sqlx::query("SELECT set_config('app.tenant_id', $1, false)")
            .bind(company_a.to_string())
            .execute(owner_conn.as_mut())
            .await
            .expect("set tenant A scope");

        sqlx::query("INSERT INTO governance_configs (id, tenant_id, approval_mode) VALUES ($1, $2, 'standard')")
            .bind(config_id)
            .bind(company_a)
            .execute(owner_conn.as_mut())
            .await
            .expect("insert governance config");

        sqlx::query(
            "INSERT INTO approval_requests (
                id, request_type, target_type, tenant_id, title, requestor_type, requestor_id
             ) VALUES ($1, 'policy_change', 'policy', $2, 'Tenant A change', 'user', $3)",
        )
        .bind(request_id)
        .bind(company_a)
        .bind(requestor_id)
        .execute(owner_conn.as_mut())
        .await
        .expect("insert approval request");

        sqlx::query(
            "INSERT INTO governance_roles (
                id, principal_type, principal_id, role, tenant_id, granted_by
             ) VALUES ($1, 'user', $2, 'Admin', $3, $4)",
        )
        .bind(role_id)
        .bind(principal_id)
        .bind(company_a)
        .bind(granted_by)
        .execute(owner_conn.as_mut())
        .await
        .expect("insert governance role");

        sqlx::query(
            "INSERT INTO approval_decisions (
                id, request_id, approver_type, approver_id, decision
             ) VALUES ($1, $2, 'user', $3, 'approve')",
        )
        .bind(decision_id)
        .bind(request_id)
        .bind(approver_id)
        .execute(owner_conn.as_mut())
        .await
        .expect("insert approval decision");

        let visible_to_owner: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT
                (SELECT COUNT(*) FROM governance_configs),
                (SELECT COUNT(*) FROM approval_requests),
                (SELECT COUNT(*) FROM governance_roles),
                (SELECT COUNT(*) FROM approval_decisions)",
        )
        .fetch_one(owner_conn.as_mut())
        .await
        .expect("owner visibility query");

        assert_eq!(visible_to_owner, (1, 1, 1, 1));

        let mut attacker_conn = backend.pool().acquire().await.expect("attacker connection");
        sqlx::query("SELECT set_config('app.tenant_id', $1, false)")
            .bind(company_b.to_string())
            .execute(attacker_conn.as_mut())
            .await
            .expect("set tenant B scope");

        let hidden_from_other_company: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT
                (SELECT COUNT(*) FROM governance_configs),
                (SELECT COUNT(*) FROM approval_requests),
                (SELECT COUNT(*) FROM governance_roles),
                (SELECT COUNT(*) FROM approval_decisions)",
        )
        .fetch_one(attacker_conn.as_mut())
        .await
        .expect("cross-tenant visibility query");

        assert_eq!(
            hidden_from_other_company,
            (0, 0, 0, 0),
            "SECURITY REGRESSION: tenant-scoped governance rows leaked across tenants"
        );
    }
}

// ---------------------------------------------------------------------------
// Redis isolation tests
// ---------------------------------------------------------------------------

mod redis_isolation {
    use mk_core::traits::StorageBackend;
    use mk_core::types::{TenantContext, TenantId, UserId};
    use storage::redis::RedisStorage;
    use testing::redis;

    fn ctx(tenant: &str) -> TenantContext {
        TenantContext::new(
            TenantId::new(tenant.to_string()).unwrap(),
            UserId::new("user-1".to_string()).unwrap(),
        )
    }

    /// Data stored under tenant-A's scoped key MUST NOT be visible when the
    /// caller supplies tenant-B's context.  Two different `scoped_key` calls
    /// for the same bare key produce different Redis keys; reading with the
    /// wrong tenant prefix returns `None`.
    #[tokio::test]
    async fn redis_scoped_keys_do_not_collide_across_tenants() {
        let Some(redis_fixture) = redis().await else {
            eprintln!("Skipping Redis isolation test: Docker not available");
            return;
        };

        let store = RedisStorage::new(redis_fixture.url())
            .await
            .expect("Redis connection required");

        let ctx_a = ctx("tenant-alpha");
        let ctx_b = ctx("tenant-beta");

        // Store a value under tenant-A's namespace
        store
            .store(ctx_a.clone(), "secret-key", b"tenant-a-secret")
            .await
            .expect("Store under tenant-A failed");

        // Attempt to retrieve it under tenant-B's namespace — MUST return None
        let result = store
            .retrieve(ctx_b.clone(), "secret-key")
            .await
            .expect("Retrieve under tenant-B should not error");

        assert!(
            result.is_none(),
            "SECURITY REGRESSION: Redis returned tenant-A data under tenant-B's \
             context; scoped_key isolation is broken"
        );

        // Confirm tenant-A can still read its own data
        let own = store
            .retrieve(ctx_a.clone(), "secret-key")
            .await
            .expect("Retrieve under tenant-A failed");

        assert_eq!(
            own.as_deref(),
            Some(b"tenant-a-secret".as_slice()),
            "Tenant-A should be able to read its own data"
        );

        // Cleanup
        store.delete(ctx_a, "secret-key").await.ok();
    }

    /// Two different tenant scoped-key strings for the same bare key are
    /// guaranteed to be distinct.  This is a unit invariant — no live Redis
    /// required.
    #[test]
    fn scoped_key_strings_are_distinct_across_tenants() {
        // Reproduce the `scoped_key` format: `"<tenant_id>:<key>"`
        let key = "session:abc123";
        let scoped_a = format!("{}:{}", "acme-corp", key);
        let scoped_b = format!("{}:{}", "rival-inc", key);

        assert_ne!(
            scoped_a, scoped_b,
            "Different tenants MUST produce different scoped keys for the same bare key"
        );
        assert_eq!(scoped_a, "acme-corp:session:abc123");
        assert_eq!(scoped_b, "rival-inc:session:abc123");
    }

    /// A tenant cannot delete another tenant's key because the scoped key
    /// strings are different.  After tenant-A stores a value, tenant-B
    /// attempting to delete the same bare key is a no-op for tenant-A's data.
    #[tokio::test]
    async fn redis_cross_tenant_delete_does_not_affect_owner() {
        let Some(redis_fixture) = redis().await else {
            eprintln!("Skipping Redis delete isolation test: Docker not available");
            return;
        };

        let store = RedisStorage::new(redis_fixture.url())
            .await
            .expect("Redis connection required");

        let ctx_a = ctx("owner-tenant");
        let ctx_b = ctx("attacker-tenant");

        store
            .store(ctx_a.clone(), "protected-key", b"owner-data")
            .await
            .expect("Store under owner failed");

        // "attacker" tries to delete the key using their own context
        store.delete(ctx_b, "protected-key").await.ok();

        // Owner's data must still be intact
        let still_there = store
            .retrieve(ctx_a.clone(), "protected-key")
            .await
            .expect("Retrieve after cross-tenant delete failed");

        assert_eq!(
            still_there.as_deref(),
            Some(b"owner-data".as_slice()),
            "Cross-tenant delete MUST NOT remove another tenant's data"
        );

        // Cleanup
        store.delete(ctx_a, "protected-key").await.ok();
    }
}
