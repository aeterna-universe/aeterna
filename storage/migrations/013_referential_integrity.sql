-- Section 13.3: PostgreSQL Referential Integrity Enhancement
-- This migration adds foreign key constraints, cascading soft-delete,
-- and orphan detection for organizational referential integrity.

-- ============================================================================
-- 13.3.1: Add Foreign Key Constraints to Existing Tables
-- Ensure all relationship columns have proper FK constraints
-- ============================================================================

-- Add missing FK constraints to agents table
ALTER TABLE agents
    DROP CONSTRAINT IF EXISTS fk_agents_delegated_by_user,
    ADD CONSTRAINT fk_agents_delegated_by_user
        FOREIGN KEY (delegated_by_user_id) 
        REFERENCES users(id) 
        ON DELETE SET NULL;

ALTER TABLE agents
    DROP CONSTRAINT IF EXISTS fk_agents_delegated_by_agent,
    ADD CONSTRAINT fk_agents_delegated_by_agent
        FOREIGN KEY (delegated_by_agent_id) 
        REFERENCES agents(id) 
        ON DELETE SET NULL;

-- Add FK constraints to email_domain_patterns
ALTER TABLE email_domain_patterns
    DROP CONSTRAINT IF EXISTS fk_email_domain_default_org,
    ADD CONSTRAINT fk_email_domain_default_org
        FOREIGN KEY (default_org_id) 
        REFERENCES organizations(id) 
        ON DELETE SET NULL;

ALTER TABLE email_domain_patterns
    DROP CONSTRAINT IF EXISTS fk_email_domain_default_team,
    ADD CONSTRAINT fk_email_domain_default_team
        FOREIGN KEY (default_team_id) 
        REFERENCES teams(id) 
        ON DELETE SET NULL;

-- ============================================================================
-- 13.3.2: Cascading Soft-Delete Function
-- When a parent is soft-deleted, optionally cascade to children
-- ============================================================================

CREATE OR REPLACE FUNCTION cascade_soft_delete()
RETURNS TRIGGER AS $$
DECLARE
    child_table TEXT;
    fk_column TEXT;
BEGIN
    -- Cascade soft delete to organizations when the legacy root row is deleted
    IF TG_TABLE_NAME = 'companies' THEN
        UPDATE organizations 
        SET deleted_at = NEW.deleted_at
        WHERE company_id = NEW.id AND deleted_at IS NULL;
        
        -- Cascade to teams (via organizations)
        UPDATE teams 
        SET deleted_at = NEW.deleted_at
        WHERE org_id IN (SELECT id FROM organizations WHERE company_id = NEW.id)
        AND deleted_at IS NULL;
        
        -- Cascade to projects (via teams)
        UPDATE projects 
        SET deleted_at = NEW.deleted_at
        WHERE team_id IN (
            SELECT t.id FROM teams t
            JOIN organizations o ON o.id = t.org_id
            WHERE o.company_id = NEW.id
        ) AND deleted_at IS NULL;
        
        -- Cascade to memberships (via teams)
        UPDATE memberships 
        SET status = 'inactive'
        WHERE team_id IN (
            SELECT t.id FROM teams t
            JOIN organizations o ON o.id = t.org_id
            WHERE o.company_id = NEW.id
        ) AND status = 'active';
    
    -- Cascade soft delete to teams when organization is deleted
    ELSIF TG_TABLE_NAME = 'organizations' THEN
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
    
    -- Cascade soft delete to projects when team is deleted
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

-- Apply cascading soft-delete trigger
DROP TRIGGER IF EXISTS cascade_soft_delete_company ON companies;
CREATE TRIGGER cascade_soft_delete_company
    AFTER UPDATE OF deleted_at ON companies
    FOR EACH ROW
    WHEN (NEW.deleted_at IS NOT NULL AND OLD.deleted_at IS NULL)
    EXECUTE FUNCTION cascade_soft_delete();

DROP TRIGGER IF EXISTS cascade_soft_delete_org ON organizations;
CREATE TRIGGER cascade_soft_delete_org
    AFTER UPDATE OF deleted_at ON organizations
    FOR EACH ROW
    WHEN (NEW.deleted_at IS NOT NULL AND OLD.deleted_at IS NULL)
    EXECUTE FUNCTION cascade_soft_delete();

DROP TRIGGER IF EXISTS cascade_soft_delete_team ON teams;
CREATE TRIGGER cascade_soft_delete_team
    AFTER UPDATE OF deleted_at ON teams
    FOR EACH ROW
    WHEN (NEW.deleted_at IS NOT NULL AND OLD.deleted_at IS NULL)
    EXECUTE FUNCTION cascade_soft_delete();

-- ============================================================================
-- 13.3.3: Orphan Detection Views and Functions
-- Identify records with broken referential integrity
-- ============================================================================

