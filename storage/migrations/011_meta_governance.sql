-- Meta-Governance Schema
-- Tables for meta-governance policies (who can govern) and human confirmation requests (agent action approval)

-- ============================================================================
-- META-GOVERNANCE POLICIES TABLE
-- Defines who can govern at each layer of the hierarchy
-- ============================================================================
CREATE TABLE IF NOT EXISTS meta_governance_policies (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    layer TEXT NOT NULL,
    scope_id UUID,
    min_role_for_governance TEXT NOT NULL,
    action_permissions JSONB NOT NULL DEFAULT '[]',
    agent_delegation JSONB NOT NULL,
    escalation_config JSONB NOT NULL,
    active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_by UUID NOT NULL,
    CONSTRAINT valid_layer CHECK (layer IN ('company', 'org', 'team', 'project')),
    CONSTRAINT valid_min_role CHECK (min_role_for_governance IN ('viewer', 'developer', 'techlead', 'architect', 'admin'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_meta_governance_layer_scope 
    ON meta_governance_policies(layer, scope_id) 
    WHERE scope_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_meta_governance_layer_default 
    ON meta_governance_policies(layer) 
    WHERE scope_id IS NULL AND active = true;

CREATE INDEX IF NOT EXISTS idx_meta_governance_active 
    ON meta_governance_policies(layer) 
    WHERE active = true;

-- ============================================================================
-- HUMAN CONFIRMATION REQUESTS TABLE
-- Tracks agent actions requiring human approval
-- ============================================================================
CREATE TABLE IF NOT EXISTS human_confirmation_requests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id UUID NOT NULL,
    action TEXT NOT NULL,
    action_description TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id TEXT,
    risk_level TEXT NOT NULL DEFAULT 'medium',
    confirmation_reason TEXT NOT NULL,
    agent_context JSONB NOT NULL DEFAULT '{}',
    authorized_approvers JSONB NOT NULL DEFAULT '[]',
    status TEXT NOT NULL DEFAULT 'pending',
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at TIMESTAMPTZ,
    resolved_by UUID,
    resolution_comment TEXT,
    CONSTRAINT valid_status CHECK (status IN ('pending', 'approved', 'denied', 'expired', 'cancelled')),
    CONSTRAINT valid_risk_level CHECK (risk_level IN ('low', 'medium', 'high', 'critical'))
);

CREATE INDEX IF NOT EXISTS idx_human_confirmation_pending 
    ON human_confirmation_requests(status, expires_at) 
    WHERE status = 'pending';

CREATE INDEX IF NOT EXISTS idx_human_confirmation_agent 
    ON human_confirmation_requests(agent_id);

CREATE INDEX IF NOT EXISTS idx_human_confirmation_approvers 
    ON human_confirmation_requests USING gin(authorized_approvers);

CREATE INDEX IF NOT EXISTS idx_human_confirmation_created 
    ON human_confirmation_requests(created_at DESC);

-- ============================================================================
-- TRIGGER: Auto-expire old confirmation requests
-- ============================================================================
CREATE OR REPLACE FUNCTION expire_confirmation_requests()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE human_confirmation_requests
    SET status = 'expired'
    WHERE status = 'pending' AND expires_at < NOW();
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- FUNCTION: Get effective meta-governance policy (with inheritance)
-- ============================================================================
CREATE OR REPLACE FUNCTION get_effective_meta_governance_policy(
    p_layer TEXT,
    p_scope_id UUID DEFAULT NULL
)
RETURNS TABLE (
    policy_id UUID,
    layer TEXT,
    scope_id UUID,
    min_role_for_governance TEXT,
    action_permissions JSONB,
    agent_delegation JSONB,
    escalation_config JSONB,
    is_default BOOLEAN
) AS $$
BEGIN
    IF p_scope_id IS NOT NULL THEN
        RETURN QUERY
        SELECT 
            mgp.id, 
            mgp.layer, 
            mgp.scope_id,
            mgp.min_role_for_governance,
            mgp.action_permissions,
            mgp.agent_delegation,
            mgp.escalation_config,
            false
        FROM meta_governance_policies mgp
        WHERE mgp.layer = p_layer 
          AND mgp.scope_id = p_scope_id 
          AND mgp.active = true
        ORDER BY mgp.updated_at DESC
        LIMIT 1;
        IF FOUND THEN RETURN; END IF;
    END IF;
    
    RETURN QUERY
    SELECT 
        mgp.id, 
        mgp.layer, 
        mgp.scope_id,
        mgp.min_role_for_governance,
        mgp.action_permissions,
        mgp.agent_delegation,
        mgp.escalation_config,
        true
    FROM meta_governance_policies mgp
    WHERE mgp.layer = p_layer 
      AND mgp.scope_id IS NULL 
      AND mgp.active = true
    ORDER BY mgp.updated_at DESC
    LIMIT 1;
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- FUNCTION: Check if principal can perform governance action
-- ============================================================================
CREATE OR REPLACE FUNCTION check_governance_authorization(
    p_layer TEXT,
    p_scope_id UUID,
    p_principal_type TEXT,
    p_principal_role TEXT,
    p_action TEXT,
    p_risk_level TEXT DEFAULT 'medium',
    p_delegation_depth INT DEFAULT NULL
)
RETURNS TABLE (
    allowed BOOLEAN,
    reason TEXT,
    requires_human_confirmation BOOLEAN
) AS $$
DECLARE
    v_policy RECORD;
    v_min_role TEXT;
    v_role_order TEXT[] := ARRAY['viewer', 'developer', 'techlead', 'architect', 'admin'];
    v_principal_rank INT;
    v_min_rank INT;
    v_agent_config JSONB;
    v_max_depth INT;
    v_autonomous_enabled BOOLEAN;
    v_confirmation_required JSONB;
BEGIN
    SELECT * INTO v_policy 
    FROM get_effective_meta_governance_policy(p_layer, p_scope_id) 
    LIMIT 1;
    
    IF v_policy IS NULL THEN
        RETURN QUERY SELECT true, 'No policy configured - allowing by default'::TEXT, false;
        RETURN;
    END IF;

    v_min_role := v_policy.min_role_for_governance;
    v_principal_rank := array_position(v_role_order, p_principal_role);
    v_min_rank := array_position(v_role_order, v_min_role);
    
    IF v_principal_rank IS NULL OR v_principal_rank < v_min_rank THEN
        RETURN QUERY SELECT 
            false, 
            format('Role %s is insufficient. Minimum required: %s', p_principal_role, v_min_role)::TEXT,
            false;
        RETURN;
    END IF;

    IF p_principal_type = 'agent' THEN
        v_agent_config := v_policy.agent_delegation;
        v_autonomous_enabled := (v_agent_config->>'autonomous_enabled')::BOOLEAN;
        
        IF NOT v_autonomous_enabled THEN
            RETURN QUERY SELECT 
                false, 
                format('Agents cannot act autonomously at %s layer', p_layer)::TEXT,
                true;
            RETURN;
        END IF;
        
        v_max_depth := (v_agent_config->>'max_delegation_depth')::INT;
        IF p_delegation_depth IS NOT NULL AND p_delegation_depth > v_max_depth THEN
            RETURN QUERY SELECT 
                false, 
                format('Delegation depth %s exceeds maximum %s for %s layer', 
                       p_delegation_depth, v_max_depth, p_layer)::TEXT,
                true;
            RETURN;
        END IF;
        
        v_confirmation_required := v_agent_config->'human_confirmation_required';
        IF v_confirmation_required @> to_jsonb(p_action) THEN
            RETURN QUERY SELECT true, 'Authorized with human confirmation required'::TEXT, true;
            RETURN;
        END IF;
    END IF;

    RETURN QUERY SELECT true, 'Authorized'::TEXT, false;
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- NOTIFY TRIGGERS FOR REAL-TIME UPDATES
-- ============================================================================
CREATE OR REPLACE FUNCTION notify_meta_governance_change()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify(
        'meta_governance_changes',
        json_build_object(
            'table', TG_TABLE_NAME,
            'operation', TG_OP,
            'id', COALESCE(NEW.id, OLD.id)::TEXT,
            'layer', CASE 
                WHEN TG_TABLE_NAME = 'meta_governance_policies' THEN COALESCE(NEW.layer, OLD.layer)
                ELSE NULL
            END,
            'timestamp', extract(epoch from now())
        )::TEXT
    );
    RETURN COALESCE(NEW, OLD);
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER notify_meta_governance_policies
    AFTER INSERT OR UPDATE OR DELETE ON meta_governance_policies
    FOR EACH ROW
    EXECUTE FUNCTION notify_meta_governance_change();

CREATE TRIGGER notify_human_confirmation_requests
    AFTER INSERT OR UPDATE ON human_confirmation_requests
    FOR EACH ROW
    EXECUTE FUNCTION notify_meta_governance_change();

-- ============================================================================
-- VIEWS
-- ============================================================================
CREATE OR REPLACE VIEW v_active_meta_governance_policies AS
SELECT
    mgp.id,
    mgp.layer,
    mgp.scope_id,
    mgp.min_role_for_governance,
    mgp.action_permissions,
    mgp.agent_delegation,
    mgp.escalation_config,
    mgp.created_at,
    mgp.updated_at,
    mgp.created_by,
    CASE 
        WHEN mgp.scope_id IS NULL THEN 'default'
        ELSE 'scoped'
    END as policy_type
FROM meta_governance_policies mgp
WHERE mgp.active = true
ORDER BY mgp.layer, mgp.scope_id NULLS FIRST;

CREATE OR REPLACE VIEW v_pending_human_confirmations AS
SELECT
    hcr.id,
    hcr.agent_id,
    a.name as agent_name,
    hcr.action,
    hcr.action_description,
    hcr.target_type,
    hcr.target_id,
    hcr.risk_level,
    hcr.confirmation_reason,
    hcr.authorized_approvers,
    hcr.expires_at,
    hcr.created_at,
    EXTRACT(EPOCH FROM (hcr.expires_at - NOW())) as seconds_until_expiry
FROM human_confirmation_requests hcr
LEFT JOIN agents a ON hcr.agent_id = a.id
WHERE hcr.status = 'pending' AND hcr.expires_at > NOW()
ORDER BY hcr.created_at DESC;

-- ============================================================================
-- COMMENTS
-- ============================================================================
COMMENT ON TABLE meta_governance_policies IS 'Defines who can govern at each layer (legacy-root/org/team/project)';
COMMENT ON TABLE human_confirmation_requests IS 'Agent actions requiring human approval before execution';
COMMENT ON FUNCTION get_effective_meta_governance_policy IS 'Returns the effective policy with scope-specific or default fallback';
COMMENT ON FUNCTION check_governance_authorization IS 'Checks if a principal can perform a governance action at a given layer';
