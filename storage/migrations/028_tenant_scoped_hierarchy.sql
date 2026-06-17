-- 028_tenant_scoped_hierarchy.sql
--
-- Make the organizational hierarchy's legacy root layer tenant-scoped by
-- introducing `companies.tenant_id UUID REFERENCES
-- tenants(id)`. Rationale, blast-radius analysis, and backfill strategy
-- are documented at length in:
--
--   openspec/changes/harden-tenant-provisioning/
--     NOTES-hierarchy-migration-blast-radius.md
--
-- Summary of the problem this fixes:
--
--   Migration 009 created the legacy root table 8 migrations before the
--   `tenants` table existed, under the explicit assumption that each root
--   row was itself a tenant. That made the legacy root slug globally unique.
--
--   Migration 017 introduced the real `tenants` table for
--   multi-tenant isolation but did not reconcile with 009.
--
--   The manifest hierarchy model assumed a single tenant could own
--   arbitrarily many legacy root rows. That contradicted the global
--   slug uniqueness constraint.
--
-- Resolution (decided 2026-04-22 22:16 UTC, option A):
--   Add a tenant FK to the legacy root table and swap global slug
--   uniqueness for per-tenant slug uniqueness.
--
-- Transitively scoped (no schema change needed):
--   - organizations (via the legacy root FK -> tenant)
--   - teams (via org -> legacy root -> tenant)
--   - projects (same chain)
--   - governance_roles / governance_configs / approval_workflows
--     (their scope-tuple columns point at legacy-root/org/team UUIDs)
--   - email_domain_patterns legacy-root FK, git_remote_patterns legacy-root FK
--
-- Not resolved here (tracked in blast-radius NOTES):
--   - email_domain_patterns.domain UNIQUE (global vs per-tenant)
--   - memberships / org_members / team_members apply from manifest
--     (deferred to §2.2-B5 follow-up)
--
-- Idempotency: this file must be runnable against a fresh DB and
-- against a DB where it has already been applied. Every statement
-- uses IF NOT EXISTS / IF EXISTS or a DO block that checks pg_*
-- catalog tables before acting.

-- ============================================================================
-- Step 1. Add tenant_id as nullable so the UPDATE below can populate it.
-- ============================================================================
ALTER TABLE companies
    ADD COLUMN IF NOT EXISTS tenant_id UUID REFERENCES tenants(id) ON DELETE CASCADE;

-- ============================================================================
-- Step 2. Backfill. Legacy root rows created before this migration were keyed only
-- by slug; match each to a tenant with the same slug. This is the invariant
-- bootstrap.rs + admin_sync.rs + commands/admin.rs now maintain after the
-- preceding "§2.2-B prereq" commit (36d2c51b) repaired their tenant-row
-- creation logic. Anything already populated (e.g. by a partial prior run)
-- is left alone.
-- ============================================================================
UPDATE companies c
   SET tenant_id = t.id
  FROM tenants t
 WHERE t.slug = c.slug
   AND c.tenant_id IS NULL;

-- ============================================================================
-- Step 3. Orphan abort. If any legacy root row has no matching tenants.slug,
-- we refuse to proceed. See blast-radius NOTES for the rationale against a
-- silent "migration-orphan" tenant (short version: silent reassignment in
-- prod is invisible data corruption). Operators facing this error should
-- either create the missing tenants rows or reassign the orphan legacy root rows
-- via manual SQL, then re-run the migration.
-- ============================================================================
DO $$
DECLARE
    orphan_count INT;
    orphan_slugs TEXT;
BEGIN
    SELECT COUNT(*), COALESCE(string_agg(slug, ', ' ORDER BY slug), '')
      INTO orphan_count, orphan_slugs
      FROM companies
     WHERE tenant_id IS NULL;

    IF orphan_count > 0 THEN
        RAISE EXCEPTION
            'Migration 028 aborted: % companies rows have no matching tenants.slug. Orphan slugs: [%]. Create the missing tenants rows (or reassign these companies) before re-running.',
            orphan_count, orphan_slugs;
    END IF;
END $$;

-- ============================================================================
-- Step 4. Lock tenant_id NOT NULL now that every row has a value.
-- Idempotent: SET NOT NULL on an already-NOT-NULL column is a no-op.
-- ============================================================================
ALTER TABLE companies
    ALTER COLUMN tenant_id SET NOT NULL;