-- View: Orphaned organizations (missing legacy root row)
CREATE OR REPLACE VIEW v_orphan_organizations AS
SELECT 
    o.id,
    o.slug,
    o.name,
    o.company_id as broken_company_id,
    o.created_at
FROM organizations o
LEFT JOIN companies c ON c.id = o.company_id
WHERE o.deleted_at IS NULL 
AND (c.id IS NULL OR c.deleted_at IS NOT NULL);

-- View: Orphaned teams (missing org)
CREATE OR REPLACE VIEW v_orphan_teams AS
SELECT 
    t.id,
    t.slug,
    t.name,
    t.org_id as broken_org_id,
    t.created_at
FROM teams t
LEFT JOIN organizations o ON o.id = t.org_id
WHERE t.deleted_at IS NULL 
AND (o.id IS NULL OR o.deleted_at IS NOT NULL);

-- View: Orphaned projects (missing team)
CREATE OR REPLACE VIEW v_orphan_projects AS
SELECT 
    p.id,
    p.slug,
    p.name,
    p.team_id as broken_team_id,
    p.created_at
FROM projects p
LEFT JOIN teams t ON t.id = p.team_id
WHERE p.deleted_at IS NULL 
AND (t.id IS NULL OR t.deleted_at IS NOT NULL);

-- View: Orphaned memberships (missing user or team)
CREATE OR REPLACE VIEW v_orphan_memberships AS
SELECT 
    m.id,
    m.user_id,
    m.team_id,
    m.role,
    m.created_at,
    CASE 
        WHEN u.id IS NULL OR u.deleted_at IS NOT NULL THEN 'missing_user'
        WHEN t.id IS NULL OR t.deleted_at IS NOT NULL THEN 'missing_team'
    END as orphan_type
FROM memberships m
LEFT JOIN users u ON u.id = m.user_id
LEFT JOIN teams t ON t.id = m.team_id
WHERE m.status = 'active'
AND (u.id IS NULL OR u.deleted_at IS NOT NULL OR t.id IS NULL OR t.deleted_at IS NOT NULL);

-- View: Orphaned agents (missing delegating user/agent)
CREATE OR REPLACE VIEW v_orphan_agents AS
SELECT 
    a.id,
    a.name,
    a.delegated_by_user_id,
    a.delegated_by_agent_id,
    a.created_at,
    CASE 
        WHEN a.delegated_by_user_id IS NOT NULL AND (u.id IS NULL OR u.deleted_at IS NOT NULL) 
            THEN 'missing_delegating_user'
        WHEN a.delegated_by_agent_id IS NOT NULL AND (ag.id IS NULL OR ag.deleted_at IS NOT NULL) 
            THEN 'missing_delegating_agent'
    END as orphan_type
FROM agents a
LEFT JOIN users u ON u.id = a.delegated_by_user_id
LEFT JOIN agents ag ON ag.id = a.delegated_by_agent_id
WHERE a.deleted_at IS NULL
AND a.status = 'active'
AND (
    (a.delegated_by_user_id IS NOT NULL AND (u.id IS NULL OR u.deleted_at IS NOT NULL))
    OR 
    (a.delegated_by_agent_id IS NOT NULL AND (ag.id IS NULL OR ag.deleted_at IS NOT NULL))
);

-- Function: Count all orphans
CREATE OR REPLACE FUNCTION count_orphans()
RETURNS TABLE (
    entity_type TEXT,
    orphan_count BIGINT
) AS $$
BEGIN
    RETURN QUERY
    SELECT 'organization'::TEXT, COUNT(*)::BIGINT FROM v_orphan_organizations
    UNION ALL
    SELECT 'team'::TEXT, COUNT(*)::BIGINT FROM v_orphan_teams
    UNION ALL
    SELECT 'project'::TEXT, COUNT(*)::BIGINT FROM v_orphan_projects
    UNION ALL
    SELECT 'membership'::TEXT, COUNT(*)::BIGINT FROM v_orphan_memberships
    UNION ALL
    SELECT 'agent'::TEXT, COUNT(*)::BIGINT FROM v_orphan_agents;
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- 13.3.4: Auto-Repair Functions
-- Repair orphaned records automatically
-- ============================================================================

-- Function: Auto-repair orphaned memberships
CREATE OR REPLACE FUNCTION auto_repair_orphan_memberships(
    dry_run BOOLEAN DEFAULT true
)
RETURNS TABLE (
    membership_id UUID,
    action_taken TEXT
) AS $$
DECLARE
    rec RECORD;
BEGIN
    FOR rec IN SELECT * FROM v_orphan_memberships
    LOOP
        IF NOT dry_run THEN
            UPDATE memberships 
            SET status = 'inactive'
            WHERE id = rec.id;
        END IF;
        
        membership_id := rec.id;
        action_taken := format('Set status to inactive (%s)', rec.orphan_type);
        RETURN NEXT;
    END LOOP;
