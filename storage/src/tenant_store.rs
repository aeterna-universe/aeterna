use chrono::Utc;
use mk_core::types::{
    AccountRef, BranchPolicy, CredentialKind, RecordSource, RepositoryKind, TenantId, TenantRecord,
    TenantRepositoryBinding, TenantStatus,
};
use sqlx::FromRow;

use crate::postgres::PostgresError;

#[derive(Debug, Clone, FromRow)]
struct TenantRow {
    id: String,
    slug: String,
    name: String,
    status: String,
    source_owner: String,
    environment: Option<String>,
    account_id: Option<String>,
    account_slug: Option<String>,
    account_name: Option<String>,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
    deactivated_at: Option<chrono::DateTime<Utc>>,
}

impl TryFrom<TenantRow> for TenantRecord {
    type Error = PostgresError;

    fn try_from(row: TenantRow) -> Result<Self, Self::Error> {
        let id = TenantId::new(row.id)
            .ok_or_else(|| PostgresError::NotFound("invalid tenant id in row".to_string()))?;
        let status = match row.status.as_str() {
            "active" => TenantStatus::Active,
            "inactive" => TenantStatus::Inactive,
            other => {
                return Err(PostgresError::NotFound(format!(
                    "invalid tenant status: {other}"
                )));
            }
        };

        Ok(TenantRecord {
            id,
            slug: row.slug,
            name: row.name,
            status,
            source_owner: row.source_owner.parse().unwrap_or(RecordSource::Admin),
            account: match (row.account_id, row.account_slug, row.account_name) {
                (Some(id), Some(slug), Some(name)) => Some(AccountRef { id, slug, name }),
                _ => None,
            },
            environment: row.environment,
            created_at: row.created_at,
            updated_at: row.updated_at,
            deactivated_at: row.deactivated_at,
        })
    }
}

