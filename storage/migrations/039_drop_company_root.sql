-- 039_drop_company_root.sql
--
-- Remove the legacy root row as a required in-tenant hierarchy concept.
-- Organizations are now owned directly by tenants.

ALTER TABLE organizations
    ALTER COLUMN company_id DROP NOT NULL;

ALTER TABLE organizations
    DROP CONSTRAINT IF EXISTS organizations_company_id_slug_key;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'organizations_tenant_id_slug_key'
           AND conrelid = 'organizations'::regclass
    ) THEN
        ALTER TABLE organizations
            ADD CONSTRAINT organizations_tenant_id_slug_key UNIQUE (tenant_id, slug);
    END IF;
END $$;

DROP INDEX IF EXISTS idx_organizations_company;
CREATE INDEX IF NOT EXISTS idx_organizations_tenant_slug
    ON organizations(tenant_id, slug)
    WHERE deleted_at IS NULL;

UPDATE organizations
   SET company_id = NULL
 WHERE company_id IS NOT NULL;
