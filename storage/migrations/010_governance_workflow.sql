-- Governance Workflow Schema
-- This migration creates the tables required for the approval workflow system
-- including governance configs, approval requests, and workflow states.

-- ============================================================================
-- GOVERNANCE CONFIGS TABLE
-- Per-scope governance configuration (legacy-root/org/team/project level)
-- ============================================================================
CREATE TABLE IF NOT EXISTS governance_configs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Scope (only one should be set)
    company_id UUID REFERENCES companies(id) ON DELETE CASCADE,
    org_id UUID REFERENCES organizations(id) ON DELETE CASCADE,
    team_id UUID REFERENCES teams(id) ON DELETE CASCADE,
    project_id UUID REFERENCES projects(id) ON DELETE CASCADE,
    -- Approval settings
    approval_mode TEXT NOT NULL DEFAULT 'quorum',  -- 'single', 'quorum', 'unanimous'
    min_approvers INT NOT NULL DEFAULT 2,
    timeout_hours INT NOT NULL DEFAULT 72,
    auto_approve_low_risk BOOLEAN NOT NULL DEFAULT false,
    -- Escalation settings
    escalation_enabled BOOLEAN NOT NULL DEFAULT true,
    escalation_timeout_hours INT NOT NULL DEFAULT 48,
    escalation_contact TEXT,
    -- Request type settings (JSON for flexibility)
    policy_settings JSONB NOT NULL DEFAULT '{"require_approval": true, "min_approvers": 2}',
    knowledge_settings JSONB NOT NULL DEFAULT '{"require_approval": true, "min_approvers": 1}',
    memory_settings JSONB NOT NULL DEFAULT '{"require_approval": false, "auto_approve_threshold": 0.8}',
    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_by UUID,
    -- Ensure only one scope is set
    CHECK (
        (company_id IS NOT NULL)::int +
        (org_id IS NOT NULL)::int +
        (team_id IS NOT NULL)::int +
        (project_id IS NOT NULL)::int = 1
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_governance_configs_company ON governance_configs(company_id) WHERE company_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_governance_configs_org ON governance_configs(org_id) WHERE org_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_governance_configs_team ON governance_configs(team_id) WHERE team_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_governance_configs_project ON governance_configs(project_id) WHERE project_id IS NOT NULL;

-- ============================================================================
-- APPROVAL REQUESTS TABLE
-- Tracks all pending and historical approval requests
-- ============================================================================
CREATE TABLE IF NOT EXISTS approval_requests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Request identification
    request_number TEXT NOT NULL UNIQUE,  -- Human-readable ID like 'REQ-2024-0001'
    -- Request type and target
    request_type TEXT NOT NULL,  -- 'policy', 'knowledge', 'memory', 'role', 'config'
    target_type TEXT NOT NULL,   -- 'cedar_policy', 'adr', 'memory_promotion', etc.
    target_id TEXT,              -- ID of the thing being created/modified
    -- Scope
    company_id UUID REFERENCES companies(id),
    org_id UUID REFERENCES organizations(id),
    team_id UUID REFERENCES teams(id),
    project_id UUID REFERENCES projects(id),
    -- Request details
    title TEXT NOT NULL,
    description TEXT,
    payload JSONB NOT NULL,      -- The actual content being requested (policy, knowledge item, etc.)
    risk_level TEXT NOT NULL DEFAULT 'medium',  -- 'low', 'medium', 'high', 'critical'
    -- Requestor
    requestor_type TEXT NOT NULL,  -- 'user', 'agent'
    requestor_id UUID NOT NULL,
    requestor_email TEXT,
    -- Approval requirements
    required_approvals INT NOT NULL DEFAULT 2,
    current_approvals INT NOT NULL DEFAULT 0,
    -- Status
    status TEXT NOT NULL DEFAULT 'pending',  -- 'pending', 'approved', 'rejected', 'expired', 'cancelled'
    -- Timing
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ,
    resolved_at TIMESTAMPTZ,
    -- Resolution details
    resolution_reason TEXT,
    applied_at TIMESTAMPTZ,      -- When the change was actually applied
    applied_by UUID
);

CREATE INDEX IF NOT EXISTS idx_approval_requests_status ON approval_requests(status) WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS idx_approval_requests_requestor ON approval_requests(requestor_type, requestor_id);
CREATE INDEX IF NOT EXISTS idx_approval_requests_company ON approval_requests(company_id);
CREATE INDEX IF NOT EXISTS idx_approval_requests_org ON approval_requests(org_id);
CREATE INDEX IF NOT EXISTS idx_approval_requests_team ON approval_requests(team_id);
CREATE INDEX IF NOT EXISTS idx_approval_requests_created ON approval_requests(created_at DESC);