#[derive(Clone)]
pub struct TenantStore {
    pool: sqlx::PgPool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TenantResolutionSource {
    Explicit,
    VerifiedDomain,
}

#[derive(Debug, Clone)]
pub struct TenantResolution {
    pub tenant: TenantRecord,
    pub source: TenantResolutionSource,
    pub mapping_domain: Option<String>,
}

#[derive(Debug)]
pub enum TenantResolutionError {
    MissingExplicitSelection,
    TenantNotFound(String),
    MissingVerifiedMapping,
    AmbiguousVerifiedMapping(Vec<String>),
    InvalidEmail,
    Storage(PostgresError),
}

impl std::fmt::Display for TenantResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TenantResolutionError::MissingExplicitSelection => {
                write!(f, "Explicit tenant selection is required")
            }
            TenantResolutionError::TenantNotFound(value) => write!(f, "Tenant not found: {value}"),
            TenantResolutionError::MissingVerifiedMapping => {
                write!(f, "No verified tenant mapping found")
            }
            TenantResolutionError::AmbiguousVerifiedMapping(values) => {
                write!(
                    f,
                    "Ambiguous verified tenant mapping: {}",
                    values.join(", ")
                )
            }
            TenantResolutionError::InvalidEmail => {
                write!(f, "Invalid email address for tenant resolution")
            }
            TenantResolutionError::Storage(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for TenantResolutionError {}

impl From<PostgresError> for TenantResolutionError {
    fn from(value: PostgresError) -> Self {
        Self::Storage(value)
    }
}

impl From<sqlx::Error> for TenantResolutionError {
    fn from(value: sqlx::Error) -> Self {
        Self::Storage(PostgresError::Database(value))
    }
}

#[derive(Debug, Clone, FromRow)]
struct TenantDomainRow {
    tenant_id: String,
}

impl TenantStore {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    pub async fn create_tenant(
        &self,
        slug: &str,
        name: &str,
    ) -> Result<TenantRecord, PostgresError> {
        self.create_tenant_with_metadata(slug, name, RecordSource::Admin, None, None)
            .await
    }

    /// Like [`create_tenant`] but lets the caller specify the `source_owner`.
    ///
    /// Use `RecordSource::Sync` when a tenant row is being created automatically
    /// by an IdP/sync flow so that it is distinguishable from admin-created tenants.
    pub async fn create_tenant_with_source(
        &self,
        slug: &str,
        name: &str,
        source_owner: RecordSource,
    ) -> Result<TenantRecord, PostgresError> {
        self.create_tenant_with_metadata(slug, name, source_owner, None, None)
            .await
    }

    pub async fn create_tenant_with_metadata(
        &self,
        slug: &str,
        name: &str,
        source_owner: RecordSource,
        account_id: Option<&str>,
        environment: Option<&str>,
    ) -> Result<TenantRecord, PostgresError> {
        let row: TenantRow = sqlx::query_as(
            r#"
            INSERT INTO tenants (slug, name, status, source_owner, account_id, environment)
            VALUES ($1, $2, 'active', $3, $4::uuid, $5)
            RETURNING id::text AS id,
                      slug,
                      name,
                      status,
                      source_owner,
                      environment,
                      (SELECT a.id::text FROM accounts a WHERE a.id = account_id AND a.deleted_at IS NULL) AS account_id,
                      (SELECT a.slug FROM accounts a WHERE a.id = account_id AND a.deleted_at IS NULL) AS account_slug,
                      (SELECT a.name FROM accounts a WHERE a.id = account_id AND a.deleted_at IS NULL) AS account_name,
                      created_at,
                      updated_at,
                      deactivated_at
            "#,
        )
        .bind(slug)
        .bind(name)
        .bind(source_owner.to_string())
        .bind(account_id)
        .bind(environment)
        .fetch_one(&self.pool)
        .await?;

        row.try_into()
    }

    pub async fn list_tenants(
        &self,
        include_inactive: bool,
    ) -> Result<Vec<TenantRecord>, PostgresError> {
        let rows: Vec<TenantRow> = sqlx::query_as(
            r#"
            SELECT t.id::text AS id,
                   t.slug,
                   t.name,
                   t.status,
                   t.source_owner,
                   t.environment,
                   a.id::text AS account_id,
                   a.slug AS account_slug,
                   a.name AS account_name,
                   t.created_at,
                   t.updated_at,
                   t.deactivated_at
              FROM tenants t
              LEFT JOIN accounts a ON a.id = t.account_id AND a.deleted_at IS NULL
             WHERE ($1::bool = true OR t.status = 'active')
             ORDER BY t.created_at ASC
            "#,
        )
        .bind(include_inactive)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn list_tenants_for_account(
        &self,
        account_ref: &str,
    ) -> Result<Vec<TenantRecord>, PostgresError> {
        let rows: Vec<TenantRow> = sqlx::query_as(
            r#"
            SELECT t.id::text AS id,
                   t.slug,
                   t.name,
                   t.status,
                   t.source_owner,
                   t.environment,
                   a.id::text AS account_id,
                   a.slug AS account_slug,
                   a.name AS account_name,
                   t.created_at,
                   t.updated_at,
                   t.deactivated_at
              FROM tenants t
              JOIN accounts a ON a.id = t.account_id AND a.deleted_at IS NULL
             WHERE a.id::text = $1 OR a.slug = $1 OR a.name = $1
             ORDER BY t.created_at ASC
            "#,
        )
        .bind(account_ref)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn get_tenant(
        &self,
        tenant_ref: &str,
    ) -> Result<Option<TenantRecord>, PostgresError> {
        let row: Option<TenantRow> = sqlx::query_as(
            r#"
            SELECT t.id::text AS id,
                   t.slug,
                   t.name,
                   t.status,
                   t.source_owner,
                   t.environment,
                   a.id::text AS account_id,
                   a.slug AS account_slug,
                   a.name AS account_name,
                   t.created_at,
                   t.updated_at,
                   t.deactivated_at
              FROM tenants t
              LEFT JOIN accounts a ON a.id = t.account_id AND a.deleted_at IS NULL
             WHERE t.id::text = $1 OR t.slug = $1 OR t.name = $1
             LIMIT 1
            "#,
        )
        .bind(tenant_ref)
        .fetch_optional(&self.pool)
        .await?;

        row.map(TryInto::try_into).transpose()
    }

    pub async fn update_tenant(
        &self,
        tenant_ref: &str,
        slug: Option<&str>,
        name: Option<&str>,
        environment: Option<&str>,
    ) -> Result<Option<TenantRecord>, PostgresError> {
        let row: Option<TenantRow> = sqlx::query_as(
            r#"
            UPDATE tenants t
               SET slug = COALESCE($2, t.slug),
                   name = COALESCE($3, t.name),
                   environment = COALESCE($4, t.environment),
                   updated_at = NOW()
             WHERE t.id::text = $1 OR t.slug = $1 OR t.name = $1
         RETURNING t.id::text AS id,
                   t.slug,
                   t.name,
                   t.status,
                   t.source_owner,
                   t.environment,
                   (SELECT a.id::text FROM accounts a WHERE a.id = t.account_id AND a.deleted_at IS NULL) AS account_id,
                   (SELECT a.slug FROM accounts a WHERE a.id = t.account_id AND a.deleted_at IS NULL) AS account_slug,
                   (SELECT a.name FROM accounts a WHERE a.id = t.account_id AND a.deleted_at IS NULL) AS account_name,
                   t.created_at,
                   t.updated_at,
                   t.deactivated_at
            "#,
        )
        .bind(tenant_ref)
        .bind(slug)
        .bind(name)
        .bind(environment)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            return row.try_into().map(Some);
        }

        self.get_tenant(tenant_ref).await
    }

    pub async fn attach_account(
        &self,
        tenant_ref: &str,
        account_id: &str,
    ) -> Result<Option<TenantRecord>, PostgresError> {
        let row: Option<TenantRow> = sqlx::query_as(
            r#"
            UPDATE tenants t
               SET account_id = $2::uuid,
                   updated_at = NOW()
             WHERE (t.id::text = $1 OR t.slug = $1 OR t.name = $1)
               AND EXISTS (
                   SELECT 1 FROM accounts a WHERE a.id::text = $2 AND a.deleted_at IS NULL
               )
         RETURNING t.id::text AS id,
                   t.slug,
                   t.name,
                   t.status,
                   t.source_owner,
                   t.environment,
                   (SELECT a.id::text FROM accounts a WHERE a.id = t.account_id AND a.deleted_at IS NULL) AS account_id,
                   (SELECT a.slug FROM accounts a WHERE a.id = t.account_id AND a.deleted_at IS NULL) AS account_slug,
                   (SELECT a.name FROM accounts a WHERE a.id = t.account_id AND a.deleted_at IS NULL) AS account_name,
                   t.created_at,
                   t.updated_at,
                   t.deactivated_at
            "#,
        )
        .bind(tenant_ref)
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await?;

        row.map(TryInto::try_into).transpose()
    }

    pub async fn detach_account(
        &self,
        tenant_ref: &str,
    ) -> Result<Option<TenantRecord>, PostgresError> {
        let row: Option<TenantRow> = sqlx::query_as(
            r#"
            UPDATE tenants t
               SET account_id = NULL,
                   updated_at = NOW()
             WHERE t.id::text = $1 OR t.slug = $1 OR t.name = $1
         RETURNING t.id::text AS id,
                   t.slug,
                   t.name,
                   t.status,
                   t.source_owner,
                   t.environment,
                   NULL::text AS account_id,
                   NULL::text AS account_slug,
                   NULL::text AS account_name,
                   t.created_at,
                   t.updated_at,
                   t.deactivated_at
            "#,
        )
        .bind(tenant_ref)
        .fetch_optional(&self.pool)
        .await?;

        row.map(TryInto::try_into).transpose()
    }

    pub async fn deactivate_tenant(
        &self,
        tenant_ref: &str,
    ) -> Result<Option<TenantRecord>, PostgresError> {
        let row: Option<TenantRow> = sqlx::query_as(
            r#"
            UPDATE tenants t
               SET status = 'inactive',
                   deactivated_at = COALESCE(deactivated_at, NOW()),
                   updated_at = NOW()
             WHERE t.id::text = $1 OR t.slug = $1 OR t.name = $1
         RETURNING t.id::text AS id,
                   t.slug,
                   t.name,
                   t.status,
                   t.source_owner,
                   t.environment,
                   (SELECT a.id::text FROM accounts a WHERE a.id = t.account_id AND a.deleted_at IS NULL) AS account_id,
                   (SELECT a.slug FROM accounts a WHERE a.id = t.account_id AND a.deleted_at IS NULL) AS account_slug,
                   (SELECT a.name FROM accounts a WHERE a.id = t.account_id AND a.deleted_at IS NULL) AS account_name,
                   t.created_at,
                   t.updated_at,
                   t.deactivated_at
            "#,
        )
        .bind(tenant_ref)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            return row.try_into().map(Some);
        }

        self.get_tenant(tenant_ref).await
    }

    pub async fn resolve_tenant_id(
        &self,
        tenant_ref: &str,
    ) -> Result<Option<TenantId>, PostgresError> {
        Ok(self.get_tenant(tenant_ref).await?.map(|record| record.id))
    }

    pub async fn ensure_tenant(&self, tenant_ref: &str) -> Result<TenantRecord, PostgresError> {
        self.ensure_tenant_with_source(tenant_ref, RecordSource::Admin)
            .await
    }

    /// Like [`ensure_tenant`] but tags new tenant rows with `source_owner`.
    ///
    /// Call with `RecordSource::Sync` from IdP/sync bootstrap paths so that
    /// sync-created tenants are distinguishable from admin-created ones.
    pub async fn ensure_tenant_with_source(
        &self,
        tenant_ref: &str,
        source_owner: RecordSource,
    ) -> Result<TenantRecord, PostgresError> {
        if let Some(record) = self.get_tenant(tenant_ref).await? {
            return Ok(record);
        }

        self.create_tenant_with_source(tenant_ref, tenant_ref, source_owner)
            .await
    }

    pub async fn add_verified_domain_mapping(
        &self,
        tenant_ref: &str,
        domain: &str,
    ) -> Result<TenantRecord, PostgresError> {
        self.add_verified_domain_mapping_with_source(tenant_ref, domain, RecordSource::Admin)
            .await
    }

    pub async fn add_verified_domain_mapping_with_source(
        &self,
        tenant_ref: &str,
        domain: &str,
        source_owner: RecordSource,
    ) -> Result<TenantRecord, PostgresError> {
        let tenant = self
            .get_tenant(tenant_ref)
            .await?
            .ok_or_else(|| PostgresError::NotFound(format!("tenant not found: {tenant_ref}")))?;

        sqlx::query(
            r#"
            INSERT INTO tenant_domain_mappings (tenant_id, domain, verified, source)
            VALUES ($1::uuid, lower($2), TRUE, $3)
            ON CONFLICT (tenant_id, domain)
            DO UPDATE SET
                verified = CASE
                    WHEN tenant_domain_mappings.source = 'admin' AND EXCLUDED.source <> 'admin'
                        THEN tenant_domain_mappings.verified
                    ELSE EXCLUDED.verified
                END,
                source = CASE
                    WHEN tenant_domain_mappings.source = 'admin' AND EXCLUDED.source <> 'admin'
                        THEN tenant_domain_mappings.source
                    ELSE EXCLUDED.source
                END,
                updated_at = CASE
                    WHEN tenant_domain_mappings.source = 'admin' AND EXCLUDED.source <> 'admin'
                        THEN tenant_domain_mappings.updated_at
                    ELSE NOW()
                END
            "#,
        )
        .bind(tenant.id.as_str())
        .bind(domain.trim())
        .bind(source_owner.to_string())
        .execute(&self.pool)
        .await?;

        Ok(tenant)
    }

    /// Returns every verified or unverified domain mapped to the given
    /// tenant, sorted lexicographically.
    ///
    /// Used by the manifest renderer (`cli/src/server/manifest_render.rs`)
    /// to populate `tenant.domainMappings` for
    /// `GET /api/v1/admin/tenants/{slug}/manifest`. The sort order is part
    /// of the contract — the renderer relies on it for deterministic output
    /// across pods, and the eventual structural-diff endpoint (§2.4) relies
    /// on it for stable comparison against submitted manifests.
    ///
    /// Returns an empty vec when the tenant exists but has no mappings.
    /// Returns `NotFound` when the tenant reference (`slug` or id) does
    /// not resolve.
    pub async fn list_domain_mappings(
        &self,
        tenant_ref: &str,
    ) -> Result<Vec<String>, PostgresError> {
        let tenant = self
            .get_tenant(tenant_ref)
            .await?
            .ok_or_else(|| PostgresError::NotFound(format!("tenant not found: {tenant_ref}")))?;

        let rows: Vec<(String,)> = sqlx::query_as(
            r#"
            SELECT domain
            FROM tenant_domain_mappings
            WHERE tenant_id = $1::uuid
            ORDER BY domain ASC
            "#,
        )
        .bind(tenant.id.as_str())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|(d,)| d).collect())
    }

