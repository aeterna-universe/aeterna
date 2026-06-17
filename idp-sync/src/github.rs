use crate::config::GitHubConfig;
use crate::error::{IdpSyncError, IdpSyncResult};
use crate::okta::{GroupPage, GroupType, IdpClient, IdpGroup, IdpUser, UserPage, UserStatus};
use async_trait::async_trait;
use chrono::Utc;
use mk_core::types::PROVIDER_GITHUB;
use serde::Deserialize;
use sqlx::PgPool;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info};
use uuid::Uuid;

struct AppCredentials {
    app_id: u64,
    installation_id: u64,
    pem_key: String,
    api_base_url: Option<String>,
}

struct CachedToken {
    token: String,
    expires_at: std::time::Instant,
}

pub struct GitHubClient {
    client: Arc<Mutex<octocrab::Octocrab>>,
    credentials: AppCredentials,
    token_cache: Arc<Mutex<Option<CachedToken>>>,
    org_name: String,
    team_filter: Option<regex::Regex>,
}

impl std::fmt::Debug for GitHubClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubClient")
            .field("org_name", &self.org_name)
            .field("app_id", &self.credentials.app_id)
            .finish_non_exhaustive()
    }
}

impl GitHubClient {
    pub async fn new(config: GitHubConfig) -> IdpSyncResult<Self> {
        let _ = jsonwebtoken::EncodingKey::from_rsa_pem(config.private_key_pem.as_bytes())
            .map_err(|e| IdpSyncError::AuthenticationError(format!("Invalid PEM key: {e}")))?;

        let credentials = AppCredentials {
            app_id: config.app_id,
            installation_id: config.installation_id,
            pem_key: config.private_key_pem,
            api_base_url: config.api_base_url.clone(),
        };

        let token = Self::mint_installation_token(&credentials).await?;

        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let octocrab = {
            let mut builder = octocrab::Octocrab::builder().personal_token(token.clone());
            if let Some(ref base_url) = config.api_base_url {
                builder = builder
                    .base_uri(base_url)
                    .map_err(|e| IdpSyncError::ConfigError(format!("Invalid base URL: {e}")))?;
            }
            builder
                .build()
                .map_err(|e| IdpSyncError::AuthenticationError(e.to_string()))?
        };

        let token_cache = Arc::new(Mutex::new(Some(CachedToken {
            token,
            expires_at: std::time::Instant::now() + std::time::Duration::from_mins(55),
        })));

        let team_filter = config
            .team_filter
            .as_deref()
            .map(|pattern| {
                regex::Regex::new(pattern).map_err(|e| {
                    IdpSyncError::ConfigError(format!("Invalid team_filter regex: {e}"))
                })
            })
            .transpose()?;

        Ok(Self {
            client: Arc::new(Mutex::new(octocrab)),
            credentials,
            token_cache,
            org_name: config.org_name,
            team_filter,
        })
    }

    async fn mint_installation_token(creds: &AppCredentials) -> IdpSyncResult<String> {
        let now = Utc::now();
        let claims = serde_json::json!({
            "iat": (now - chrono::Duration::seconds(60)).timestamp(),
            "exp": (now + chrono::Duration::seconds(600)).timestamp(),
            "iss": creds.app_id.to_string(),
        });

        let jwt_key = jsonwebtoken::EncodingKey::from_rsa_pem(creds.pem_key.as_bytes())
            .map_err(|e| IdpSyncError::AuthenticationError(format!("PEM encode error: {e}")))?;
        let jwt = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            &claims,
            &jwt_key,
        )
        .map_err(|e| IdpSyncError::AuthenticationError(format!("JWT sign error: {e}")))?;

        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let http_client = {
            let mut builder = octocrab::Octocrab::builder().personal_token(jwt);
            if let Some(ref base_url) = creds.api_base_url {
                builder = builder
                    .base_uri(base_url)
                    .map_err(|e| IdpSyncError::ConfigError(format!("Invalid base URL: {e}")))?;
            }
            builder
                .build()
                .map_err(|e| IdpSyncError::AuthenticationError(e.to_string()))?
        };

        let url = format!("/app/installations/{}/access_tokens", creds.installation_id);
        let resp: serde_json::Value = http_client.post(url, None::<&()>).await.map_err(|e| {
            IdpSyncError::AuthenticationError(format!("Token exchange failed: {e:?}"))
        })?;

