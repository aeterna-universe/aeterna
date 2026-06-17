-- Organizational Referential Schema for OPAL/Cedar Integration
-- This migration creates the normalized entity tables for the organizational hierarchy
-- that will be consumed by OPAL data fetchers and Cedar authorization policies.

-- ============================================================================
-- LEGACY ROOT TABLE
-- Root of the original organizational hierarchy. This pre-tenant design
-- treated each root row as its own tenant.
-- ============================================================================
CREATE TABLE IF NOT EXISTS companies (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug TEXT NOT NULL UNIQUE,  -- URL-friendly identifier (e.g., 'acme-corp')
    name TEXT NOT NULL,         -- Display name (e.g., 'Acme Corporation')
    settings JSONB NOT NULL DEFAULT '{}',  -- Root-scope settings
    -- Governance settings
    governance_mode TEXT NOT NULL DEFAULT 'standard',  -- 'permissive', 'standard', 'strict'
    default_approval_required BOOLEAN NOT NULL DEFAULT true,
    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ  -- Soft delete
);

CREATE INDEX IF NOT EXISTS idx_companies_slug ON companies(slug) WHERE deleted_at IS NULL;

-- ============================================================================
-- ORGANIZATIONS TABLE
-- Divisions/departments within a legacy root row (e.g., 'Platform Engineering')
-- ============================================================================
CREATE TABLE IF NOT EXISTS organizations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id UUID NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    slug TEXT NOT NULL,         -- Unique within legacy root row
    name TEXT NOT NULL,
    settings JSONB NOT NULL DEFAULT '{}',
    -- Governance overrides (inherit from legacy root scope if null)
    governance_mode TEXT,
    approval_required BOOLEAN,
    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ,
    UNIQUE (company_id, slug)
);

CREATE INDEX IF NOT EXISTS idx_organizations_company ON organizations(company_id) WHERE deleted_at IS NULL;

-- ============================================================================
-- TEAMS TABLE
-- Working groups within an organization (e.g., 'API Team', 'Data Platform')
-- ============================================================================
CREATE TABLE IF NOT EXISTS teams (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    slug TEXT NOT NULL,         -- Unique within organization
    name TEXT NOT NULL,
    settings JSONB NOT NULL DEFAULT '{}',
    -- Governance overrides
    governance_mode TEXT,
    approval_required BOOLEAN,
    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ,
    UNIQUE (org_id, slug)
);

CREATE INDEX IF NOT EXISTS idx_teams_org ON teams(org_id) WHERE deleted_at IS NULL;

-- ============================================================================
-- PROJECTS TABLE
-- Repositories/codebases owned by a team
-- ============================================================================
CREATE TABLE IF NOT EXISTS projects (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    team_id UUID NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    slug TEXT NOT NULL,         -- Unique within team
    name TEXT NOT NULL,
    -- Git integration
    git_remote TEXT,            -- e.g., 'git@github.com:acme/payments.git'
    git_branch TEXT DEFAULT 'main',
    -- Settings
    settings JSONB NOT NULL DEFAULT '{}',
    governance_mode TEXT,
    approval_required BOOLEAN,
    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ,
    UNIQUE (team_id, slug)
);

CREATE INDEX IF NOT EXISTS idx_projects_team ON projects(team_id) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_projects_git_remote ON projects(git_remote) WHERE git_remote IS NOT NULL AND deleted_at IS NULL;

-- ============================================================================
-- USERS TABLE
-- Human identities with IdP integration
-- ============================================================================
CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email TEXT NOT NULL UNIQUE,
    name TEXT,
    avatar_url TEXT,
    -- IdP integration
    idp_provider TEXT,          -- 'okta', 'azure_ad', 'google', 'github', null for local
    idp_subject TEXT,           -- Subject claim from IdP token
    -- Status
    status TEXT NOT NULL DEFAULT 'active',  -- 'active', 'inactive', 'suspended'
    last_login_at TIMESTAMPTZ,
    -- Settings
    settings JSONB NOT NULL DEFAULT '{}',
    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ,
    UNIQUE (idp_provider, idp_subject)
);