-- ============================================================================
-- Step 5. Constraint surgery. `companies_slug_key` is the auto-named
-- constraint from `slug TEXT UNIQUE` in migration 009. Drop it and replace
-- with a tenant-scoped composite UNIQUE.
--
-- ADD CONSTRAINT does not support IF NOT EXISTS for UNIQUE in stable
-- Postgres (<= 16), so we guard with a catalog lookup to keep the migration
-- idempotent on repeat application.
-- ============================================================================
ALTER TABLE companies
    DROP CONSTRAINT IF EXISTS companies_slug_key;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'companies_tenant_slug_key'
           AND conrelid = 'companies'::regclass
    ) THEN
        ALTER TABLE companies
            ADD CONSTRAINT companies_tenant_slug_key UNIQUE (tenant_id, slug);
    END IF;
END $$;

-- ============================================================================
-- Step 6. Replace the old slug-only index with a tenant-scoped one.
-- The old index was `CREATE INDEX idx_companies_slug ON companies(slug)
-- WHERE deleted_at IS NULL` (migration 009).
-- ============================================================================
DROP INDEX IF EXISTS idx_companies_slug;
CREATE INDEX IF NOT EXISTS idx_companies_tenant_slug
    ON companies(tenant_id, slug)
    WHERE deleted_at IS NULL;

-- ============================================================================
-- Step 7. Rewrite views to surface tenant_id.
--
-- CREATE OR REPLACE VIEW would be preferred to preserve grants, but it
-- requires the replacement to have a compatible column list. We're adding
-- tenant_id as the first column, which changes the column sequence; that
-- is not a compatible change under Postgres's CREATE OR REPLACE rules.
-- DROP + CREATE is the only portable path.
--
-- No other views depend on v_hierarchy or v_user_permissions at time of
-- writing (verified via pg_depend on 009's post-migration state), so the
-- DROP has no cascading consequences. If a future migration adds a
-- dependent view, it must also DROP CASCADE when replacing these.
-- ============================================================================
DROP VIEW IF EXISTS v_hierarchy;
CREATE VIEW v_hierarchy AS
SELECT
    c.tenant_id,
    c.id as company_id,
    c.slug as company_slug,
    c.name as company_name,
    o.id as org_id,
    o.slug as org_slug,
    o.name as org_name,
    t.id as team_id,
    t.slug as team_slug,
    t.name as team_name,
    p.id as project_id,
    p.slug as project_slug,
    p.name as project_name,
    p.git_remote
FROM companies c
LEFT JOIN organizations o ON o.company_id = c.id AND o.deleted_at IS NULL
LEFT JOIN teams t ON t.org_id = o.id AND t.deleted_at IS NULL
LEFT JOIN projects p ON p.team_id = t.id AND p.deleted_at IS NULL
WHERE c.deleted_at IS NULL;

DROP VIEW IF EXISTS v_user_permissions;
CREATE VIEW v_user_permissions AS
SELECT
    c.tenant_id,
    u.id as user_id,
    u.email,
    u.name as user_name,
    u.status as user_status,
    m.team_id,
    m.role,
    m.permissions,
    t.org_id,
    o.company_id,
    c.slug as company_slug,
    o.slug as org_slug,
    t.slug as team_slug
FROM users u
JOIN memberships m ON m.user_id = u.id AND m.status = 'active'
JOIN teams t ON t.id = m.team_id AND t.deleted_at IS NULL
JOIN organizations o ON o.id = t.org_id AND o.deleted_at IS NULL
JOIN companies c ON c.id = o.company_id AND c.deleted_at IS NULL
WHERE u.deleted_at IS NULL AND u.status = 'active';

COMMENT ON VIEW v_hierarchy IS 'OPAL view: tenant-scoped organizational hierarchy for Cedar entities. tenant_id added in migration 028.';
COMMENT ON VIEW v_user_permissions IS 'OPAL view: tenant-scoped user permissions for Cedar authorization. tenant_id added in migration 028.';
COMMENT ON COLUMN companies.tenant_id IS 'FK to tenants(id). Added by migration 028. Each tenant owns one or more companies; (tenant_id, slug) is the per-tenant unique identifier of a company.';
