-- Migration 029: agents.tenant_id + v_agent_permissions relocation
--
-- Closes the second half of issue #130: agent-permissions isolation.
--
-- PR #131 added tenant-filtering to hierarchy and users endpoints in
-- opal-fetcher, but /v1/agents still returned the globally-merged agent
-- set across every tenant — because the `agents` table (migration 009)
-- has no `tenant_id` column. An agent's tenant was only knowable
-- transitively via the legacy root-scope UUID array -> the tenant link on
-- the legacy root table,
-- and that set can in principle span multiple tenants.
--
-- This migration establishes the invariant **one agent = one tenant**:
--
--   1. ADDs a nullable `agents.tenant_id UUID` column (FK -> tenants.id).
--   2. AUDITs existing agents for cross-tenant scope; ABORTs with a
--      loud RAISE if any agent's legacy root-scope UUID array spans
--      multiple tenants or if any active agent has no derivable tenant.
--      This is
--      deliberate: we do not want to pick a tenant for you, because a
--      silent wrong choice would be a durable authz bug.
--   3. BACKFILLs tenant_id from the distinct tenant implied by the
--      legacy root-scope UUID array (plus fallback derivations through
--      orgs / teams / projects for agents scoped at finer granularities).
--   4. SETs NOT NULL once the backfill is complete.
--   5. RELOCATEs the `v_agent_permissions` view from
--      `idp-sync::github::initialize_opal_views` into this migration
--      (same pattern as migration 028 for v_hierarchy and
--      v_user_permissions), surfacing tenant_id as the first column.
--      Fixes a latent bug: the idp-sync definition referenced
--      `u.display_name`, which does not exist in the users table
--      (migration 009 defines `users.name`). The relocated view uses
--      `u.name` correctly.
--
-- Operational note: if step 2 aborts in staging/prod, the offending
-- agents need to be manually split or re-scoped before retrying. The
-- error message lists every problem agent id so the fix is mechanical.
--
-- Follow-up #130 subtask remaining after this migration: relocate
-- idp-sync writes from `organizational_units` to the modern hierarchy
-- tables, then remove the legacy OU writes
-- from `PostgresBackend::initialize_schema` in the first migration that
-- lands the modern-table IdP sync writer (target milestone: the next
-- post-029 hierarchy migration, not another ad-hoc runtime override).

-- ---------------------------------------------------------------------
-- 1. Add column (nullable initially for backfill)
-- ---------------------------------------------------------------------

ALTER TABLE agents
    ADD COLUMN IF NOT EXISTS tenant_id UUID REFERENCES tenants(id);

-- ---------------------------------------------------------------------
-- 2. Audit: fail loud on cross-tenant agents
-- ---------------------------------------------------------------------

DO $$
DECLARE
    offenders TEXT;
BEGIN
    -- For each not-yet-backfilled active agent, compute the set of
    -- distinct tenants implied by its entire scope (legacy root rows + the
    -- roots reachable through orgs / teams / projects). An agent
    -- is an offender if that set has more than one element.
    SELECT string_agg(agent_id::text, ', ')
      INTO offenders
      FROM (
        SELECT a.id AS agent_id
        FROM agents a
        LEFT JOIN LATERAL (
            SELECT DISTINCT c.tenant_id
            FROM companies c
            WHERE c.deleted_at IS NULL
              AND (
                c.id = ANY(a.allowed_company_ids)
                OR c.id IN (
                    SELECT o.company_id FROM organizations o
                    WHERE o.deleted_at IS NULL
                      AND o.id = ANY(a.allowed_org_ids)
                )
                OR c.id IN (
                    SELECT o.company_id FROM organizations o
                    JOIN teams t ON t.org_id = o.id
                    WHERE o.deleted_at IS NULL
                      AND t.deleted_at IS NULL
                      AND t.id = ANY(a.allowed_team_ids)
                )
                OR c.id IN (
                    SELECT o.company_id FROM organizations o
                    JOIN teams t ON t.org_id = o.id
                    JOIN projects p ON p.team_id = t.id
                    WHERE o.deleted_at IS NULL
                      AND t.deleted_at IS NULL
                      AND p.deleted_at IS NULL
                      AND p.id = ANY(a.allowed_project_ids)
                )
              )
        ) tenants ON TRUE
        WHERE a.deleted_at IS NULL
          AND a.status <> 'revoked'
          AND a.tenant_id IS NULL
        GROUP BY a.id
        HAVING COUNT(DISTINCT tenants.tenant_id) > 1
      ) bad;

    IF offenders IS NOT NULL THEN
        RAISE EXCEPTION
            'Migration 029 abort: agents % have allowed_* scope spanning multiple tenants. Split or re-scope them to a single tenant before retrying (see docs/issue #130).',
            offenders;
    END IF;