END;
$$ LANGUAGE plpgsql;

-- Function: Auto-repair orphaned agents
CREATE OR REPLACE FUNCTION auto_repair_orphan_agents(
    dry_run BOOLEAN DEFAULT true
)
RETURNS TABLE (
    agent_id UUID,
    action_taken TEXT
) AS $$
DECLARE
    rec RECORD;
BEGIN
    FOR rec IN SELECT * FROM v_orphan_agents
    LOOP
        IF NOT dry_run THEN
            UPDATE agents 
            SET status = 'revoked',
                revoked_reason = format('Orphaned: %s', rec.orphan_type)
            WHERE id = rec.id;
        END IF;
        
        agent_id := rec.id;
        action_taken := format('Set status to revoked (%s)', rec.orphan_type);
        RETURN NEXT;
    END LOOP;
END;
$$ LANGUAGE plpgsql;

-- Function: Full referential integrity check and repair
CREATE OR REPLACE FUNCTION check_and_repair_referential_integrity(
    dry_run BOOLEAN DEFAULT true
)
RETURNS TABLE (
    check_name TEXT,
    issue_count BIGINT,
    repaired_count BIGINT
) AS $$
DECLARE
    v_count BIGINT;
    v_repaired BIGINT;
BEGIN
    -- Check orphaned organizations
    SELECT COUNT(*) INTO v_count FROM v_orphan_organizations;
    check_name := 'orphan_organizations';
    issue_count := v_count;
    repaired_count := 0;
    RETURN NEXT;
    
    -- Check orphaned teams
    SELECT COUNT(*) INTO v_count FROM v_orphan_teams;
    check_name := 'orphan_teams';
    issue_count := v_count;
    repaired_count := 0;
    RETURN NEXT;
    
    -- Check orphaned projects
    SELECT COUNT(*) INTO v_count FROM v_orphan_projects;
    check_name := 'orphan_projects';
    issue_count := v_count;
    repaired_count := 0;
    RETURN NEXT;
    
    -- Check and repair orphaned memberships
    SELECT COUNT(*) INTO v_count FROM v_orphan_memberships;
    SELECT COUNT(*) INTO v_repaired FROM auto_repair_orphan_memberships(dry_run);
    check_name := 'orphan_memberships';
    issue_count := v_count;
    repaired_count := v_repaired;
    RETURN NEXT;
    
    -- Check and repair orphaned agents
    SELECT COUNT(*) INTO v_count FROM v_orphan_agents;
    SELECT COUNT(*) INTO v_repaired FROM auto_repair_orphan_agents(dry_run);
    check_name := 'orphan_agents';
    issue_count := v_count;
    repaired_count := v_repaired;
    RETURN NEXT;
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- 13.3.5: Migration Script for Existing Data
-- Run integrity check on existing data
-- ============================================================================

-- Log current orphan counts before migration
DO $$
DECLARE
    rec RECORD;
BEGIN
    RAISE NOTICE 'Running referential integrity check before migration...';
    
    FOR rec IN SELECT * FROM count_orphans()
    LOOP
        RAISE NOTICE 'Found % orphaned %', rec.orphan_count, rec.entity_type;
    END LOOP;
END;
$$;

-- ============================================================================
-- 13.3.6: Indexes for Referential Integrity Checks
-- Optimize orphan detection queries
-- ============================================================================

-- Indexes for foreign key columns (if not already present)
CREATE INDEX IF NOT EXISTS idx_orgs_company_not_deleted 
    ON organizations(company_id) 
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_teams_org_not_deleted 
    ON teams(org_id) 
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_projects_team_not_deleted 
    ON projects(team_id) 
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_memberships_user_status 
    ON memberships(user_id, status) 
    WHERE status = 'active';

CREATE INDEX IF NOT EXISTS idx_memberships_team_status 
    ON memberships(team_id, status) 
    WHERE status = 'active';

-- ============================================================================
-- Comments
-- ============================================================================

COMMENT ON VIEW v_orphan_organizations IS 'Organizations with missing or deleted parent legacy-root rows';
COMMENT ON VIEW v_orphan_teams IS 'Teams with missing or deleted parent organizations';
COMMENT ON VIEW v_orphan_projects IS 'Projects with missing or deleted parent teams';
COMMENT ON VIEW v_orphan_memberships IS 'Memberships with missing user or team';
COMMENT ON VIEW v_orphan_agents IS 'Agents with missing delegating user/agent';
COMMENT ON FUNCTION count_orphans() IS 'Count all orphaned records in the system';
COMMENT ON FUNCTION check_and_repair_referential_integrity(BOOLEAN) IS 'Check and optionally repair referential integrity issues';