-- ============================================================================
-- APPROVAL DECISIONS TABLE
-- Individual approve/reject decisions on requests
-- ============================================================================
CREATE TABLE IF NOT EXISTS approval_decisions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    request_id UUID NOT NULL REFERENCES approval_requests(id) ON DELETE CASCADE,
    -- Decision maker
    approver_type TEXT NOT NULL,  -- 'user', 'agent', 'system'
    approver_id UUID NOT NULL,
    approver_email TEXT,
    -- Decision
    decision TEXT NOT NULL,  -- 'approve', 'reject', 'abstain'
    comment TEXT,
    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (request_id, approver_id)  -- One decision per approver per request
);

CREATE INDEX IF NOT EXISTS idx_approval_decisions_request ON approval_decisions(request_id);
CREATE INDEX IF NOT EXISTS idx_approval_decisions_approver ON approval_decisions(approver_id);

-- ============================================================================
-- GOVERNANCE ROLES TABLE
-- Role assignments for governance purposes (extends memberships)
-- ============================================================================
CREATE TABLE IF NOT EXISTS governance_roles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Principal
    principal_type TEXT NOT NULL,  -- 'user', 'agent'
    principal_id UUID NOT NULL,
    -- Role
    role TEXT NOT NULL,  -- 'governance_admin', 'approver', 'auditor'
    -- Scope (only one should be set)
    company_id UUID REFERENCES companies(id) ON DELETE CASCADE,
    org_id UUID REFERENCES organizations(id) ON DELETE CASCADE,
    team_id UUID REFERENCES teams(id) ON DELETE CASCADE,
    project_id UUID REFERENCES projects(id) ON DELETE CASCADE,
    -- Validity
    granted_by UUID NOT NULL,
    granted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    revoked_by UUID,
    -- Constraints
    CHECK (
        (company_id IS NOT NULL)::int +
        (org_id IS NOT NULL)::int +
        (team_id IS NOT NULL)::int +
        (project_id IS NOT NULL)::int = 1
    )
);

CREATE INDEX IF NOT EXISTS idx_governance_roles_principal ON governance_roles(principal_type, principal_id) WHERE revoked_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_governance_roles_company ON governance_roles(company_id) WHERE company_id IS NOT NULL AND revoked_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_governance_roles_org ON governance_roles(org_id) WHERE org_id IS NOT NULL AND revoked_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_governance_roles_team ON governance_roles(team_id) WHERE team_id IS NOT NULL AND revoked_at IS NULL;