END $$;

-- ---------------------------------------------------------------------
-- 3. Backfill tenant_id from the (now-verified-single) tenant of scope
-- ---------------------------------------------------------------------

UPDATE agents a SET tenant_id = sub.tenant_id
FROM (
    -- Postgres has no min(uuid) aggregate. Step 2's audit guarantees at most
-- one distinct tenant per agent, so any element of the array is correct.
SELECT a.id AS agent_id, (array_agg(c.tenant_id))[1] AS tenant_id
    FROM agents a
    JOIN companies c ON c.deleted_at IS NULL AND (
        c.id = ANY(a.allowed_company_ids)
        OR c.id IN (
            SELECT o.company_id FROM organizations o
            WHERE o.deleted_at IS NULL
              AND o.id = ANY(a.allowed_org_ids)
        )
        OR c.id IN (
            SELECT o.company_id FROM organizations o
            JOIN teams t ON t.org_id = o.id
            WHERE o.deleted_at IS NULL
              AND t.deleted_at IS NULL
              AND t.id = ANY(a.allowed_team_ids)
        )
        OR c.id IN (
            SELECT o.company_id FROM organizations o
            JOIN teams t ON t.org_id = o.id
            JOIN projects p ON p.team_id = t.id
            WHERE o.deleted_at IS NULL
              AND t.deleted_at IS NULL
              AND p.deleted_at IS NULL
              AND p.id = ANY(a.allowed_project_ids)
        )
    )
    WHERE a.deleted_at IS NULL
      AND a.tenant_id IS NULL
    GROUP BY a.id
) sub
WHERE a.id = sub.agent_id;

-- ---------------------------------------------------------------------
-- 4. Fail loud if any active agent still lacks a tenant after backfill
--    (e.g. agents with empty allowed_* arrays — truly-global agents
--    cannot exist under the one-agent-one-tenant invariant).
-- ---------------------------------------------------------------------

DO $$
DECLARE
    unscoped TEXT;
BEGIN
    SELECT string_agg(id::text, ', ') INTO unscoped
    FROM agents
    WHERE tenant_id IS NULL
      AND deleted_at IS NULL
      AND status <> 'revoked';

    IF unscoped IS NOT NULL THEN
        RAISE EXCEPTION
            'Migration 029 abort: agents % have no derivable tenant (empty allowed_* scopes). Assign tenant_id manually or revoke them before retrying.',
            unscoped;
    END IF;
END $$;

-- ---------------------------------------------------------------------
-- 5. Enforce NOT NULL going forward
-- ---------------------------------------------------------------------

-- Soft-deleted / revoked agents may retain NULL tenant_id (they are
-- harmless historical rows). New inserts and updates of active agents
-- must provide tenant_id.
ALTER TABLE agents
    ADD CONSTRAINT agents_tenant_id_required_when_active
    CHECK (
        tenant_id IS NOT NULL
        OR status = 'revoked'
        OR deleted_at IS NOT NULL
    );

CREATE INDEX IF NOT EXISTS idx_agents_tenant_id
    ON agents(tenant_id)
    WHERE deleted_at IS NULL;

-- ---------------------------------------------------------------------
-- 6. Relocate v_agent_permissions into the migration (was defined by
--    idp-sync::github::initialize_opal_views; see PR #131 for the
--    v_hierarchy / v_user_permissions relocation that preceded this).
--    Fixes u.display_name -> u.name (display_name never existed).
-- ---------------------------------------------------------------------

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
    a.allowed_company_ids AS allowed_tenant_ids,
    a.allowed_org_ids,
    a.allowed_team_ids,
    a.allowed_project_ids,
    a.status AS agent_status,
    u.email AS delegating_user_email,
    u.name AS delegating_user_name
FROM agents a
LEFT JOIN users u ON u.id = a.delegated_by_user_id;
