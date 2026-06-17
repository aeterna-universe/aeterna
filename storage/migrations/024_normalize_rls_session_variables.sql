-- Migration 024 — Normalize RLS session variable names
--
-- Context (see issue #59):
-- Three different Postgres session variables are used across RLS policies:
--   - app.tenant_id          : migrations 008, 009, 010, 013, 017, 020, GDPR runtime (canonical)
--   - app.current_tenant_id  : migration 006 only (pure naming inconsistency — same concept)
--   - legacy governance-scope GUC : migration 016 (DIFFERENT scoping axis — NOT renamed)
--
-- This migration rewrites the three policies from migration 006 to use the
-- canonical `app.tenant_id` so a single `activate_tenant_context` call can
-- satisfy every tenant-scoped RLS policy in the schema.
--
-- No Rust code reads `app.current_tenant_id` today (verified via grep), so
-- this is migration-only with zero runtime impact.
--
-- Related issues:
--   #57 — pool leak in set_config (orthogonal, fixed separately)
--   #58 — RLS not activated on most query paths (architectural, deferred)

BEGIN;

-- governance_events
DROP POLICY IF EXISTS governance_events_tenant_isolation ON governance_events;
CREATE POLICY governance_events_tenant_isolation ON governance_events
    FOR ALL
    USING (tenant_id = current_setting('app.tenant_id', true));

-- event_delivery_metrics
DROP POLICY IF EXISTS event_metrics_tenant_isolation ON event_delivery_metrics;
CREATE POLICY event_metrics_tenant_isolation ON event_delivery_metrics
    FOR ALL
    USING (tenant_id = current_setting('app.tenant_id', true));

-- event_consumer_state
DROP POLICY IF EXISTS consumer_state_tenant_isolation ON event_consumer_state;
CREATE POLICY consumer_state_tenant_isolation ON event_consumer_state
    FOR ALL
    USING (tenant_id = current_setting('app.tenant_id', true));

COMMIT;

-- Verification query (run post-migration to confirm zero remaining references):
--   SELECT schemaname, tablename, policyname, qual
--     FROM pg_policies
--    WHERE qual LIKE '%app.current_tenant_id%';
-- Expected: 0 rows.
