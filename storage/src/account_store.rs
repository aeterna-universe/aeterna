use chrono::Utc;
use mk_core::types::AccountRecord;
use sqlx::FromRow;

use crate::postgres::PostgresError;

#[derive(Debug, Clone, FromRow)]
struct AccountRow {
    id: String,
    slug: String,
    name: String,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
    deleted_at: Option<chrono::DateTime<Utc>>,
}

impl From<AccountRow> for AccountRecord {
    fn from(row: AccountRow) -> Self {
        Self {
            id: row.id,
            slug: row.slug,
            name: row.name,
            created_at: row.created_at,
            updated_at: row.updated_at,
            deleted_at: row.deleted_at,
        }
    }
}

#[derive(Clone)]
pub struct AccountStore {
    pool: sqlx::PgPool,
}

impl AccountStore {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, slug: &str, name: &str) -> Result<AccountRecord, PostgresError> {
        let row: AccountRow = sqlx::query_as(
            r#"
            INSERT INTO accounts (slug, name, created_at, updated_at)
            VALUES ($1, $2, NOW(), NOW())
            RETURNING id::text AS id, slug, name, created_at, updated_at, deleted_at
            "#,
        )
        .bind(slug)
        .bind(name)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.into())
    }

    pub async fn list(&self) -> Result<Vec<AccountRecord>, PostgresError> {
        let rows: Vec<AccountRow> = sqlx::query_as(
            r#"
            SELECT id::text AS id, slug, name, created_at, updated_at, deleted_at
              FROM accounts
             WHERE deleted_at IS NULL
             ORDER BY created_at ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn get(&self, account_ref: &str) -> Result<Option<AccountRecord>, PostgresError> {
        let row: Option<AccountRow> = sqlx::query_as(
            r#"
            SELECT id::text AS id, slug, name, created_at, updated_at, deleted_at
              FROM accounts
             WHERE deleted_at IS NULL
               AND (id::text = $1 OR slug = $1 OR name = $1)
             LIMIT 1
            "#,
        )
        .bind(account_ref)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }

    pub async fn update(
        &self,
        account_ref: &str,
        slug: Option<&str>,
        name: Option<&str>,
    ) -> Result<Option<AccountRecord>, PostgresError> {
        let row: Option<AccountRow> = sqlx::query_as(
            r#"
            UPDATE accounts
               SET slug = COALESCE($2, slug),
                   name = COALESCE($3, name),
                   updated_at = NOW()
             WHERE deleted_at IS NULL
               AND (id::text = $1 OR slug = $1 OR name = $1)
         RETURNING id::text AS id, slug, name, created_at, updated_at, deleted_at
            "#,
        )
        .bind(account_ref)
        .bind(slug)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }

    pub async fn soft_delete(
        &self,
        account_ref: &str,
    ) -> Result<Option<AccountRecord>, PostgresError> {
        let row: Option<AccountRow> = sqlx::query_as(
            r#"
            UPDATE accounts
               SET deleted_at = NOW(),
                   updated_at = NOW()
             WHERE deleted_at IS NULL
               AND (id::text = $1 OR slug = $1 OR name = $1)
         RETURNING id::text AS id, slug, name, created_at, updated_at, deleted_at
            "#,
        )
        .bind(account_ref)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }

    pub async fn list_tenants(
        &self,
        account_ref: &str,
    ) -> Result<Vec<mk_core::types::TenantRecord>, PostgresError> {
        let account = match self.get(account_ref).await? {
            Some(account) => account,
            None => return Ok(Vec::new()),
        };
        crate::tenant_store::TenantStore::new(self.pool.clone())
            .list_tenants_for_account(&account.id)
            .await
    }
}
