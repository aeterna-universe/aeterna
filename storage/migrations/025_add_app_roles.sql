-- =============================================================================
-- Migration 025 — Dual-role access model for RLS enforcement
-- =============================================================================
--
-- Creates two login roles that the application uses at runtime:
--
--   * aeterna_app   — NOBYPASSRLS, runs 99% of traffic. Every query it issues
--                     against an RLS-protected table is filtered by the table's
--                     policy evaluating current_setting('app.tenant_id').
--
--   * aeterna_admin — BYPASSRLS, used narrowly for PlatformAdmin cross-tenant
--                     list endpoints, scheduled cross-tenant maintenance, and
--                     the migration runner.
--
-- Companion to the RLS enforcement RFC:
--   openspec/changes/decide-rls-enforcement-model/proposal.md
--
-- This migration is Bundle A.2 of issue #58. DATABASE_URL is NOT flipped in
-- this bundle — it stays on its current role (typically `postgres`). The
-- connection-string cutover is Bundle A.3 Wave 6.
--
-- Idempotent: re-runnable. All CREATE ROLE statements are guarded by DO
-- blocks that check pg_roles first. GRANT statements are idempotent in
-- PostgreSQL by default.
--
-- Passwords are sourced from psql variables `:app_password` / `:admin_password`
-- in non-psql execution paths (sqlx migrations) we fall back to MD5 hashes
-- derived from environment variables set by the deployment pipeline; the
-- CREATE ROLE statements use PASSWORD NULL here and the deployment layer
-- issues ALTER ROLE … PASSWORD after migration runs. See
-- DEVELOPER_GUIDE.md §RLS enforcement for local setup.
-- =============================================================================

-- --------------------------------------------------------------------------
-- 1. Role creation (idempotent, superuser-aware)
-- --------------------------------------------------------------------------
--
-- The BYPASSRLS / NOBYPASSRLS role attributes can ONLY be set by a
-- SUPERUSER; CREATEROLE (which the CNPG-managed `aeterna` app role has) is
-- insufficient. Running this migration as the non-superuser app role on a
-- greenfield or DB-wipe deploy therefore used to stall with an error here,
-- because every branch below touched a BYPASSRLS clause (issue #194).
--
-- Strategy (see issue #194 / decide-rls-enforcement-model proposal.md §A.2):
--
--   * The two login roles AND their BYPASSRLS/NOBYPASSRLS attributes are the
--     responsibility of the **superuser prereqs layer** (CNPG `managed.roles`
--     in charts/aeterna-prereqs). The operator runs as superuser, so the roles
--     exist with the correct attributes before the app-user migration runs.
--   * When this migration IS executed by a superuser (local dev with the
--     `postgres` superuser, or an operator-initiated run) it still creates /
--     repairs the roles itself for convenience and dev ergonomics — exactly the
--     pre-#194 behaviour, gated on `current_user` actually being a superuser.
--   * When executed by a non-superuser it skips the BYPASSRLS-touching
--     statements with a WARNING (not an error): the roles are expected to be
--     pre-provisioned by the prereqs layer. The subsequent GRANT / column / index
--     statements below are unconditional and work fine for the app owner.
-- --------------------------------------------------------------------------

DO $mig$
DECLARE
    is_superuser BOOLEAN;
BEGIN
    SELECT rolsuper INTO is_superuser FROM pg_roles WHERE rolname = current_user;

    -- ---------- aeterna_app ------------------------------------------------
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'aeterna_app') THEN
        IF is_superuser THEN
            -- NOBYPASSRLS is the default for CREATE ROLE but stated explicitly
            -- here so the intent is unmistakable to future readers.
            CREATE ROLE aeterna_app LOGIN NOBYPASSRLS PASSWORD NULL;
            RAISE NOTICE 'Created role aeterna_app (NOBYPASSRLS)';
        ELSE
            -- Cannot CREATE ROLE ... BYPASSRLS|NOBYPASSRLS without SUPERUSER.
            -- The prereqs layer (CNPG managed.roles) must provision it.
            RAISE WARNING
                'Migration 025 (non-superuser run): role aeterna_app is missing. '
                'It must be pre-provisioned by the superuser prereqs layer '
                '(CNPG managed.roles in charts/aeterna-prereqs). Skipping '
                'role creation; downstream GRANTs require it to already exist.';
        END IF;
    ELSE
        IF is_superuser THEN
            -- Defensive: some dev environments may have created the role with
            -- BYPASSRLS by mistake. Force the correct state on every run.
            ALTER ROLE aeterna_app NOBYPASSRLS;
            RAISE NOTICE 'Role aeterna_app already exists; ensured NOBYPASSRLS';
        ELSE
            RAISE NOTICE
                'Role aeterna_app already exists; skipping NOBYPASSRLS '
                'enforcement (requires SUPERUSER — prereqs layer owns this)';
        END IF;
    END IF;

    -- ---------- aeterna_admin ----------------------------------------------
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'aeterna_admin') THEN
        IF is_superuser THEN
            CREATE ROLE aeterna_admin LOGIN BYPASSRLS PASSWORD NULL;
            RAISE NOTICE 'Created role aeterna_admin (BYPASSRLS)';
        ELSE
            RAISE WARNING
                'Migration 025 (non-superuser run): role aeterna_admin is missing. '
                'It must be pre-provisioned by the superuser prereqs layer '
                '(CNPG managed.roles in charts/aeterna-prereqs). Skipping '
                'role creation; downstream GRANTs require it to already exist.';
        END IF;
    ELSE
        IF is_superuser THEN
            ALTER ROLE aeterna_admin BYPASSRLS;
            RAISE NOTICE 'Role aeterna_admin already exists; ensured BYPASSRLS';
        ELSE
            RAISE NOTICE
                'Role aeterna_admin already exists; skipping BYPASSRLS '
                'enforcement (requires SUPERUSER — prereqs layer owns this)';
        END IF;
    END IF;