        resp["token"]
            .as_str()
            .ok_or_else(|| IdpSyncError::AuthenticationError("No token in response".to_string()))
            .map(std::string::ToString::to_string)
    }

    async fn ensure_valid_token(&self) -> IdpSyncResult<()> {
        let needs_refresh = {
            let cache = self.token_cache.lock().await;
            match *cache {
                Some(ref cached) => {
                    cached.expires_at
                        <= std::time::Instant::now() + std::time::Duration::from_mins(5)
                }
                None => true,
            }
        };

        if !needs_refresh {
            return Ok(());
        }

        info!("Refreshing GitHub installation token");
        let token = Self::mint_installation_token(&self.credentials).await?;

        let new_client = {
            let mut builder = octocrab::Octocrab::builder().personal_token(token.clone());
            if let Some(ref base_url) = self.credentials.api_base_url {
                builder = builder
                    .base_uri(base_url)
                    .map_err(|e| IdpSyncError::ConfigError(format!("Invalid base URL: {e}")))?;
            }
            builder
                .build()
                .map_err(|e| IdpSyncError::AuthenticationError(e.to_string()))?
        };

        *self.client.lock().await = new_client;
        *self.token_cache.lock().await = Some(CachedToken {
            token,
            expires_at: std::time::Instant::now() + std::time::Duration::from_mins(55),
        });

        Ok(())
    }

    fn matches_team_filter(&self, team_name: &str) -> bool {
        match &self.team_filter {
            Some(re) => re.is_match(team_name),
            None => true,
        }
    }
}

