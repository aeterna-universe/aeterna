-- 036_accounts.sql
--
-- Introduce first-class accounts above tenants and attach existing tenants
-- to accounts derived from the v1.5.x `tenants.legal_entity_name` seed.
--
-- This migration is intentionally additive for the first rollout step:
-- it backfills `accounts` + `tenants.account_id` but does not yet remove the
-- old `legal_entity_name` column. Later steps in the account/tenant hierarchy
-- refactor can delete the seed once all runtime paths read the new model.

CREATE TABLE IF NOT EXISTS accounts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug TEXT UNIQUE NOT NULL,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ
);

ALTER TABLE tenants
    ADD COLUMN IF NOT EXISTS account_id UUID REFERENCES accounts(id) ON DELETE SET NULL;

ALTER TABLE tenants
    ADD COLUMN IF NOT EXISTS environment TEXT;

CREATE INDEX IF NOT EXISTS idx_tenants_account_id
    ON tenants(account_id)
    WHERE account_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_tenants_environment
    ON tenants(environment)
    WHERE environment IS NOT NULL;

WITH source_names AS (
    SELECT DISTINCT trim(legal_entity_name) AS name
      FROM tenants
     WHERE legal_entity_name IS NOT NULL
       AND trim(legal_entity_name) <> ''
),
slug_base AS (
    SELECT
        name,
        trim(both '-' FROM regexp_replace(lower(name), '[^a-z0-9]+', '-', 'g')) AS base_slug
    FROM source_names
),
slugged AS (
    SELECT
        name,
        CASE
            WHEN COUNT(*) OVER (PARTITION BY base_slug) = 1 THEN NULLIF(base_slug, '')
            ELSE NULLIF(base_slug, '') || '-' || ROW_NUMBER() OVER (PARTITION BY base_slug ORDER BY name)
        END AS slug
    FROM slug_base
)
INSERT INTO accounts (slug, name, created_at, updated_at)
SELECT COALESCE(slug, 'account'), name, NOW(), NOW()
  FROM slugged
ON CONFLICT (slug) DO NOTHING;

UPDATE tenants t
   SET account_id = a.id
  FROM accounts a
 WHERE t.account_id IS NULL
   AND t.legal_entity_name IS NOT NULL
   AND trim(t.legal_entity_name) <> ''
   AND a.name = trim(t.legal_entity_name);