END
$mig$;

-- --------------------------------------------------------------------------
-- 2. Schema usage
-- --------------------------------------------------------------------------

GRANT USAGE ON SCHEMA public TO aeterna_app, aeterna_admin;

-- --------------------------------------------------------------------------
-- 3. Table grants for aeterna_app (enumerated explicitly)
-- --------------------------------------------------------------------------
--
-- Tables listed here MUST match the set of tables currently reporting
-- `rowsecurity = true` in pg_tables. The CI pre-flight in
-- cli/tests/rls_enforcement_test.rs enumerates that set at test time and
-- fails if any RLS-protected table is missing a grant. A broad
-- `GRANT … ON ALL TABLES` would silently paper over such omissions and
-- is deliberately NOT used here.
-- --------------------------------------------------------------------------

GRANT SELECT, INSERT, UPDATE, DELETE ON approval_decisions           TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON approval_requests            TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON codesearch_identities        TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON codesearch_index_metadata    TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON codesearch_repositories      TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON codesearch_requests          TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON codesearch_usage_metrics     TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON decomposition_policy_state   TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON decomposition_policy_weights TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON decomposition_trajectories   TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON error_signatures             TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON escalation_queue             TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON event_consumer_state         TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON event_delivery_metrics       TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON governance_configs           TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON governance_events            TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON governance_roles             TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON hindsight_notes              TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON knowledge_items              TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON memory_entries               TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON resolutions                  TO aeterna_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON sync_state                   TO aeterna_app;

-- aeterna_app also needs access to non-RLS supporting tables it reads/writes
-- during normal request processing (tenants, users, roles, audit logs, …).
-- These do not have RLS enabled but the role still needs grants because
-- NOBYPASSRLS does not imply "no grants" — it's a separate check.
GRANT SELECT, INSERT, UPDATE, DELETE ON
    tenants,
    users,
    user_roles,
    organizational_units,
    governance_audit_log,
    referential_audit_log
TO aeterna_app;

-- --------------------------------------------------------------------------
-- 4. Table grants for aeterna_admin
-- --------------------------------------------------------------------------
--
-- aeterna_admin is BYPASSRLS and operates globally, so a broad grant is
-- both correct and future-proof. The narrow set of call sites that use it
-- (via with_admin_context) is audited at the application layer.
-- --------------------------------------------------------------------------

GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO aeterna_admin;
-- Defaults must attach to the table-creating role (aeterna), not whichever
-- role happens to run this migration (which may be postgres in bootstrap).
ALTER DEFAULT PRIVILEGES FOR ROLE aeterna IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO aeterna_admin;

-- --------------------------------------------------------------------------
-- 5. Sequence grants
-- --------------------------------------------------------------------------

GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO aeterna_app, aeterna_admin;
ALTER DEFAULT PRIVILEGES FOR ROLE aeterna IN SCHEMA public
    GRANT USAGE, SELECT ON SEQUENCES TO aeterna_app, aeterna_admin;

-- --------------------------------------------------------------------------
-- 6. governance_audit_log.admin_scope column
-- --------------------------------------------------------------------------
--
-- Marks audit rows produced by with_admin_context. Combined with the
-- existing acting_as_tenant_id column (migration 023) this gives us full
-- provenance for every cross-tenant administrative access.
-- --------------------------------------------------------------------------

ALTER TABLE governance_audit_log
    ADD COLUMN IF NOT EXISTS admin_scope BOOLEAN NOT NULL DEFAULT FALSE;

COMMENT ON COLUMN governance_audit_log.admin_scope IS
    'TRUE when the audit row was recorded inside with_admin_context '
    '(cross-tenant administrative access via the BYPASSRLS aeterna_admin '
    'role). FALSE for normal tenant-scoped audit events. Introduced in '
    'migration 025 as part of the RLS enforcement model (issue #58).';

CREATE INDEX IF NOT EXISTS idx_governance_audit_log_admin_scope
    ON governance_audit_log(admin_scope, created_at DESC)
    WHERE admin_scope = TRUE;