    pub async fn resolve_verified_tenant(
        &self,
        explicit_tenant: Option<&str>,
        email: Option<&str>,
    ) -> Result<TenantResolution, TenantResolutionError> {
        if let Some(explicit) = explicit_tenant {
            let tenant = self
                .get_tenant(explicit)
                .await?
                .ok_or_else(|| TenantResolutionError::TenantNotFound(explicit.to_string()))?;
            return Ok(TenantResolution {
                tenant,
                source: TenantResolutionSource::Explicit,
                mapping_domain: None,
            });
        }

        let email = email.ok_or(TenantResolutionError::MissingExplicitSelection)?;
        let domain = email
            .split('@')
            .nth(1)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(TenantResolutionError::InvalidEmail)?
            .to_lowercase();

        let rows: Vec<TenantDomainRow> = sqlx::query_as(
            r#"
            SELECT tdm.tenant_id::text AS tenant_id, tdm.domain
            FROM tenant_domain_mappings tdm
            JOIN tenants t ON t.id = tdm.tenant_id
            WHERE tdm.verified = TRUE
              AND lower(tdm.domain) = $1
              AND t.status = 'active'
            ORDER BY t.created_at ASC
            "#,
        )
        .bind(&domain)
        .fetch_all(&self.pool)
        .await?;

        if rows.is_empty() {
            return Err(TenantResolutionError::MissingVerifiedMapping);
        }
        if rows.len() > 1 {
            return Err(TenantResolutionError::AmbiguousVerifiedMapping(
                rows.into_iter().map(|row| row.tenant_id).collect(),
            ));
        }

        let row = &rows[0];
        let tenant = self
            .get_tenant(&row.tenant_id)
            .await?
            .ok_or_else(|| TenantResolutionError::TenantNotFound(row.tenant_id.clone()))?;

        Ok(TenantResolution {
            tenant,
            source: TenantResolutionSource::VerifiedDomain,
            mapping_domain: Some(domain),
        })
    }

