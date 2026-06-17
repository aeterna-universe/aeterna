-- 037_organizations_tenant_root.sql
--
-- Prepare the tenant-root hierarchy migration by attaching every organization
-- directly to its owning tenant while the legacy root chain still exists.
--
-- IMPORTANT: this migration intentionally fails closed for tenants that still
-- have more than one active legacy root row. The target model is
-- Tenant -> Organization -> Team -> Project; automatically flattening multiple
-- active legacy root rows under the same tenant would be ambiguous and lossy.

ALTER TABLE organizations
    ADD COLUMN IF NOT EXISTS tenant_id UUID REFERENCES tenants(id) ON DELETE CASCADE;

DO $$
DECLARE
    conflicting_count INT;
    conflicting_tenants TEXT;
BEGIN
    SELECT COUNT(*),
           COALESCE(string_agg(t.slug, ', ' ORDER BY t.slug), '')
      INTO conflicting_count, conflicting_tenants
      FROM (
            SELECT c.tenant_id
              FROM companies c
             WHERE c.deleted_at IS NULL
             GROUP BY c.tenant_id
            HAVING COUNT(*) > 1
           ) multi
      JOIN tenants t ON t.id = multi.tenant_id;

    IF conflicting_count > 0 THEN
        RAISE EXCEPTION
            'Migration 037 aborted: % tenant(s) still have more than one active legacy root row. Automatic flattening to tenant-root organizations would be ambiguous. Affected tenants: [%]. Re-home or deactivate extra root rows before re-running.',
            conflicting_count,
            conflicting_tenants;
    END IF;
END $$;

UPDATE organizations o
   SET tenant_id = c.tenant_id
  FROM companies c
 WHERE o.company_id = c.id
   AND o.tenant_id IS NULL;

DO $$
DECLARE
    orphan_count INT;
BEGIN
    SELECT COUNT(*)
      INTO orphan_count
      FROM organizations
     WHERE tenant_id IS NULL;

    IF orphan_count > 0 THEN
        RAISE EXCEPTION
            'Migration 037 aborted: % organizations rows could not be backfilled with tenant_id from the legacy root table. Repair root ownership before re-running.',
            orphan_count;
    END IF;
END $$;

ALTER TABLE organizations
    ALTER COLUMN tenant_id SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_organizations_tenant
    ON organizations(tenant_id)
    WHERE deleted_at IS NULL;

COMMENT ON COLUMN organizations.tenant_id IS
    'Direct tenant ownership of an organization. Added by migration 037 as the first step toward a tenant-root hierarchy model.';
