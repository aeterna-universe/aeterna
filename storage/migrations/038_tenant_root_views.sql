-- 038_tenant_root_views.sql
--
-- Rewrite OPAL hierarchy views to anchor on tenant-owned organizations.
-- Legacy root columns are removed completely.

DROP VIEW IF EXISTS v_hierarchy;
CREATE VIEW v_hierarchy AS
SELECT
    o.tenant_id,
    o.id as org_id,
    o.slug as org_slug,
    o.name as org_name,
    t.id as team_id,
    t.slug as team_slug,
    t.name as team_name,
    p.id as project_id,
    p.slug as project_slug,
    p.name as project_name,
    p.git_remote
FROM organizations o
LEFT JOIN teams t ON t.org_id = o.id AND t.deleted_at IS NULL
LEFT JOIN projects p ON p.team_id = t.id AND p.deleted_at IS NULL
WHERE o.deleted_at IS NULL;

DROP VIEW IF EXISTS v_user_permissions;
CREATE VIEW v_user_permissions AS
SELECT
    o.tenant_id,
    u.id as user_id,
    u.email,
    u.name as user_name,
    u.status as user_status,
    m.team_id,
    m.role,
    m.permissions,
    t.org_id,
    o.slug as org_slug,
    t.slug as team_slug
FROM users u
JOIN memberships m ON m.user_id = u.id AND m.status = 'active'
JOIN teams t ON t.id = m.team_id AND t.deleted_at IS NULL
JOIN organizations o ON o.id = t.org_id AND o.deleted_at IS NULL
WHERE u.deleted_at IS NULL AND u.status = 'active';

COMMENT ON VIEW v_hierarchy IS 'OPAL view: tenant-root organizational hierarchy for Cedar entities. Anchored on organizations as of migration 038.';
COMMENT ON VIEW v_user_permissions IS 'OPAL view: tenant-root user permissions for Cedar authorization. Anchored on organizations as of migration 038.';
