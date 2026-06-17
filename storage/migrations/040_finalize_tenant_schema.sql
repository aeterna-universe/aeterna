-- 040_finalize_tenant_schema.sql
--
-- Finalize the tenant-root schema by removing the remaining live
-- company-root concepts from the end-state database objects.
--
-- This migration intentionally does NOT rewrite historical migration
-- files; it only mutates the resulting schema forward.

-- ---------------------------------------------------------------------
-- 1. Governance tables: company_id -> tenant_id
-- ---------------------------------------------------------------------
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'governance_configs' AND column_name = 'company_id'
    ) AND NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'governance_configs' AND column_name = 'tenant_id'
    ) THEN
        ALTER TABLE governance_configs RENAME COLUMN company_id TO tenant_id;
    END IF;

    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'approval_requests' AND column_name = 'company_id'
    ) AND NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'approval_requests' AND column_name = 'tenant_id'
    ) THEN
        ALTER TABLE approval_requests RENAME COLUMN company_id TO tenant_id;
    END IF;

    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'governance_roles' AND column_name = 'company_id'
    ) AND NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'governance_roles' AND column_name = 'tenant_id'
    ) THEN
        ALTER TABLE governance_roles RENAME COLUMN company_id TO tenant_id;
    END IF;
END $$;

ALTER INDEX IF EXISTS idx_governance_configs_company RENAME TO idx_governance_configs_tenant;
ALTER INDEX IF EXISTS idx_approval_requests_company RENAME TO idx_approval_requests_tenant;
ALTER INDEX IF EXISTS idx_governance_roles_company RENAME TO idx_governance_roles_tenant;

-- ---------------------------------------------------------------------
-- 2. Governance helper function: tenant-root naming and semantics
-- ---------------------------------------------------------------------
CREATE OR REPLACE FUNCTION get_effective_governance_config(
    p_tenant_id UUID DEFAULT NULL,
    p_org_id UUID DEFAULT NULL,
    p_team_id UUID DEFAULT NULL,
    p_project_id UUID DEFAULT NULL
) RETURNS TABLE (
    config_id UUID,
    scope_level TEXT,
    approval_mode TEXT,
    min_approvers INT,
    timeout_hours INT,
    auto_approve_low_risk BOOLEAN,
    escalation_enabled BOOLEAN,
    escalation_timeout_hours INT,
    escalation_contact TEXT,
    policy_settings JSONB,
    knowledge_settings JSONB,
    memory_settings JSONB
) AS $$
BEGIN
    IF p_project_id IS NOT NULL THEN
        RETURN QUERY
        SELECT gc.id, 'project'::TEXT, gc.approval_mode, gc.min_approvers, gc.timeout_hours,
               gc.auto_approve_low_risk, gc.escalation_enabled, gc.escalation_timeout_hours,
               gc.escalation_contact, gc.policy_settings, gc.knowledge_settings, gc.memory_settings
        FROM governance_configs gc
        WHERE gc.project_id = p_project_id LIMIT 1;
        IF FOUND THEN RETURN; END IF;
    END IF;

    IF p_team_id IS NOT NULL THEN
        RETURN QUERY
        SELECT gc.id, 'team'::TEXT, gc.approval_mode, gc.min_approvers, gc.timeout_hours,
               gc.auto_approve_low_risk, gc.escalation_enabled, gc.escalation_timeout_hours,
               gc.escalation_contact, gc.policy_settings, gc.knowledge_settings, gc.memory_settings
        FROM governance_configs gc
        WHERE gc.team_id = p_team_id AND gc.project_id IS NULL LIMIT 1;
        IF FOUND THEN RETURN; END IF;
    END IF;

    IF p_org_id IS NOT NULL THEN
        RETURN QUERY
        SELECT gc.id, 'org'::TEXT, gc.approval_mode, gc.min_approvers, gc.timeout_hours,
               gc.auto_approve_low_risk, gc.escalation_enabled, gc.escalation_timeout_hours,
               gc.escalation_contact, gc.policy_settings, gc.knowledge_settings, gc.memory_settings
        FROM governance_configs gc
        WHERE gc.org_id = p_org_id AND gc.team_id IS NULL AND gc.project_id IS NULL LIMIT 1;
        IF FOUND THEN RETURN; END IF;
    END IF;

    IF p_tenant_id IS NOT NULL THEN
        RETURN QUERY
        SELECT gc.id, 'tenant'::TEXT, gc.approval_mode, gc.min_approvers, gc.timeout_hours,
               gc.auto_approve_low_risk, gc.escalation_enabled, gc.escalation_timeout_hours,
               gc.escalation_contact, gc.policy_settings, gc.knowledge_settings, gc.memory_settings
        FROM governance_configs gc
        WHERE gc.tenant_id = p_tenant_id AND gc.org_id IS NULL AND gc.team_id IS NULL AND gc.project_id IS NULL LIMIT 1;
        IF FOUND THEN RETURN; END IF;
    END IF;

    RETURN QUERY SELECT
        gen_random_uuid(), 'default'::TEXT, 'standard'::TEXT, 1::INT, 72::INT,
        false::BOOLEAN, false::BOOLEAN, NULL::INT, NULL::TEXT,
        '{}'::JSONB, '{}'::JSONB, '{}'::JSONB;
