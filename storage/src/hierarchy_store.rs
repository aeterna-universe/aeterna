//! Tenant-root organizational hierarchy storage.
//!
//! Owns read and write access to the modern tenant-scoped hierarchy tables in
//! their target shape: `Tenant -> Organization -> Team`. Legacy root-hierarchy
//! units are not canonical in-tenant nodes.

use sqlx::Row;
use uuid::Uuid;

use crate::postgres::PostgresError;

/// An organization owned directly by a tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Org {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub slug: String,
    pub name: String,
    pub teams: Vec<Team>,
}

/// A team under an organization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Team {
    pub id: Uuid,
    pub org_id: Uuid,
    pub slug: String,
    pub name: String,
}

/// Input shape for [`HierarchyStore::upsert_hierarchy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgInput {
    pub slug: String,
    pub name: String,
    pub teams: Vec<TeamInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamInput {
    pub slug: String,
    pub name: String,
}

/// Result of a successful [`HierarchyStore::upsert_hierarchy`] call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpsertSummary {
    pub orgs_upserted: usize,
    pub teams_upserted: usize,
}

/// Derive a URL-safe, lowercase, kebab-case slug from a free-form name.
pub fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = true;
    for ch in name.chars() {
        let mapped = ch.to_ascii_lowercase();
        if mapped.is_ascii_alphanumeric() {
            out.push(mapped);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "unnamed".to_string()
    } else {
        out
    }
}

#[derive(Clone)]
pub struct HierarchyStore {
    pool: sqlx::PgPool,
}

impl HierarchyStore {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Upsert a full tenant-root hierarchy under `tenant_id`.
    pub async fn upsert_hierarchy(
        &self,
        tenant_id: Uuid,
        orgs: &[OrgInput],
    ) -> Result<UpsertSummary, PostgresError> {
        let mut tx = self.pool.begin().await.map_err(PostgresError::Database)?;
        let mut summary = UpsertSummary::default();

        for org in orgs {
            let org_id: Uuid = sqlx::query_scalar(
                "INSERT INTO organizations (tenant_id, slug, name, created_at, updated_at)
                 VALUES ($1, $2, $3, NOW(), NOW())
                 ON CONFLICT (tenant_id, slug)
                 DO UPDATE SET name = EXCLUDED.name, updated_at = NOW()
                 RETURNING id",
            )
            .bind(tenant_id)
            .bind(&org.slug)
            .bind(&org.name)
            .fetch_one(&mut *tx)
            .await
            .map_err(PostgresError::Database)?;
            summary.orgs_upserted += 1;

            for team in &org.teams {
                sqlx::query(
                    "INSERT INTO teams (org_id, slug, name, created_at, updated_at)
                     VALUES ($1, $2, $3, NOW(), NOW())
                     ON CONFLICT (org_id, slug)
                     DO UPDATE SET name = EXCLUDED.name, updated_at = NOW()",
                )
                .bind(org_id)
                .bind(&team.slug)
                .bind(&team.name)
                .execute(&mut *tx)
                .await
                .map_err(PostgresError::Database)?;
                summary.teams_upserted += 1;
            }
        }

        tx.commit().await.map_err(PostgresError::Database)?;
        Ok(summary)
    }

    /// Read the full tenant-root hierarchy for `tenant_id` as a nested `Vec<Org>`.
    pub async fn get_hierarchy(&self, tenant_id: Uuid) -> Result<Vec<Org>, PostgresError> {
        let rows = sqlx::query(
            "SELECT org_id, org_slug, org_name,
                    team_id, team_slug, team_name
               FROM v_hierarchy
              WHERE tenant_id = $1
              ORDER BY org_slug, team_slug NULLS FIRST",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(PostgresError::Database)?;

        let mut orgs: Vec<Org> = Vec::new();
        for row in rows {
            let org_id: Uuid = row.get("org_id");
            let org_slug: String = row.get("org_slug");
            let org_name: String = row.get("org_name");

            let org_idx = match orgs.iter().position(|o| o.id == org_id) {
                Some(idx) => idx,
                None => {
                    orgs.push(Org {
                        id: org_id,
                        tenant_id,
                        slug: org_slug,
                        name: org_name,
                        teams: Vec::new(),
                    });
                    orgs.len() - 1
                }
            };

            let team_id: Option<Uuid> = row.try_get("team_id").ok();
            let Some(team_id) = team_id else {
                continue;
            };
            let team_slug: String = row.get("team_slug");
            let team_name: String = row.get("team_name");

            let org = &mut orgs[org_idx];
            if !org.teams.iter().any(|t| t.id == team_id) {
                org.teams.push(Team {
                    id: team_id,
                    org_id: org.id,
                    slug: team_slug,
                    name: team_name,
                });
            }
        }

        Ok(orgs)
    }
}

#[cfg(test)]
mod slugify_tests {
    use super::slugify;

    #[test]
    fn lowercases_and_kebab_cases_ascii() {
        assert_eq!(slugify("Acme Corp"), "acme-corp");
        assert_eq!(slugify("Acme   Corp"), "acme-corp");
        assert_eq!(slugify("Acme/Corp_2024"), "acme-corp-2024");
    }

    #[test]
    fn strips_leading_and_trailing_non_alphanumerics() {
        assert_eq!(slugify("   hello   "), "hello");
        assert_eq!(slugify("--foo--"), "foo");
        assert_eq!(slugify("///a//b///"), "a-b");
    }

    #[test]
    fn empty_or_all_punctuation_becomes_unnamed() {
        assert_eq!(slugify(""), "unnamed");
        assert_eq!(slugify("   "), "unnamed");
        assert_eq!(slugify("!!!"), "unnamed");
    }

    #[test]
    fn unicode_is_lossy_but_nonempty() {
        assert_eq!(slugify("café"), "caf");
        assert_eq!(slugify("日本語"), "unnamed");
        assert_eq!(slugify("日本 Corp"), "corp");
    }

    #[test]
    fn preserves_digits_and_single_dashes() {
        assert_eq!(slugify("team-42"), "team-42");
        assert_eq!(slugify("v1.0 release"), "v1-0-release");
    }
}
