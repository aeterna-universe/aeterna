use sqlx::{AssertSqlSafe, PgPool};

pub const TENANT_TABLES: [&str; 3] = ["sync_state", "memory_entries", "knowledge_items"];
pub const GOVERNANCE_TABLES: [&str; 4] = [
    "governance_configs",
    "approval_requests",
    "governance_roles",
    "approval_decisions",
];

pub async fn run_rls_migration(pool: &PgPool) -> Result<(), sqlx::Error> {
    for table in TENANT_TABLES {
        enable_tenant_rls_for_table(pool, table).await?;
    }

    enable_tenant_scope_rls_for_table(pool, "governance_configs").await?;
    enable_tenant_scope_rls_for_table(pool, "approval_requests").await?;
    enable_tenant_scope_rls_for_table(pool, "governance_roles").await?;
    enable_tenant_scope_rls_for_approval_decisions(pool).await?;

    Ok(())
}

async fn enable_tenant_rls_for_table(pool: &PgPool, table: &str) -> Result<(), sqlx::Error> {
    let enable_rls = format!("ALTER TABLE {} ENABLE ROW LEVEL SECURITY", table);
    sqlx::query(AssertSqlSafe(enable_rls.as_str()))
        .execute(pool)
        .await
        .ok();

    let policy_name = format!("{}_tenant_isolation", table);
    let drop_policy = format!("DROP POLICY IF EXISTS {} ON {}", policy_name, table);
    sqlx::query(AssertSqlSafe(drop_policy.as_str()))
        .execute(pool)
        .await
        .ok();

    let create_policy = format!(
        "CREATE POLICY {} ON {} FOR ALL USING (tenant_id = current_setting('app.tenant_id', \
         true)::text)",
        policy_name, table
    );
    sqlx::query(AssertSqlSafe(create_policy.as_str()))
        .execute(pool)
        .await
        .ok();

    Ok(())
}

async fn enable_tenant_scope_rls_for_table(pool: &PgPool, table: &str) -> Result<(), sqlx::Error> {
    let enable_rls = format!("ALTER TABLE {} ENABLE ROW LEVEL SECURITY", table);
    sqlx::query(AssertSqlSafe(enable_rls.as_str()))
        .execute(pool)
        .await
        .ok();

    let policy_name = format!("{}_tenant_scope_isolation", table);
    let drop_policy = format!("DROP POLICY IF EXISTS {} ON {}", policy_name, table);
    sqlx::query(AssertSqlSafe(drop_policy.as_str()))
        .execute(pool)
        .await
        .ok();

    // H2 normalization (issue #58 Bundle A.1): the single canonical app-side
    // GUC is `app.tenant_id`, and governance tables now use tenant_id
    // directly.
    let tenant_scope_clause = "tenant_id = current_setting('app.tenant_id', true)::uuid";
    let create_policy = format!(
        "CREATE POLICY {} ON {} FOR ALL USING ({}) WITH CHECK ({})",
        policy_name, table, tenant_scope_clause, tenant_scope_clause
    );
    sqlx::query(AssertSqlSafe(create_policy.as_str()))
        .execute(pool)
        .await
        .ok();

    Ok(())
}

async fn enable_tenant_scope_rls_for_approval_decisions(pool: &PgPool) -> Result<(), sqlx::Error> {
    let table = "approval_decisions";
    let enable_rls = format!("ALTER TABLE {} ENABLE ROW LEVEL SECURITY", table);
    sqlx::query(AssertSqlSafe(enable_rls.as_str()))
        .execute(pool)
        .await
        .ok();

    let policy_name = "approval_decisions_tenant_scope_isolation";
    let drop_policy = format!("DROP POLICY IF EXISTS {} ON {}", policy_name, table);
    sqlx::query(AssertSqlSafe(drop_policy.as_str()))
        .execute(pool)
        .await
        .ok();

    // H2 normalization: see note in `enable_tenant_scope_rls_for_table` above.
    let tenant_scope_clause = "EXISTS (SELECT 1 FROM approval_requests ar WHERE ar.id = approval_decisions.request_id AND ar.tenant_id = current_setting('app.tenant_id', true)::uuid)";
    let create_policy = format!(
        "CREATE POLICY {} ON {} FOR ALL USING ({}) WITH CHECK ({})",
        policy_name, table, tenant_scope_clause, tenant_scope_clause
    );
    sqlx::query(AssertSqlSafe(create_policy.as_str()))
        .execute(pool)
        .await
        .ok();

    Ok(())
}