    pub fn pool(&self) -> &sqlx::PgPool {
        &self.pool
    }

    // ──── Manifest state (B2, migration 027) ────────────────────────────
    //
    // The `tenants` table carries two extra columns introduced by
    // `027_tenant_manifest_state.sql`:
    //
    //   - `last_applied_manifest_hash TEXT NULL`      SHA-256 fingerprint
    //                                                 of the last apply
    //   - `manifest_generation BIGINT NOT NULL = 0`   caller-owned revision
    //
    // The three methods below are the entire read/write surface. We keep
    // them small and orthogonal because `provision_tenant` is the only
    // caller today and the state model is narrow. Introducing a big
    // `TenantManifestState` type would lock in a shape before we have
    // more than one consumer.

    /// Read the manifest state for a tenant by slug.
    ///
    /// Returns `(last_applied_manifest_hash, manifest_generation)`. A hash
    /// of `None` means the tenant has never been applied via the B2
    /// idempotent path; callers MUST treat this as "no short-circuit
    /// possible" and run a full apply.
    ///
    /// Returns `PostgresError::NotFound` if the tenant row does not exist.
    pub async fn get_manifest_state(
        &self,
        slug: &str,
    ) -> Result<(Option<String>, i64), PostgresError> {
        let row: Option<(Option<String>, i64)> = sqlx::query_as(
            "SELECT last_applied_manifest_hash, manifest_generation \
             FROM tenants WHERE slug = $1",
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(PostgresError::from)?;

        row.ok_or_else(|| PostgresError::NotFound(format!("tenant not found: {slug}")))
    }

    /// Atomically set the manifest state on a successful apply.
    ///
    /// This method is intentionally permissive on `generation`: callers are
    /// expected to have performed the strict-monotonic check *before* calling
    /// this, while holding whatever lock is appropriate (today: the tx
    /// inside `provision_tenant`). The CHECK constraint from migration 027
    /// enforces `manifest_generation >= 0` at the DB layer; anything finer
    /// is application-level invariant.
    ///
    /// The hash argument is validated against the column CHECK constraint;
    /// a malformed value surfaces as a `PostgresError` at write time rather
    /// than being silently accepted.
    ///
    /// Returns `PostgresError::NotFound` if no tenant row exists for `slug`.
    pub async fn set_manifest_state(
        &self,
        slug: &str,
        hash: &str,
        generation: i64,
    ) -> Result<(), PostgresError> {
        let rows_affected = sqlx::query(
            "UPDATE tenants \
             SET last_applied_manifest_hash = $1, \
                 manifest_generation = $2, \
                 updated_at = NOW() \
             WHERE slug = $3",
        )
        .bind(hash)
        .bind(generation)
        .bind(slug)
        .execute(&self.pool)
        .await
        .map_err(PostgresError::from)?
        .rows_affected();

        if rows_affected == 0 {
            return Err(PostgresError::NotFound(format!("tenant not found: {slug}")));
        }
        Ok(())
    }

    /// Same as [`set_manifest_state`] but bound to an explicit tx. Used by
    /// `provision_tenant` when the state update must commit atomically with
    /// the manifest-body changes in the same transaction.
    pub async fn set_manifest_state_tx<'c>(
        tx: &mut sqlx::Transaction<'c, sqlx::Postgres>,
        slug: &str,
        hash: &str,
        generation: i64,
    ) -> Result<(), PostgresError> {
        let rows_affected = sqlx::query(
            "UPDATE tenants \
             SET last_applied_manifest_hash = $1, \
                 manifest_generation = $2, \
                 updated_at = NOW() \
             WHERE slug = $3",
        )
        .bind(hash)
        .bind(generation)
        .bind(slug)
        .execute(&mut **tx)
        .await
        .map_err(PostgresError::from)?
        .rows_affected();

        if rows_affected == 0 {
            return Err(PostgresError::NotFound(format!("tenant not found: {slug}")));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tenant repository binding store (task 3.1)
// ---------------------------------------------------------------------------

/// Private row type for `tenant_repository_bindings`.
#[derive(Debug, Clone, sqlx::FromRow)]
struct TenantRepositoryBindingRow {
    id: String,
    tenant_id: String,
    kind: String,
    local_path: Option<String>,
    remote_url: Option<String>,
    branch: String,
    branch_policy: String,
    credential_kind: String,
    credential_ref: Option<String>,
    github_owner: Option<String>,
    github_repo: Option<String>,
    source_owner: String,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
    // Added in task 3.4; the column may not yet exist in existing deployments.
    // sqlx will return None when the column is absent.
    #[sqlx(default)]
    git_provider_connection_id: Option<String>,
}

impl TryFrom<TenantRepositoryBindingRow> for TenantRepositoryBinding {
    type Error = PostgresError;

    fn try_from(row: TenantRepositoryBindingRow) -> Result<Self, Self::Error> {
        let tenant_id = TenantId::new(row.tenant_id).ok_or_else(|| {
            PostgresError::NotFound("invalid tenant_id in binding row".to_string())
        })?;

        let kind: RepositoryKind = row.kind.parse().map_err(|_| {
            PostgresError::NotFound(format!("unknown repository kind: {}", row.kind))
        })?;

        let branch_policy: BranchPolicy = row.branch_policy.parse().map_err(|_| {
            PostgresError::NotFound(format!("unknown branch policy: {}", row.branch_policy))
        })?;

        let credential_kind: CredentialKind = row.credential_kind.parse().map_err(|_| {
            PostgresError::NotFound(format!("unknown credential kind: {}", row.credential_kind))
        })?;

        Ok(TenantRepositoryBinding {
            id: row.id,
            tenant_id,
            kind,
            local_path: row.local_path,
            remote_url: row.remote_url,
            branch: row.branch,
            branch_policy,
            credential_kind,
            credential_ref: row.credential_ref,
            github_owner: row.github_owner,
            github_repo: row.github_repo,
            source_owner: row.source_owner.parse().unwrap_or(RecordSource::Admin),
            git_provider_connection_id: row.git_provider_connection_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

/// Request payload for creating or replacing a tenant repository binding.
#[derive(Debug, Clone)]
pub struct UpsertTenantRepositoryBinding {
    pub tenant_id: TenantId,
    pub kind: RepositoryKind,
    pub local_path: Option<String>,
    pub remote_url: Option<String>,
    pub branch: String,
    pub branch_policy: BranchPolicy,
    pub credential_kind: CredentialKind,
    /// Opaque secret-provider reference — never the raw credential value.
    pub credential_ref: Option<String>,
    pub github_owner: Option<String>,
    pub github_repo: Option<String>,
    pub source_owner: RecordSource,
    /// When set, reference a platform-owned Git provider connection by ID
    /// instead of embedding App credentials directly in `credential_ref`.
    pub git_provider_connection_id: Option<String>,
}

/// Storage operations for canonical per-tenant repository bindings.
pub struct TenantRepositoryBindingStore {
    pool: sqlx::PgPool,
}

impl TenantRepositoryBindingStore {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Returns the single canonical binding for `tenant_id`, or `None` if not
    /// yet configured.
    pub async fn get_binding(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Option<TenantRepositoryBinding>, PostgresError> {
        let row: Option<TenantRepositoryBindingRow> = sqlx::query_as(
            r#"
            SELECT
                id::text       AS id,
                tenant_id::text AS tenant_id,
                kind,
                local_path,
                remote_url,
                branch,
                branch_policy,
                credential_kind,
                credential_ref,
                github_owner,
                github_repo,
                source_owner,
                created_at,
                updated_at
            FROM tenant_repository_bindings
            WHERE tenant_id = $1::uuid
            LIMIT 1
            "#,
        )
        .bind(tenant_id.as_str())
        .fetch_optional(&self.pool)
        .await?;

        row.map(TryInto::try_into).transpose()
    }

    /// Upserts (insert-or-replace) the canonical binding for a tenant.
    ///
    /// **Ownership guard**: when the existing row has `source_owner = 'admin'`
    /// and the incoming request carries `source_owner = 'sync'` (or any
    /// non-admin value), every field of the existing row is preserved unchanged.
    /// This prevents IdP/sync runs from overwriting admin-managed bindings.
    ///
    /// Admin-originated requests (`source_owner = 'admin'`) always overwrite
    /// regardless of what the existing row holds.
    pub async fn upsert_binding(
        &self,
        req: UpsertTenantRepositoryBinding,
    ) -> Result<TenantRepositoryBinding, PostgresError> {
        let row: TenantRepositoryBindingRow = sqlx::query_as(
            r#"
            INSERT INTO tenant_repository_bindings
                (tenant_id, kind, local_path, remote_url, branch,
                 branch_policy, credential_kind, credential_ref,
                 github_owner, github_repo, source_owner)
            VALUES
                ($1::uuid, $2, $3, $4, $5,
                 $6, $7, $8,
                 $9, $10, $11)
            ON CONFLICT (tenant_id) DO UPDATE SET
                kind            = CASE WHEN tenant_repository_bindings.source_owner = 'admin' AND EXCLUDED.source_owner <> 'admin'
                                       THEN tenant_repository_bindings.kind
                                       ELSE EXCLUDED.kind            END,
                local_path      = CASE WHEN tenant_repository_bindings.source_owner = 'admin' AND EXCLUDED.source_owner <> 'admin'
                                       THEN tenant_repository_bindings.local_path
                                       ELSE EXCLUDED.local_path      END,
                remote_url      = CASE WHEN tenant_repository_bindings.source_owner = 'admin' AND EXCLUDED.source_owner <> 'admin'
                                       THEN tenant_repository_bindings.remote_url
                                       ELSE EXCLUDED.remote_url      END,
                branch          = CASE WHEN tenant_repository_bindings.source_owner = 'admin' AND EXCLUDED.source_owner <> 'admin'
                                       THEN tenant_repository_bindings.branch
                                       ELSE EXCLUDED.branch          END,
                branch_policy   = CASE WHEN tenant_repository_bindings.source_owner = 'admin' AND EXCLUDED.source_owner <> 'admin'
                                       THEN tenant_repository_bindings.branch_policy
                                       ELSE EXCLUDED.branch_policy   END,
                credential_kind = CASE WHEN tenant_repository_bindings.source_owner = 'admin' AND EXCLUDED.source_owner <> 'admin'
                                       THEN tenant_repository_bindings.credential_kind
                                       ELSE EXCLUDED.credential_kind END,
                credential_ref  = CASE WHEN tenant_repository_bindings.source_owner = 'admin' AND EXCLUDED.source_owner <> 'admin'
                                       THEN tenant_repository_bindings.credential_ref
                                       ELSE EXCLUDED.credential_ref  END,
                github_owner    = CASE WHEN tenant_repository_bindings.source_owner = 'admin' AND EXCLUDED.source_owner <> 'admin'
                                       THEN tenant_repository_bindings.github_owner
                                       ELSE EXCLUDED.github_owner    END,
                github_repo     = CASE WHEN tenant_repository_bindings.source_owner = 'admin' AND EXCLUDED.source_owner <> 'admin'
                                       THEN tenant_repository_bindings.github_repo
                                       ELSE EXCLUDED.github_repo     END,
                source_owner    = CASE WHEN tenant_repository_bindings.source_owner = 'admin' AND EXCLUDED.source_owner <> 'admin'
                                       THEN tenant_repository_bindings.source_owner
                                       ELSE EXCLUDED.source_owner    END,
                updated_at      = CASE WHEN tenant_repository_bindings.source_owner = 'admin' AND EXCLUDED.source_owner <> 'admin'
                                       THEN tenant_repository_bindings.updated_at
                                       ELSE NOW()                    END
            RETURNING
                id::text        AS id,
                tenant_id::text AS tenant_id,
                kind,
                local_path,
                remote_url,
                branch,
                branch_policy,
                credential_kind,
                credential_ref,
                github_owner,
                github_repo,
                source_owner,
                created_at,
                updated_at
            "#,
        )
        .bind(req.tenant_id.as_str())
        .bind(req.kind.to_string())
        .bind(&req.local_path)
        .bind(&req.remote_url)
        .bind(&req.branch)
        .bind(req.branch_policy.to_string())
        .bind(req.credential_kind.to_string())
        .bind(&req.credential_ref)
        .bind(&req.github_owner)
        .bind(&req.github_repo)
        .bind(req.source_owner.to_string())
        .fetch_one(&self.pool)
        .await?;

        row.try_into()
    }

    /// Deletes the canonical binding for `tenant_id`.
    ///
    /// Returns `true` if a row was deleted, `false` if no binding existed.
    pub async fn delete_binding(&self, tenant_id: &TenantId) -> Result<bool, PostgresError> {
        let result =
            sqlx::query("DELETE FROM tenant_repository_bindings WHERE tenant_id = $1::uuid")
                .bind(tenant_id.as_str())
                .execute(&self.pool)
                .await?;

        Ok(result.rows_affected() > 0)
    }
}