CREATE INDEX IF NOT EXISTS idx_users_email ON users(email) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_users_idp ON users(idp_provider, idp_subject) WHERE idp_provider IS NOT NULL AND deleted_at IS NULL;

-- ============================================================================
-- AGENTS TABLE
-- AI agent identities with delegation chains
-- ============================================================================
CREATE TABLE IF NOT EXISTS agents (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    agent_type TEXT NOT NULL,   -- 'opencode', 'langchain', 'autogen', 'custom'
    -- Delegation chain (who authorized this agent)
    delegated_by_user_id UUID REFERENCES users(id),
    delegated_by_agent_id UUID REFERENCES agents(id),
    delegation_depth INT NOT NULL DEFAULT 1,  -- How many hops from human
    max_delegation_depth INT NOT NULL DEFAULT 3,  -- Max allowed hops
    -- Capabilities (what this agent can do)
    capabilities JSONB NOT NULL DEFAULT '[]',  -- ['memory:read', 'memory:write', 'knowledge:read']
    -- Scopes (where this agent can operate)
    allowed_company_ids UUID[] DEFAULT '{}',
    allowed_org_ids UUID[] DEFAULT '{}',
    allowed_team_ids UUID[] DEFAULT '{}',
    allowed_project_ids UUID[] DEFAULT '{}',
    -- Status
    status TEXT NOT NULL DEFAULT 'active',  -- 'active', 'revoked', 'expired'
    expires_at TIMESTAMPTZ,
    -- Audit
    last_activity_at TIMESTAMPTZ,
    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ,
    -- Ensure delegation chain is valid
    CHECK (
        (delegated_by_user_id IS NOT NULL AND delegated_by_agent_id IS NULL) OR
        (delegated_by_user_id IS NULL AND delegated_by_agent_id IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_agents_delegated_by_user ON agents(delegated_by_user_id) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_agents_delegated_by_agent ON agents(delegated_by_agent_id) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_agents_status ON agents(status) WHERE deleted_at IS NULL;

-- ============================================================================
-- MEMBERSHIPS TABLE
-- User ↔ Team relationships with roles
-- ============================================================================
CREATE TABLE IF NOT EXISTS memberships (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    team_id UUID NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'developer',  -- 'admin', 'architect', 'tech_lead', 'developer', 'viewer'
    -- Permissions granted through this membership
    permissions JSONB NOT NULL DEFAULT '[]',
    -- Status
    status TEXT NOT NULL DEFAULT 'active',  -- 'active', 'pending', 'inactive'
    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (user_id, team_id)
);

CREATE INDEX IF NOT EXISTS idx_memberships_user ON memberships(user_id);
CREATE INDEX IF NOT EXISTS idx_memberships_team ON memberships(team_id);
CREATE INDEX IF NOT EXISTS idx_memberships_role ON memberships(role);

-- ============================================================================
-- GIT REMOTE PATTERNS TABLE
-- Patterns for auto-detecting project from git remote
-- ============================================================================
CREATE TABLE IF NOT EXISTS git_remote_patterns (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id UUID NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    pattern TEXT NOT NULL,      -- Regex pattern, e.g., '^git@github\.com:acme/(.+)\.git$'
    org_slug TEXT,              -- Optional: auto-assign to this org
    team_slug TEXT,             -- Optional: auto-assign to this team
    priority INT NOT NULL DEFAULT 0,  -- Higher priority patterns match first
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_git_remote_patterns_company ON git_remote_patterns(company_id) WHERE enabled = true;

-- ============================================================================
-- EMAIL DOMAIN PATTERNS TABLE
-- Patterns for auto-detecting company from email domain
-- ============================================================================
CREATE TABLE IF NOT EXISTS email_domain_patterns (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_id UUID NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    domain TEXT NOT NULL,       -- e.g., 'acme.com', '*.acme.com'
    auto_create_user BOOLEAN NOT NULL DEFAULT false,
    default_org_id UUID REFERENCES organizations(id),
    default_team_id UUID REFERENCES teams(id),
    priority INT NOT NULL DEFAULT 0,
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (domain)
);

CREATE INDEX IF NOT EXISTS idx_email_domain_patterns_domain ON email_domain_patterns(domain) WHERE enabled = true;

-- ============================================================================
-- REFERENTIAL AUDIT LOG TABLE
-- Tracks all changes to organizational referential for compliance
-- ============================================================================
CREATE TABLE IF NOT EXISTS referential_audit_log (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- What changed
    entity_type TEXT NOT NULL,  -- 'company', 'organization', 'team', 'project', 'user', 'agent', 'membership'
    entity_id UUID NOT NULL,
    action TEXT NOT NULL,       -- 'create', 'update', 'delete', 'restore'
    -- Change details
    old_values JSONB,
    new_values JSONB,
    changed_fields TEXT[],
    -- Who made the change
    actor_type TEXT NOT NULL,   -- 'user', 'agent', 'system', 'idp_sync'
    actor_id UUID,
    actor_email TEXT,
    -- Context
    ip_address INET,
    user_agent TEXT,
    request_id TEXT,
    -- Timestamp
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_audit_log_entity ON referential_audit_log(entity_type, entity_id);
CREATE INDEX IF NOT EXISTS idx_audit_log_actor ON referential_audit_log(actor_type, actor_id);
CREATE INDEX IF NOT EXISTS idx_audit_log_created ON referential_audit_log(created_at);

-- ============================================================================
-- OPAL VIEWS
-- Views optimized for OPAL data fetcher queries
-- ============================================================================

-- `v_hierarchy` and `v_user_permissions` were originally introduced here.
-- They are now owned by migration 028 (`028_tenant_scoped_hierarchy.sql`),
-- which adds the canonical `tenant_id` column and is the only remaining
-- definition in the repo. Keeping the historical SQL here would reintroduce
-- duplicate view ownership and drift the repository contract from the runtime
-- schema, so fresh installs rely on migration 028 for both views.

-- Agent permissions view for Cedar authorization
CREATE OR REPLACE VIEW v_agent_permissions AS
SELECT
    a.id as agent_id,
    a.name as agent_name,
    a.agent_type,
    a.delegated_by_user_id,
    a.delegated_by_agent_id,
    a.delegation_depth,
    a.capabilities,
    a.allowed_company_ids,
    a.allowed_org_ids,
    a.allowed_team_ids,
    a.allowed_project_ids,
    a.status as agent_status,
    -- Include delegating user info if available
    u.email as delegating_user_email,
    u.name as delegating_user_name
FROM agents a
LEFT JOIN users u ON u.id = a.delegated_by_user_id
WHERE a.deleted_at IS NULL AND a.status = 'active';

-- ============================================================================
-- TRIGGERS FOR UPDATED_AT
-- ============================================================================
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ language 'plpgsql';

-- Apply to all tables with updated_at
DO $$
DECLARE
    t TEXT;
BEGIN
    FOR t IN SELECT unnest(ARRAY['companies', 'organizations', 'teams', 'projects', 'users', 'agents', 'memberships', 'git_remote_patterns', 'email_domain_patterns'])
    LOOP
        EXECUTE format('DROP TRIGGER IF EXISTS update_%s_updated_at ON %s', t, t);
        EXECUTE format('CREATE TRIGGER update_%s_updated_at BEFORE UPDATE ON %s FOR EACH ROW EXECUTE FUNCTION update_updated_at_column()', t, t);
    END LOOP;
END;
$$;

-- ============================================================================
-- TRIGGERS FOR AUDIT LOG
-- ============================================================================
CREATE OR REPLACE FUNCTION log_referential_change()
RETURNS TRIGGER AS $$
DECLARE
    entity TEXT;
    action_type TEXT;
    old_json JSONB;
    new_json JSONB;
BEGIN
    entity := TG_TABLE_NAME;
    
    IF TG_OP = 'INSERT' THEN
        action_type := 'create';
        old_json := NULL;
        new_json := to_jsonb(NEW);
    ELSIF TG_OP = 'UPDATE' THEN
        action_type := 'update';
        old_json := to_jsonb(OLD);
        new_json := to_jsonb(NEW);
    ELSIF TG_OP = 'DELETE' THEN
        action_type := 'delete';
        old_json := to_jsonb(OLD);
        new_json := NULL;
    END IF;
    
    INSERT INTO referential_audit_log (
        entity_type,
        entity_id,
        action,
        old_values,
        new_values,
        actor_type,
        request_id
    ) VALUES (
        entity,
        COALESCE(NEW.id, OLD.id),
        action_type,
        old_json,
        new_json,
        'system',
        current_setting('app.request_id', true)
    );
    
    RETURN COALESCE(NEW, OLD);
END;
$$ language 'plpgsql';

-- Apply audit triggers to core tables
DO $$
DECLARE
    t TEXT;
BEGIN
    FOR t IN SELECT unnest(ARRAY['companies', 'organizations', 'teams', 'projects', 'users', 'agents', 'memberships'])
    LOOP
        EXECUTE format('DROP TRIGGER IF EXISTS audit_%s ON %s', t, t);
        EXECUTE format('CREATE TRIGGER audit_%s AFTER INSERT OR UPDATE OR DELETE ON %s FOR EACH ROW EXECUTE FUNCTION log_referential_change()', t, t);
    END LOOP;
END;
$$;

-- ============================================================================
-- NOTIFY TRIGGERS FOR OPAL REAL-TIME SYNC
-- ============================================================================
CREATE OR REPLACE FUNCTION notify_referential_change()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify(
        'referential_changes',
        json_build_object(
            'table', TG_TABLE_NAME,
            'operation', TG_OP,
            'id', COALESCE(NEW.id, OLD.id)::TEXT,
            'timestamp', extract(epoch from now())
        )::TEXT
    );
    RETURN COALESCE(NEW, OLD);
END;
$$ language 'plpgsql';

-- Apply notify triggers to all referential tables
DO $$
DECLARE
    t TEXT;
BEGIN
    FOR t IN SELECT unnest(ARRAY['companies', 'organizations', 'teams', 'projects', 'users', 'agents', 'memberships'])
    LOOP
        EXECUTE format('DROP TRIGGER IF EXISTS notify_%s ON %s', t, t);
        EXECUTE format('CREATE TRIGGER notify_%s AFTER INSERT OR UPDATE OR DELETE ON %s FOR EACH ROW EXECUTE FUNCTION notify_referential_change()', t, t);
    END LOOP;
END;
$$;

-- ============================================================================
-- MIGRATION FROM organizational_units (if exists)
-- This section migrates data from the legacy schema to the new normalized tables
-- ============================================================================

-- Migrate companies
INSERT INTO companies (id, slug, name, settings, created_at, updated_at)
SELECT 
    gen_random_uuid(),
    COALESCE(LOWER(REGEXP_REPLACE(name, '[^a-zA-Z0-9]', '-', 'g')), id),
    name,
    COALESCE(metadata, '{}'),
    to_timestamp(created_at / 1000),
    to_timestamp(updated_at / 1000)
FROM organizational_units
WHERE type = 'company'
ON CONFLICT (slug) DO NOTHING;

-- Note: Full migration of orgs/teams/projects would require additional logic
-- to rebuild the hierarchy. This is left as a manual step for existing deployments.

COMMENT ON TABLE companies IS 'Legacy root organizational entities from the pre-tenant hierarchy design.';
COMMENT ON TABLE organizations IS 'Divisions within a legacy root row (e.g., Platform Engineering).';
COMMENT ON TABLE teams IS 'Working groups within an organization.';
COMMENT ON TABLE projects IS 'Repositories owned by a team.';
COMMENT ON TABLE users IS 'Human identities with optional IdP integration.';
COMMENT ON TABLE agents IS 'AI agent identities with delegation chains.';
COMMENT ON TABLE memberships IS 'User-team relationships with roles.';
COMMENT ON VIEW v_hierarchy IS 'OPAL view: organizational hierarchy for Cedar entities.';
COMMENT ON VIEW v_user_permissions IS 'OPAL view: user permissions for Cedar authorization.';
COMMENT ON VIEW v_agent_permissions IS 'OPAL view: agent permissions for Cedar authorization.';