#[derive(Debug, Deserialize)]
struct GitHubOrgMember {
    login: String,
    id: u64,
    #[serde(rename = "type")]
    user_type: Option<String>,
    site_admin: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct GitHubUser {
    login: String,
    id: u64,
    name: Option<String>,
    email: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubTeam {
    id: u64,
    slug: String,
    name: String,
    description: Option<String>,
    parent: Option<GitHubTeamParent>,
}

#[derive(Debug, Deserialize)]
struct GitHubTeamParent {
    id: u64,
    slug: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct GitHubTeamMember {
    login: String,
    id: u64,
}

fn parse_github_datetime(s: &str) -> chrono::DateTime<Utc> {
    chrono::DateTime::parse_from_rfc3339(s).map_or_else(|_| Utc::now(), |dt| dt.with_timezone(&Utc))
}

fn map_org_role_to_status(role: &str) -> UserStatus {
    match role {
        "admin" => UserStatus::Active,
        "member" => UserStatus::Active,
        _ => UserStatus::Active,
    }
}

#[async_trait]
impl IdpClient for GitHubClient {
    async fn list_users(&self, page_token: Option<&str>) -> IdpSyncResult<UserPage> {
        self.ensure_valid_token().await?;
        let client = self.client.lock().await;

        let page: u32 = page_token.and_then(|s| s.parse().ok()).unwrap_or(1);

        let url = format!(
            "/orgs/{}/members?per_page=100&page={}&role=all",
            self.org_name, page
        );

        let members: Vec<GitHubOrgMember> =
            client
                .get(url, None::<&()>)
                .await
                .map_err(|e| IdpSyncError::IdpApiError {
                    status: 0,
                    message: format!("Failed to list org members: {e:?}"),
                })?;

        let member_count = members.len();

        let mut users = Vec::with_capacity(members.len());
        for member in members {
            let now = Utc::now();
            users.push(IdpUser {
                id: member.id.to_string(),
                email: format!("{}@github.com", member.login),
                first_name: None,
                last_name: None,
                display_name: Some(member.login.clone()),
                status: UserStatus::Active,
                created_at: now,
                updated_at: now,
                idp_provider: PROVIDER_GITHUB.to_string(),
                idp_subject: member.login,
            });
        }

        let next_page_token = if member_count >= 100 {
            Some((page + 1).to_string())
        } else {
            None
        };

        Ok(UserPage {
            users,
            next_page_token,
        })
    }

    async fn list_groups(&self, page_token: Option<&str>) -> IdpSyncResult<GroupPage> {
        self.ensure_valid_token().await?;
        let client = self.client.lock().await;

        let page: u32 = page_token.and_then(|s| s.parse().ok()).unwrap_or(1);

        let url = format!("/orgs/{}/teams?per_page=100&page={}", self.org_name, page);

        let teams: Vec<GitHubTeam> =
            client
                .get(url, None::<&()>)
                .await
                .map_err(|e| IdpSyncError::IdpApiError {
                    status: 0,
                    message: format!("Failed to list org teams: {e:?}"),
                })?;

        let team_count = teams.len();

        let mut groups = Vec::with_capacity(teams.len());
        for team in teams {
            if !self.matches_team_filter(&team.name) {
                debug!(team_name = %team.name, "Skipping team (filtered)");
                continue;
            }

            let (group_type, description) = match &team.parent {
                None => (GroupType::GitHubTeam, team.description.clone()),
                Some(parent) => {
                    let desc = format!("parent:{}", parent.slug);
                    (GroupType::GitHubNestedTeam, Some(desc))
                }
            };

            let now = Utc::now();
            groups.push(IdpGroup {
                id: team.slug.clone(),
                name: team.name,
                description,
                group_type,
                created_at: now,
                updated_at: now,
            });
        }

        let next_page_token = if team_count >= 100 {
            Some((page + 1).to_string())
        } else {
            None
        };

        Ok(GroupPage {
            groups,
            next_page_token,
        })
    }

    async fn get_group_members(&self, group_id: &str) -> IdpSyncResult<Vec<IdpUser>> {
        self.ensure_valid_token().await?;
        let client = self.client.lock().await;

        let mut all_members = Vec::new();
        let mut page: u32 = 1;

        loop {
            let url = format!(
                "/orgs/{}/teams/{}/members?per_page=100&page={}",
                self.org_name, group_id, page
            );

            let members: Vec<GitHubTeamMember> =
                client
                    .get(url, None::<&()>)
                    .await
                    .map_err(|e| IdpSyncError::IdpApiError {
                        status: 0,
                        message: format!("Failed to list team members for {group_id}: {e:?}"),
                    })?;

            let count = members.len();

            for member in members {
                let now = Utc::now();
                all_members.push(IdpUser {
                    id: member.id.to_string(),
                    email: format!("{}@github.com", member.login),
                    first_name: None,
                    last_name: None,
                    display_name: Some(member.login.clone()),
                    status: UserStatus::Active,
                    created_at: now,
                    updated_at: now,
                    idp_provider: PROVIDER_GITHUB.to_string(),
                    idp_subject: member.login,
                });
            }

            if count < 100 {
                break;
            }
            page += 1;
        }

        Ok(all_members)
    }

    async fn get_user(&self, user_id: &str) -> IdpSyncResult<IdpUser> {
        self.ensure_valid_token().await?;
        let client = self.client.lock().await;

        let url = format!("/users/{user_id}");
        let user: GitHubUser = client
            .get(url, None::<&()>)
            .await
            .map_err(|e| IdpSyncError::UserNotFound(format!("{user_id}: {e:?}")))?;

        let now = Utc::now();
        let created = user
            .created_at
            .as_deref()
            .map_or(now, parse_github_datetime);
        let updated = user
            .updated_at
            .as_deref()
            .map_or(now, parse_github_datetime);

        let (first_name, last_name) = user
            .name
            .as_deref()
            .and_then(|n| n.split_once(' '))
            .map(|(f, l)| (Some(f.to_string()), Some(l.to_string())))
            .unwrap_or((user.name.clone(), None));

        Ok(IdpUser {
            id: user.id.to_string(),
            email: user
                .email
                .unwrap_or_else(|| format!("{}@github.com", user.login)),
            first_name,
            last_name,
            display_name: Some(user.login.clone()),
            status: UserStatus::Active,
            created_at: created,
            updated_at: updated,
            idp_provider: PROVIDER_GITHUB.to_string(),
            idp_subject: user.login,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AeternaRole {
    Admin,
    TechLead,
    Developer,
}

impl AeternaRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            AeternaRole::Admin => "admin",
            AeternaRole::TechLead => "techlead",
            AeternaRole::Developer => "developer",
        }
    }

    pub fn precedence(&self) -> u8 {
        match self {
            AeternaRole::Admin => 4,
            AeternaRole::TechLead => 2,
            AeternaRole::Developer => 1,
        }
    }
}

pub fn map_github_org_role(role: &str) -> AeternaRole {
    match role {
        "admin" => AeternaRole::Admin,
        _ => AeternaRole::Developer,
    }
}

pub fn map_github_team_role(role: &str) -> AeternaRole {
    match role {
        "maintainer" => AeternaRole::TechLead,
        _ => AeternaRole::Developer,
    }
}

pub struct GitHubHierarchyMapper {
    db_pool: PgPool,
    tenant_id: Uuid,
}

/// Initialize the database schema required for GitHub org sync.
///
/// This creates the idp-sync tables (`users`, `memberships`, `idp_group_mappings`)
/// and ensures the auxiliary OPAL/code-search stubs and NOTIFY triggers exist.
/// GitHub hierarchy materialization writes to the modern tenant-root
/// hierarchy tables.
pub async fn initialize_github_sync_schema(pool: &PgPool) -> IdpSyncResult<()> {
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pgcrypto")
        .execute(pool)
        .await
        .map_err(IdpSyncError::DatabaseError)?;

    sqlx::query(r#"CREATE EXTENSION IF NOT EXISTS "uuid-ossp""#)
        .execute(pool)
        .await
        .map_err(IdpSyncError::DatabaseError)?;

    // Tenants table — required before resolve_tenant_id() can query it
    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS tenants (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            name TEXT NOT NULL UNIQUE,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        ",
    )
    .execute(pool)
    .await
    .map_err(IdpSyncError::DatabaseError)?;

    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS users (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            email TEXT NOT NULL UNIQUE,
            first_name TEXT,
            last_name TEXT,
            display_name TEXT,
            idp_provider TEXT NOT NULL,
            idp_subject TEXT NOT NULL,
            is_active BOOLEAN NOT NULL DEFAULT true,
            deactivated_at TIMESTAMPTZ,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            UNIQUE (idp_provider, idp_subject)
        )
        ",
    )
    .execute(pool)
    .await
    .map_err(IdpSyncError::DatabaseError)?;

    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS memberships (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            team_id UUID NOT NULL,
            user_id UUID NOT NULL REFERENCES users(id),
            role TEXT NOT NULL DEFAULT 'member',
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            UNIQUE (team_id, user_id)
        )
        ",
    )
    .execute(pool)
    .await
    .map_err(IdpSyncError::DatabaseError)?;

    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS idp_group_mappings (
            idp_group_id TEXT NOT NULL,
            team_id UUID NOT NULL,
            idp_provider TEXT NOT NULL DEFAULT 'github',
            created_at BIGINT NOT NULL DEFAULT 0,
            PRIMARY KEY (idp_group_id, idp_provider)
        )
        ",
    )
    .execute(pool)
    .await
    .map_err(IdpSyncError::DatabaseError)?;

    // Agents table — required for v_agent_permissions OPAL view
    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS agents (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            name TEXT NOT NULL,
            agent_type TEXT NOT NULL DEFAULT 'coding-assistant',
            delegated_by_user_id UUID REFERENCES users(id),
            delegated_by_agent_id UUID,
            delegation_depth INT NOT NULL DEFAULT 0,
            capabilities JSONB DEFAULT '[]',
            allowed_tenant_ids UUID[],
            allowed_org_ids UUID[],
            allowed_team_ids UUID[],
            allowed_project_ids UUID[],
            status TEXT NOT NULL DEFAULT 'active',
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        ",
    )
    .execute(pool)
    .await
    .map_err(IdpSyncError::DatabaseError)?;

    initialize_opal_views(pool).await?;
    initialize_notify_triggers(pool).await?;

    info!("GitHub sync schema initialized");
    Ok(())
}

/// Initializes Code Search stub views (`v_code_search_repositories`,
/// `v_code_search_requests`, `v_code_search_identities`) that the Code
/// Search feature has not yet populated with its own schema.
///
/// # History
///
/// Up to PR #129 this function defined five views, including
/// `v_hierarchy`, `v_user_permissions`, and `v_agent_permissions`. All
/// three raced the storage migrations via `CREATE OR REPLACE VIEW`;
/// whichever ran last won, and `aeterna admin sync github` could
/// silently revert migration-owned definitions. The relocation was done
/// in two steps:
///
/// - PR #131 moved `v_hierarchy` and `v_user_permissions` ownership to
///   migration `028_tenant_scoped_hierarchy.sql`, and additionally
///   fixed a latent bug where the idp-sync definitions pointed at the
///   legacy `organizational_units` table with `uuid_generate_v5`-
///   synthesized IDs that didn't match real hierarchy row IDs.
/// - PR #132 (this PR) moves `v_agent_permissions` ownership to
///   migration `029_agents_tenant_scope.sql`, which also adds
///   `agents.tenant_id` (backfilled + enforced) so that the view can
///   surface a tenant column for opal-fetcher's isolation filter. The
///   idp-sync definition additionally had `u.display_name` referenced,
///   which never existed on the `users` table — the migration uses
///   `u.name` correctly.
///
/// This function now owns only the Code Search stubs, which are
/// empty-result placeholders until the Code Search schema lands in its
/// own migration.
async fn initialize_opal_views(pool: &PgPool) -> IdpSyncResult<()> {
    sqlx::query(
        r"
        CREATE OR REPLACE VIEW v_code_search_repositories AS
        SELECT
            '00000000-0000-0000-0000-000000000000'::UUID AS id,
            '' AS tenant_id,
            '' AS name,
            '' AS status,
            '' AS sync_strategy,
            '' AS current_branch
        WHERE FALSE
        ",
    )
    .execute(pool)
    .await
    .map_err(IdpSyncError::DatabaseError)?;

    sqlx::query(
        r"
        CREATE OR REPLACE VIEW v_code_search_requests AS
        SELECT
            '00000000-0000-0000-0000-000000000000'::UUID AS id,
            '00000000-0000-0000-0000-000000000000'::UUID AS repository_id,
            '' AS requester_id,
            '' AS status,
            '' AS tenant_id
        WHERE FALSE
        ",
    )
    .execute(pool)
    .await
    .map_err(IdpSyncError::DatabaseError)?;

    sqlx::query(
        r"
        CREATE OR REPLACE VIEW v_code_search_identities AS
        SELECT
            '00000000-0000-0000-0000-000000000000'::UUID AS id,
            '' AS tenant_id,
            '' AS name,
            '' AS provider
        WHERE FALSE
        ",
    )
    .execute(pool)
    .await
    .map_err(IdpSyncError::DatabaseError)?;

    info!("OPAL authorization views initialized");
    Ok(())
}

async fn initialize_notify_triggers(pool: &PgPool) -> IdpSyncResult<()> {
    sqlx::query(
        r"
        CREATE OR REPLACE FUNCTION fn_notify_entity_change()
        RETURNS TRIGGER AS $$
        BEGIN
            PERFORM pg_notify('aeterna_entity_change', json_build_object(
                'type', TG_TABLE_NAME,
                'op', TG_OP,
                'id', COALESCE(NEW.id::TEXT, OLD.id::TEXT)
            )::TEXT);
            RETURN COALESCE(NEW, OLD);
        END;
        $$ LANGUAGE plpgsql
        ",
    )
    .execute(pool)
    .await
    .map_err(IdpSyncError::DatabaseError)?;

    sqlx::query("DROP TRIGGER IF EXISTS trg_users_entity_change ON users")
        .execute(pool)
        .await
        .map_err(IdpSyncError::DatabaseError)?;
    sqlx::query(
        "CREATE TRIGGER trg_users_entity_change \
         AFTER INSERT OR UPDATE OR DELETE ON users \
         FOR EACH ROW EXECUTE FUNCTION fn_notify_entity_change()",
    )
    .execute(pool)
    .await
    .map_err(IdpSyncError::DatabaseError)?;

    sqlx::query("DROP TRIGGER IF EXISTS trg_memberships_entity_change ON memberships")
        .execute(pool)
        .await
        .map_err(IdpSyncError::DatabaseError)?;
    sqlx::query(
        "CREATE TRIGGER trg_memberships_entity_change \
         AFTER INSERT OR UPDATE OR DELETE ON memberships \
         FOR EACH ROW EXECUTE FUNCTION fn_notify_entity_change()",
    )
    .execute(pool)
    .await
    .map_err(IdpSyncError::DatabaseError)?;

    sqlx::query("DROP TRIGGER IF EXISTS trg_organizations_entity_change ON organizations")
        .execute(pool)
        .await
        .map_err(IdpSyncError::DatabaseError)?;
    sqlx::query(
        "CREATE TRIGGER trg_organizations_entity_change \
         AFTER INSERT OR UPDATE OR DELETE ON organizations \
         FOR EACH ROW EXECUTE FUNCTION fn_notify_entity_change()",
    )
    .execute(pool)
    .await
    .map_err(IdpSyncError::DatabaseError)?;

    sqlx::query("DROP TRIGGER IF EXISTS trg_teams_entity_change ON teams")
        .execute(pool)
        .await
        .map_err(IdpSyncError::DatabaseError)?;
    sqlx::query(
        "CREATE TRIGGER trg_teams_entity_change \
         AFTER INSERT OR UPDATE OR DELETE ON teams \
         FOR EACH ROW EXECUTE FUNCTION fn_notify_entity_change()",
    )
    .execute(pool)
    .await
    .map_err(IdpSyncError::DatabaseError)?;

    sqlx::query("DROP TRIGGER IF EXISTS trg_governance_roles_entity_change ON governance_roles")
        .execute(pool)
        .await
        .map_err(IdpSyncError::DatabaseError)?;
    sqlx::query(
        "CREATE TRIGGER trg_governance_roles_entity_change \
         AFTER INSERT OR UPDATE OR DELETE ON governance_roles \
         FOR EACH ROW EXECUTE FUNCTION fn_notify_entity_change()",
    )
    .execute(pool)
    .await
    .map_err(IdpSyncError::DatabaseError)?;

    sqlx::query("DROP TRIGGER IF EXISTS trg_agents_entity_change ON agents")
        .execute(pool)
        .await
        .map_err(IdpSyncError::DatabaseError)?;
    sqlx::query(
        "CREATE TRIGGER trg_agents_entity_change \
         AFTER INSERT OR UPDATE OR DELETE ON agents \
         FOR EACH ROW EXECUTE FUNCTION fn_notify_entity_change()",
    )
    .execute(pool)
    .await
    .map_err(IdpSyncError::DatabaseError)?;

    info!("PG NOTIFY triggers initialized");
    Ok(())
}

impl GitHubHierarchyMapper {
    pub fn new(db_pool: PgPool, tenant_id: Uuid) -> Self {
        Self { db_pool, tenant_id }
    }

    fn parent_slug(group: &IdpGroup) -> Option<&str> {
        group
            .description
            .as_deref()
            .and_then(|d| d.strip_prefix("parent:"))
            .filter(|value| !value.is_empty())
    }

    fn root_slug<'a>(group: &'a IdpGroup, by_slug: &HashMap<String, &'a IdpGroup>) -> &'a str {
        let mut current = group;
        let mut seen: HashSet<&str> = HashSet::new();
        while let Some(parent_slug) = Self::parent_slug(current) {
            if !seen.insert(parent_slug) {
                break;
            }
            let Some(parent) = by_slug.get(parent_slug).copied() else {
                break;
            };
            current = parent;
        }
        current.id.as_str()
    }

    pub async fn create_hierarchy(
        &self,
        org_name: &str,
        groups: &[IdpGroup],
    ) -> IdpSyncResult<HashMap<String, Uuid>> {
        info!(tenant_id = %self.tenant_id, org = %org_name, "Ensuring tenant-root GitHub hierarchy");

        let by_slug: HashMap<String, &IdpGroup> = groups
            .iter()
            .map(|group| (group.id.clone(), group))
            .collect();

        let mut org_inputs: BTreeMap<String, (String, Vec<(String, String)>)> = BTreeMap::new();
        for group in groups {
            let root_slug = Self::root_slug(group, &by_slug).to_string();
            let root_group = by_slug.get(&root_slug).copied().ok_or_else(|| {
                IdpSyncError::ConfigError(format!("Missing root group '{root_slug}'"))
            })?;
            let entry = org_inputs
                .entry(root_slug.clone())
                .or_insert_with(|| (root_group.name.clone(), Vec::new()));
            entry.1.push((group.id.clone(), group.name.clone()));
        }

        let mut org_ids: HashMap<String, Uuid> = HashMap::new();
        let mut slug_to_team_id: HashMap<String, Uuid> = HashMap::new();

        for (org_slug, (org_name, team_entries)) in org_inputs {
            let org_id: Uuid = sqlx::query_scalar(
                "INSERT INTO organizations (tenant_id, slug, name, settings, created_at, updated_at)
                 VALUES ($1, $2, $3, '{}'::jsonb, NOW(), NOW())
                 ON CONFLICT (tenant_id, slug)
                 DO UPDATE SET name = EXCLUDED.name, updated_at = NOW()
                 RETURNING id",
            )
            .bind(self.tenant_id)
            .bind(&org_slug)
            .bind(&org_name)
            .fetch_one(&self.db_pool)
            .await
            .map_err(IdpSyncError::DatabaseError)?;
            org_ids.insert(org_slug.clone(), org_id);

            for (team_slug, team_name) in team_entries {
                let team_id: Uuid = sqlx::query_scalar(
                    "INSERT INTO teams (org_id, slug, name, settings, created_at, updated_at)
                     VALUES ($1, $2, $3, '{}'::jsonb, NOW(), NOW())
                     ON CONFLICT (org_id, slug)
                     DO UPDATE SET name = EXCLUDED.name, updated_at = NOW()
                     RETURNING id",
                )
                .bind(org_id)
                .bind(&team_slug)
                .bind(&team_name)
                .fetch_one(&self.db_pool)
                .await
                .map_err(IdpSyncError::DatabaseError)?;
                slug_to_team_id.insert(team_slug.clone(), team_id);
                debug!(org_slug = %org_slug, team_slug = %team_slug, team_id = %team_id, "Team ensured in modern hierarchy");
            }
        }

        Ok(slug_to_team_id)
    }

    pub async fn store_group_to_team_mappings(
        &self,
        mappings: &HashMap<String, Uuid>,
    ) -> IdpSyncResult<()> {
        let now_epoch = Utc::now().timestamp();
        for (slug, unit_id) in mappings {
            sqlx::query(
                r"
                INSERT INTO idp_group_mappings (idp_group_id, team_id, idp_provider, created_at)
                VALUES ($1, $2, 'github', $3)
                ON CONFLICT (idp_group_id, idp_provider)
                DO UPDATE SET team_id = EXCLUDED.team_id
                ",
            )
            .bind(slug)
            .bind(*unit_id)
            .bind(now_epoch)
            .execute(&self.db_pool)
            .await
            .map_err(IdpSyncError::DatabaseError)?;
        }
        info!(count = mappings.len(), "Group-to-team mappings stored");
        Ok(())
    }
}

pub async fn run_github_sync(
    config: &GitHubConfig,
    db_pool: &PgPool,
    tenant_id: Uuid,
) -> IdpSyncResult<crate::sync::SyncReport> {
    // Ensure schema exists before any DB queries (CronJob path bypasses admin_sync.rs)
    initialize_github_sync_schema(db_pool).await?;

    let github_client = GitHubClient::new(config.clone()).await?;
    let mapper = GitHubHierarchyMapper::new(db_pool.clone(), tenant_id);

    let mut all_groups = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let page = github_client.list_groups(page_token.as_deref()).await?;
        all_groups.extend(page.groups);
        page_token = page.next_page_token;
        if page_token.is_none() {
            break;
        }
    }
    info!(count = all_groups.len(), "Fetched GitHub teams");

    let slug_mappings = mapper
        .create_hierarchy(&config.org_name, &all_groups)
        .await?;
    mapper.store_group_to_team_mappings(&slug_mappings).await?;

    let sync_config = crate::config::IdpSyncConfig {
        provider: crate::config::IdpProvider::GitHub(config.clone()),
        database_url: String::new(),
        ..Default::default()
    };

    let client: Arc<dyn IdpClient> = Arc::new(github_client);
    let sync_service = crate::sync::IdpSyncService::new(sync_config, client, db_pool.clone());
    let report = sync_service.sync_all().await?;

    Ok(report)
}

pub async fn bridge_sync_to_governance(
    pool: &PgPool,
    tenant_id: Uuid,
) -> IdpSyncResult<(usize, usize)> {
    let tenant_uuid_expr =
        format!("uuid_generate_v5('6ba7b810-9dad-11d1-80b4-00c04fd430c8'::UUID, '{tenant_id}')");
    let _ = tenant_uuid_expr;

    let roles_created = sqlx::query_scalar::<_, i64>(
        r"
        WITH synced AS (
            INSERT INTO governance_roles (
                principal_type, principal_id, role,
                tenant_id, org_id, team_id,
                granted_by, granted_at
            )
            SELECT
                'user',
                u.id,
                m.role,
                NULL::UUID,
                o.id,
                t.id,
                u.id,
                NOW()
            FROM users u
            JOIN memberships m ON m.user_id = u.id
            JOIN teams t ON t.id = m.team_id AND t.deleted_at IS NULL
            JOIN organizations o ON o.id = t.org_id AND o.deleted_at IS NULL
            WHERE o.tenant_id = $1
            ON CONFLICT (
                principal_type, principal_id, role,
                COALESCE(tenant_id, '00000000-0000-0000-0000-000000000000'::UUID),
                COALESCE(org_id, '00000000-0000-0000-0000-000000000000'::UUID),
                COALESCE(team_id, '00000000-0000-0000-0000-000000000000'::UUID),
                COALESCE(project_id, '00000000-0000-0000-0000-000000000000'::UUID)
            )
            DO NOTHING
            RETURNING 1
        )
        SELECT COUNT(*) FROM synced
        ",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await
    .map_err(IdpSyncError::DatabaseError)?;

    let user_roles_created = sqlx::query_scalar::<_, i64>(
        r"
        WITH synced AS (
            INSERT INTO user_roles (user_id, tenant_id, unit_id, role, created_at)
            SELECT
                u.id::TEXT,
                o.tenant_id::TEXT,
                t.id::TEXT,
                m.role,
                NOW()
            FROM users u
            JOIN memberships m ON m.user_id = u.id
            JOIN teams t ON t.id = m.team_id AND t.deleted_at IS NULL
            JOIN organizations o ON o.id = t.org_id AND o.deleted_at IS NULL
            WHERE o.tenant_id = $1
            ON CONFLICT (user_id, tenant_id, unit_id, role)
            DO NOTHING
            RETURNING 1
        )
        SELECT COUNT(*) FROM synced
        ",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await
    .map_err(IdpSyncError::DatabaseError)?;

    info!(
        governance_roles = roles_created,
        user_roles = user_roles_created,
        "Governance bridge completed"
    );

    Ok((roles_created as usize, user_roles_created as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_role_mapping_org_admin() {
        assert_eq!(map_github_org_role("admin"), AeternaRole::Admin);
        assert_eq!(map_github_org_role("admin").precedence(), 4);
    }

    #[test]
    fn test_role_mapping_org_member() {
        assert_eq!(map_github_org_role("member"), AeternaRole::Developer);
    }

    #[test]
    fn test_role_mapping_team_maintainer() {
        assert_eq!(map_github_team_role("maintainer"), AeternaRole::TechLead);
        assert_eq!(map_github_team_role("maintainer").precedence(), 2);
    }

    #[test]
    fn test_role_mapping_team_member() {
        assert_eq!(map_github_team_role("member"), AeternaRole::Developer);
    }

    #[test]
    fn test_role_precedence_ordering() {
        assert!(AeternaRole::Admin.precedence() > AeternaRole::TechLead.precedence());
        assert!(AeternaRole::TechLead.precedence() > AeternaRole::Developer.precedence());
    }

    #[test]
    fn test_parse_github_datetime_valid() {
        let dt = parse_github_datetime("2024-01-15T10:30:00Z");
        assert_eq!(dt.year(), 2024);
    }

    #[test]
    fn test_parse_github_datetime_invalid_returns_now() {
        let before = Utc::now();
        let dt = parse_github_datetime("not-a-date");
        assert!(dt >= before);
    }

    #[test]
    fn test_group_type_classification_top_level() {
        let group = IdpGroup {
            id: "platform".to_string(),
            name: "Platform".to_string(),
            description: None,
            group_type: GroupType::GitHubTeam,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert_eq!(group.group_type, GroupType::GitHubTeam);
    }

    #[test]
    fn test_group_type_classification_nested() {
        let group = IdpGroup {
            id: "api-team".to_string(),
            name: "API Team".to_string(),
            description: Some("parent:platform".to_string()),
            group_type: GroupType::GitHubNestedTeam,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert_eq!(group.group_type, GroupType::GitHubNestedTeam);
        let parent = group
            .description
            .as_deref()
            .and_then(|d| d.strip_prefix("parent:"));
        assert_eq!(parent, Some("platform"));
    }

    use chrono::Datelike;
}