-- ============================================================================
-- GOVERNANCE AUDIT LOG TABLE
-- Tracks all governance-related actions for compliance
-- ============================================================================
CREATE TABLE IF NOT EXISTS governance_audit_log (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- What happened
    action TEXT NOT NULL,  -- 'request_created', 'approved', 'rejected', 'escalated', 'expired', 'applied', 'config_changed'
    -- Target
    request_id UUID REFERENCES approval_requests(id),
    target_type TEXT,
    target_id TEXT,
    -- Actor
    actor_type TEXT NOT NULL,  -- 'user', 'agent', 'system'
    actor_id UUID,
    actor_email TEXT,
    -- Details
    details JSONB NOT NULL DEFAULT '{}',
    old_values JSONB,
    new_values JSONB,
    -- Context
    ip_address INET,
    user_agent TEXT,
    -- Timestamp
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_governance_audit_request ON governance_audit_log(request_id);
CREATE INDEX IF NOT EXISTS idx_governance_audit_action ON governance_audit_log(action);
CREATE INDEX IF NOT EXISTS idx_governance_audit_actor ON governance_audit_log(actor_type, actor_id);
CREATE INDEX IF NOT EXISTS idx_governance_audit_created ON governance_audit_log(created_at DESC);

-- ============================================================================
-- ESCALATION QUEUE TABLE
-- Tracks escalated requests
-- ============================================================================
CREATE TABLE IF NOT EXISTS escalation_queue (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    request_id UUID NOT NULL REFERENCES approval_requests(id) ON DELETE CASCADE,
    -- Escalation level (1 = first escalation, 2 = second, etc.)
    level INT NOT NULL DEFAULT 1,
    -- Escalation targets
    escalated_to JSONB NOT NULL,  -- Array of user/team IDs to notify
    -- Status
    notified_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    responded_at TIMESTAMPTZ,
    response TEXT,  -- 'approved', 'rejected', 'further_escalated'
    UNIQUE (request_id, level)
);

CREATE INDEX IF NOT EXISTS idx_escalation_queue_pending ON escalation_queue(request_id) WHERE responded_at IS NULL;

-- ============================================================================
-- HELPER FUNCTIONS
-- ============================================================================

-- Generate human-readable request number
CREATE OR REPLACE FUNCTION generate_request_number()
RETURNS TEXT AS $$
DECLARE
    year_part TEXT;
    seq_num INT;
    result TEXT;
BEGIN
    year_part := to_char(NOW(), 'YYYY');
    
    SELECT COALESCE(MAX(
        CAST(SUBSTRING(request_number FROM 'REQ-' || year_part || '-(\d+)') AS INT)
    ), 0) + 1
    INTO seq_num
    FROM approval_requests
    WHERE request_number LIKE 'REQ-' || year_part || '-%';
    
    result := 'REQ-' || year_part || '-' || LPAD(seq_num::TEXT, 4, '0');
    RETURN result;
END;
$$ LANGUAGE plpgsql;

-- Get effective governance config for a scope (with inheritance)
CREATE OR REPLACE FUNCTION get_effective_governance_config(
    p_company_id UUID DEFAULT NULL,
    p_org_id UUID DEFAULT NULL,
    p_team_id UUID DEFAULT NULL,
    p_project_id UUID DEFAULT NULL
)
RETURNS TABLE (
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
    -- Try project level first
    IF p_project_id IS NOT NULL THEN
        RETURN QUERY
        SELECT gc.id, 'project'::TEXT, gc.approval_mode, gc.min_approvers, gc.timeout_hours,
               gc.auto_approve_low_risk, gc.escalation_enabled, gc.escalation_timeout_hours,
               gc.escalation_contact, gc.policy_settings, gc.knowledge_settings, gc.memory_settings
        FROM governance_configs gc
        WHERE gc.project_id = p_project_id;
        IF FOUND THEN RETURN; END IF;
    END IF;
    
    -- Try team level
    IF p_team_id IS NOT NULL THEN
        RETURN QUERY
        SELECT gc.id, 'team'::TEXT, gc.approval_mode, gc.min_approvers, gc.timeout_hours,
               gc.auto_approve_low_risk, gc.escalation_enabled, gc.escalation_timeout_hours,
               gc.escalation_contact, gc.policy_settings, gc.knowledge_settings, gc.memory_settings
        FROM governance_configs gc
        WHERE gc.team_id = p_team_id;
        IF FOUND THEN RETURN; END IF;
    END IF;
    
    -- Try org level
    IF p_org_id IS NOT NULL THEN
        RETURN QUERY
        SELECT gc.id, 'org'::TEXT, gc.approval_mode, gc.min_approvers, gc.timeout_hours,
               gc.auto_approve_low_risk, gc.escalation_enabled, gc.escalation_timeout_hours,
               gc.escalation_contact, gc.policy_settings, gc.knowledge_settings, gc.memory_settings
        FROM governance_configs gc
        WHERE gc.org_id = p_org_id;
        IF FOUND THEN RETURN; END IF;
    END IF;
    
    -- Try legacy root level
    IF p_company_id IS NOT NULL THEN
        RETURN QUERY
        SELECT gc.id, 'company'::TEXT, gc.approval_mode, gc.min_approvers, gc.timeout_hours,
               gc.auto_approve_low_risk, gc.escalation_enabled, gc.escalation_timeout_hours,
               gc.escalation_contact, gc.policy_settings, gc.knowledge_settings, gc.memory_settings
        FROM governance_configs gc
        WHERE gc.company_id = p_company_id;
        IF FOUND THEN RETURN; END IF;
    END IF;
    
    -- Return defaults if no config found
    RETURN QUERY
    SELECT NULL::UUID, 'default'::TEXT, 'quorum'::TEXT, 2, 72,
           false, true, 48, NULL::TEXT,
           '{"require_approval": true, "min_approvers": 2}'::JSONB,
           '{"require_approval": true, "min_approvers": 1}'::JSONB,
           '{"require_approval": false, "auto_approve_threshold": 0.8}'::JSONB;
END;
$$ LANGUAGE plpgsql;

-- Trigger to auto-set request_number on insert
CREATE OR REPLACE FUNCTION set_request_number()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.request_number IS NULL THEN
        NEW.request_number := generate_request_number();
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trigger_set_request_number
    BEFORE INSERT ON approval_requests
    FOR EACH ROW
    EXECUTE FUNCTION set_request_number();

-- Trigger to update approval count
CREATE OR REPLACE FUNCTION update_approval_count()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.decision = 'approve' THEN
        UPDATE approval_requests
        SET current_approvals = current_approvals + 1,
            updated_at = NOW()
        WHERE id = NEW.request_id;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trigger_update_approval_count
    AFTER INSERT ON approval_decisions
    FOR EACH ROW
    EXECUTE FUNCTION update_approval_count();

-- Trigger to auto-approve when threshold met
CREATE OR REPLACE FUNCTION check_auto_approve()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.current_approvals >= NEW.required_approvals AND NEW.status = 'pending' THEN
        NEW.status := 'approved';
        NEW.resolved_at := NOW();
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trigger_check_auto_approve
    BEFORE UPDATE ON approval_requests
    FOR EACH ROW
    WHEN (OLD.current_approvals != NEW.current_approvals)
    EXECUTE FUNCTION check_auto_approve();

-- ============================================================================
-- VIEWS FOR COMMON QUERIES
-- ============================================================================

-- Pending requests with requestor info
CREATE OR REPLACE VIEW v_pending_requests AS
SELECT
    ar.id,
    ar.request_number,
    ar.request_type,
    ar.target_type,
    ar.title,
    ar.description,
    ar.risk_level,
    ar.requestor_type,
    ar.requestor_id,
    COALESCE(ar.requestor_email, u.email) as requestor_email,
    COALESCE(u.name, a.name) as requestor_name,
    ar.required_approvals,
    ar.current_approvals,
    ar.status,
    ar.created_at,
    ar.expires_at,
    ar.company_id,
    ar.org_id,
    ar.team_id,
    ar.project_id
FROM approval_requests ar
LEFT JOIN users u ON ar.requestor_type = 'user' AND ar.requestor_id = u.id
LEFT JOIN agents a ON ar.requestor_type = 'agent' AND ar.requestor_id = a.id
WHERE ar.status = 'pending';

-- Governance audit with actor info
CREATE OR REPLACE VIEW v_governance_audit AS
SELECT
    gal.id,
    gal.action,
    gal.request_id,
    ar.request_number,
    ar.title as request_title,
    gal.target_type,
    gal.target_id,
    gal.actor_type,
    gal.actor_id,
    COALESCE(gal.actor_email, u.email) as actor_email,
    COALESCE(u.name, a.name) as actor_name,
    gal.details,
    gal.created_at
FROM governance_audit_log gal
LEFT JOIN approval_requests ar ON gal.request_id = ar.id
LEFT JOIN users u ON gal.actor_type = 'user' AND gal.actor_id = u.id
LEFT JOIN agents a ON gal.actor_type = 'agent' AND gal.actor_id = a.id
ORDER BY gal.created_at DESC;

-- ============================================================================
-- NOTIFY TRIGGERS FOR REAL-TIME UPDATES
-- ============================================================================
CREATE OR REPLACE FUNCTION notify_governance_change()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify(
        'governance_changes',
        json_build_object(
            'table', TG_TABLE_NAME,
            'operation', TG_OP,
            'id', COALESCE(NEW.id, OLD.id)::TEXT,
            'request_id', CASE 
                WHEN TG_TABLE_NAME = 'approval_requests' THEN COALESCE(NEW.id, OLD.id)::TEXT
                WHEN TG_TABLE_NAME = 'approval_decisions' THEN COALESCE(NEW.request_id, OLD.request_id)::TEXT
                ELSE NULL
            END,
            'timestamp', extract(epoch from now())
        )::TEXT
    );
    RETURN COALESCE(NEW, OLD);
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER notify_approval_requests
    AFTER INSERT OR UPDATE ON approval_requests
    FOR EACH ROW
    EXECUTE FUNCTION notify_governance_change();

CREATE TRIGGER notify_approval_decisions
    AFTER INSERT ON approval_decisions
    FOR EACH ROW
    EXECUTE FUNCTION notify_governance_change();

-- ============================================================================
-- COMMENTS
-- ============================================================================
COMMENT ON TABLE governance_configs IS 'Per-scope governance configuration with inheritance';
COMMENT ON TABLE approval_requests IS 'All approval requests (pending, approved, rejected, expired)';
COMMENT ON TABLE approval_decisions IS 'Individual approval/rejection decisions';
COMMENT ON TABLE governance_roles IS 'Governance-specific role assignments';
COMMENT ON TABLE governance_audit_log IS 'Compliance audit trail for all governance actions';
COMMENT ON TABLE escalation_queue IS 'Tracks escalated requests and their resolution';
COMMENT ON FUNCTION get_effective_governance_config IS 'Returns the effective config with inheritance chain';
