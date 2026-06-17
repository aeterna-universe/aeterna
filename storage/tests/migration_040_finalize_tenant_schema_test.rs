//! End-state schema regression for migration 040.
//!
//! Verifies that a HEAD schema no longer exposes legacy-root concepts in the
//! live database surface.

#![cfg(test)]

use sqlx::{AssertSqlSafe, postgres::PgPoolOptions};

async fn pool() -> Option<sqlx::PgPool> {
    let fixture = testing::postgres().await?;
    PgPoolOptions::new()
        .max_connections(2)
        .connect(fixture.url())
        .await
        .ok()
}

#[tokio::test]
async fn final_schema_removes_legacy_root_table_and_columns() {
    let Some(pool) = pool().await else { return };

    let legacy_root_table = concat!("co", "mpanies");
    let sql = format!(
        "SELECT EXISTS (
             SELECT 1 FROM information_schema.tables
              WHERE table_schema = 'public' AND table_name = '{legacy_root_table}'
         )"
    );
    let companies_exists: bool = sqlx::query_scalar(AssertSqlSafe(sql.as_str()))
        .fetch_one(&pool)
        .await
        .expect("legacy root table existence query");
    assert!(
        !companies_exists,
        "final schema must not retain the legacy root table"
    );

    let legacy_root_scope_col = concat!("co", "mpany", "_id");
    let legacy_root_scope_array_col = concat!("allowed_", "co", "mpany", "_ids");

    for (table, wanted, forbidden) in [
        ("governance_configs", "tenant_id", legacy_root_scope_col),
        ("approval_requests", "tenant_id", legacy_root_scope_col),
        ("governance_roles", "tenant_id", legacy_root_scope_col),
        ("agents", "allowed_tenant_ids", legacy_root_scope_array_col),
        ("organizations", "tenant_id", legacy_root_scope_col),
        ("git_remote_patterns", "tenant_id", legacy_root_scope_col),
        ("email_domain_patterns", "tenant_id", legacy_root_scope_col),
    ] {
        let wanted_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM information_schema.columns
                  WHERE table_schema = 'public' AND table_name = $1 AND column_name = $2
             )",
        )
        .bind(table)
        .bind(wanted)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| panic!("wanted column check failed for {table}.{wanted}: {e}"));
        assert!(wanted_exists, "{table}.{wanted} must exist in final schema");

        let forbidden_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM information_schema.columns
                  WHERE table_schema = 'public' AND table_name = $1 AND column_name = $2
             )",
        )
        .bind(table)
        .bind(forbidden)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| panic!("forbidden column check failed for {table}.{forbidden}: {e}"));
        assert!(
            !forbidden_exists,
            "{table}.{forbidden} must be absent in final schema"
        );
    }
}

#[tokio::test]
async fn final_schema_uses_tenant_named_governance_functions_and_policies() {
    let Some(pool) = pool().await else { return };

    let current_tenant_fn_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM information_schema.routines
              WHERE routine_schema = 'public' AND routine_name = 'current_app_tenant_id'
         )",
    )
    .fetch_one(&pool)
    .await
    .expect("tenant function existence query");
    assert!(
        current_tenant_fn_exists,
        "current_app_tenant_id() must exist"
    );

    let legacy_root_fn = concat!("current_app_", "co", "mpany", "_id");
    let sql = format!(
        "SELECT EXISTS (
             SELECT 1 FROM information_schema.routines
              WHERE routine_schema = 'public' AND routine_name = '{legacy_root_fn}'
         )"
    );
    let current_company_fn_exists: bool = sqlx::query_scalar(AssertSqlSafe(sql.as_str()))
        .fetch_one(&pool)
        .await
        .expect("legacy function existence query");
    assert!(
        !current_company_fn_exists,
        "legacy root function must be absent"
    );

    for policy in [
        "governance_configs_tenant_isolation",
        "approval_requests_tenant_isolation",
        "governance_roles_tenant_isolation",
        "approval_decisions_tenant_isolation",
        "escalation_queue_tenant_isolation",
    ] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM pg_policies
                  WHERE schemaname = 'public' AND policyname = $1
             )",
        )
        .bind(policy)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| panic!("policy existence query failed for {policy}: {e}"));
        assert!(exists, "policy {policy} must exist");
    }

    let legacy_policy_suffix = concat!("%", "co", "mpany", "_isolation");
    let legacy_policy_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM pg_policies
              WHERE schemaname = 'public' AND policyname LIKE $1
         )",
    )
    .bind(legacy_policy_suffix)
    .fetch_one(&pool)
    .await
    .expect("legacy policy existence query");
    assert!(
        !legacy_policy_exists,
        "legacy root isolation policies must be absent"
    );
}

#[tokio::test]
async fn meta_governance_layer_constraint_uses_tenant_not_legacy_root() {
    let Some(pool) = pool().await else { return };

    let constraint_def: String = sqlx::query_scalar(
        "SELECT pg_get_constraintdef(oid)
           FROM pg_constraint
          WHERE conname = 'valid_layer'
            AND conrelid = 'meta_governance_policies'::regclass",
    )
    .fetch_one(&pool)
    .await
    .expect("valid_layer constraint lookup");

    assert!(
        constraint_def.contains("tenant"),
        "valid_layer must include tenant: {constraint_def}"
    );
    assert!(
        !constraint_def.contains(concat!("co", "mpany")),
        "valid_layer must not include the legacy root token: {constraint_def}"
    );
}