END;
$$ LANGUAGE plpgsql;

-- ---------------------------------------------------------------------
-- 3. Governance RLS helpers/policies: company -> tenant
-- ---------------------------------------------------------------------
DROP POLICY IF EXISTS governance_configs_company_isolation ON governance_configs;
DROP POLICY IF EXISTS governance_configs_tenant_isolation ON governance_configs;
DROP POLICY IF EXISTS approval_requests_company_isolation ON approval_requests;
DROP POLICY IF EXISTS approval_requests_tenant_isolation ON approval_requests;
DROP POLICY IF EXISTS governance_roles_company_isolation ON governance_roles;
DROP POLICY IF EXISTS governance_roles_tenant_isolation ON governance_roles;
DROP POLICY IF EXISTS approval_decisions_company_isolation ON approval_decisions;
DROP POLICY IF EXISTS approval_decisions_tenant_isolation ON approval_decisions;
DROP POLICY IF EXISTS escalation_queue_company_isolation ON escalation_queue;
DROP POLICY IF EXISTS escalation_queue_tenant_isolation ON escalation_queue;

DROP FUNCTION IF EXISTS approval_request_belongs_to_current_company(uuid);
DROP FUNCTION IF EXISTS scope_belongs_to_current_company(uuid, uuid, uuid, uuid);
DROP FUNCTION IF EXISTS current_app_company_id();

CREATE OR REPLACE FUNCTION current_app_tenant_id()
RETURNS uuid AS $$
    SELECT NULLIF(current_setting('app.tenant_id', true), '')::uuid
$$ LANGUAGE sql STABLE;

CREATE OR REPLACE FUNCTION scope_belongs_to_current_tenant(
    p_tenant_id uuid,
    p_org_id uuid,
    p_team_id uuid,
    p_project_id uuid
)
RETURNS boolean AS $$
    SELECT CASE
        WHEN current_app_tenant_id() IS NULL THEN FALSE
        WHEN p_tenant_id IS NOT NULL THEN p_tenant_id = current_app_tenant_id()
        WHEN p_org_id IS NOT NULL THEN EXISTS (
            SELECT 1
            FROM organizations o
            WHERE o.id = p_org_id
              AND o.tenant_id = current_app_tenant_id()
        )
        WHEN p_team_id IS NOT NULL THEN EXISTS (
            SELECT 1
            FROM teams t
            JOIN organizations o ON o.id = t.org_id
            WHERE t.id = p_team_id
              AND o.tenant_id = current_app_tenant_id()
        )
        WHEN p_project_id IS NOT NULL THEN EXISTS (
            SELECT 1
            FROM projects p
            JOIN teams t ON t.id = p.team_id
            JOIN organizations o ON o.id = t.org_id
            WHERE p.id = p_project_id
              AND o.tenant_id = current_app_tenant_id()
        )
        ELSE FALSE
    END
$$ LANGUAGE sql STABLE;

CREATE OR REPLACE FUNCTION approval_request_belongs_to_current_tenant(
    p_request_id uuid
)
RETURNS boolean AS $$
    SELECT EXISTS (
        SELECT 1
        FROM approval_requests ar
        WHERE ar.id = p_request_id
          AND scope_belongs_to_current_tenant(
              ar.tenant_id,
              ar.org_id,
              ar.team_id,
              ar.project_id
          )
    )
$$ LANGUAGE sql STABLE;

CREATE POLICY governance_configs_tenant_isolation ON governance_configs
    FOR ALL
    USING (scope_belongs_to_current_tenant(tenant_id, org_id, team_id, project_id))
    WITH CHECK (scope_belongs_to_current_tenant(tenant_id, org_id, team_id, project_id));

CREATE POLICY approval_requests_tenant_isolation ON approval_requests
    FOR ALL
    USING (scope_belongs_to_current_tenant(tenant_id, org_id, team_id, project_id))
    WITH CHECK (scope_belongs_to_current_tenant(tenant_id, org_id, team_id, project_id));

CREATE POLICY governance_roles_tenant_isolation ON governance_roles
    FOR ALL
    USING (scope_belongs_to_current_tenant(tenant_id, org_id, team_id, project_id))
    WITH CHECK (scope_belongs_to_current_tenant(tenant_id, org_id, team_id, project_id));

CREATE POLICY approval_decisions_tenant_isolation ON approval_decisions
    FOR ALL
    USING (approval_request_belongs_to_current_tenant(request_id))
    WITH CHECK (approval_request_belongs_to_current_tenant(request_id));

CREATE POLICY escalation_queue_tenant_isolation ON escalation_queue
    FOR ALL
    USING (approval_request_belongs_to_current_tenant(request_id))
    WITH CHECK (approval_request_belongs_to_current_tenant(request_id));

-- ---------------------------------------------------------------------
-- 4. Agents: allowed_company_ids -> allowed_tenant_ids (semantic backfill)
-- ---------------------------------------------------------------------
DO $$
BEGIN
    ALTER TABLE agents
        ADD COLUMN IF NOT EXISTS allowed_tenant_ids UUID[];

    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'agents' AND column_name = 'allowed_company_ids'
    ) THEN
        EXECUTE $sql$
        UPDATE agents a
           SET allowed_tenant_ids = mapped.tenant_ids
          FROM (
                SELECT ag.id,
                       COALESCE(array_agg(DISTINCT c.tenant_id) FILTER (WHERE c.tenant_id IS NOT NULL), ARRAY[]::UUID[]) AS tenant_ids
                  FROM agents ag
             LEFT JOIN companies c ON c.id = ANY(ag.allowed_company_ids)
                 GROUP BY ag.id
               ) mapped
         WHERE a.id = mapped.id
        $sql$;

        ALTER TABLE agents
            DROP COLUMN IF EXISTS allowed_company_ids;
    END IF;
END $$;

DROP VIEW IF EXISTS v_agent_permissions;
CREATE VIEW v_agent_permissions AS
SELECT
    a.tenant_id,
    a.id AS agent_id,
    a.name AS agent_name,
    a.agent_type,
    a.delegated_by_user_id,
    a.delegated_by_agent_id,
    a.delegation_depth,
    a.capabilities,
    a.allowed_tenant_ids,
    a.allowed_org_ids,
    a.allowed_team_ids,
    a.allowed_project_ids,
    a.status AS agent_status,
    u.email AS delegating_user_email,
    u.name AS delegating_user_name
FROM agents a
LEFT JOIN users u ON u.id = a.delegated_by_user_id;

-- ---------------------------------------------------------------------
-- 5. Meta-governance layer values: company -> tenant
-- ---------------------------------------------------------------------
UPDATE meta_governance_policies
   SET layer = 'tenant'
 WHERE layer = 'company';

ALTER TABLE meta_governance_policies
    DROP CONSTRAINT IF EXISTS valid_layer;

ALTER TABLE meta_governance_policies
    ADD CONSTRAINT valid_layer CHECK (layer IN ('tenant', 'org', 'team', 'project'));

-- ---------------------------------------------------------------------
-- 6. Remove legacy root table from the end-state hierarchy
-- ---------------------------------------------------------------------
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.tables
        WHERE table_schema = 'public' AND table_name = 'companies'
    ) THEN
        UPDATE organizations o
           SET tenant_id = c.tenant_id
          FROM companies c
         WHERE o.company_id = c.id
           AND o.tenant_id IS NULL;

        ALTER TABLE git_remote_patterns
            ADD COLUMN IF NOT EXISTS tenant_id UUID REFERENCES tenants(id) ON DELETE CASCADE;

        UPDATE git_remote_patterns grp
           SET tenant_id = c.tenant_id
          FROM companies c
         WHERE grp.company_id = c.id
           AND grp.tenant_id IS NULL;

        ALTER TABLE email_domain_patterns
            ADD COLUMN IF NOT EXISTS tenant_id UUID REFERENCES tenants(id) ON DELETE CASCADE;

        UPDATE email_domain_patterns edp
           SET tenant_id = c.tenant_id
          FROM companies c
         WHERE edp.company_id = c.id
           AND edp.tenant_id IS NULL;
    END IF;
END $$;

ALTER TABLE organizations
    DROP COLUMN IF EXISTS company_id;

ALTER TABLE git_remote_patterns
    ALTER COLUMN tenant_id SET NOT NULL;
ALTER TABLE git_remote_patterns
    DROP COLUMN IF EXISTS company_id;
DROP INDEX IF EXISTS idx_git_remote_patterns_company;
CREATE INDEX IF NOT EXISTS idx_git_remote_patterns_tenant ON git_remote_patterns(tenant_id) WHERE enabled = true;

ALTER TABLE email_domain_patterns
    ALTER COLUMN tenant_id SET NOT NULL;
ALTER TABLE email_domain_patterns
    DROP COLUMN IF EXISTS company_id;

CREATE OR REPLACE FUNCTION cascade_soft_delete()
RETURNS TRIGGER AS $$
BEGIN
    IF TG_TABLE_NAME = 'organizations' THEN
        UPDATE teams
           SET deleted_at = NEW.deleted_at
         WHERE org_id = NEW.id AND deleted_at IS NULL;

        UPDATE projects
           SET deleted_at = NEW.deleted_at
         WHERE team_id IN (SELECT id FROM teams WHERE org_id = NEW.id)
           AND deleted_at IS NULL;

        UPDATE memberships
           SET status = 'inactive'
         WHERE team_id IN (SELECT id FROM teams WHERE org_id = NEW.id)
           AND status = 'active';
    ELSIF TG_TABLE_NAME = 'teams' THEN
        UPDATE projects
           SET deleted_at = NEW.deleted_at
         WHERE team_id = NEW.id AND deleted_at IS NULL;

        UPDATE memberships
           SET status = 'inactive'
         WHERE team_id = NEW.id AND status = 'active';
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP VIEW IF EXISTS v_orphan_organizations;
CREATE OR REPLACE VIEW v_orphan_organizations AS
SELECT
    o.id,
    o.slug,
    o.name,
    o.tenant_id AS broken_tenant_id,
    o.created_at
FROM organizations o
LEFT JOIN tenants t ON t.id = o.tenant_id
WHERE o.deleted_at IS NULL
  AND t.id IS NULL;

DROP TABLE IF EXISTS companies;
