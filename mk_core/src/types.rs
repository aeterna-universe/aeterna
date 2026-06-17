use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};
use utoipa::ToSchema;
use validator::Validate;

/// Sentinel `tenant_id` used to store instance-scoped roles (e.g. `PlatformAdmin`)
/// in the `user_roles` table. Using a reserved value instead of a real tenant slug
/// prevents ambiguity and ensures platform-level grants are never confused with
/// tenant-scoped grants.
pub const INSTANCE_SCOPE_TENANT_ID: &str = "__root__";
pub const SYSTEM_USER_ID: &str = "system";
pub const DEFAULT_TENANT_SLUG: &str = "default";
pub const PROVIDER_GITHUB: &str = "github";
pub const PROVIDER_KUBERNETES: &str = "kubernetes";

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    EnumString,
    Display,
)]
#[strum(ascii_case_insensitive)]
pub enum Role {
    Viewer,
    Developer,
    TechLead,
    Architect,
    Admin,
    TenantAdmin,
    Agent,
    PlatformAdmin,
}

// ---------------------------------------------------------------------------
// Tenant record and repository binding types
// ---------------------------------------------------------------------------

/// Whether a tenant is active or has been deactivated.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, EnumString, Display,
)]
#[strum(ascii_case_insensitive)]
#[derive(Default)]
pub enum TenantStatus {
    #[default]
    Active,
    Inactive,
}

/// Who owns/manages a record: a human admin or an automated sync process.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, EnumString, Display,
)]
#[strum(ascii_case_insensitive)]
#[derive(Default)]
pub enum RecordSource {
    #[default]
    Admin,
    Sync,
}

/// Canonical kind of backing repository for a tenant knowledge binding.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, EnumString, Display,
)]
#[strum(ascii_case_insensitive)]
pub enum RepositoryKind {
    Local,
    GitHub,
    GitRemote,
}

/// Branch protection policy for a tenant knowledge repository.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, EnumString, Display,
)]
#[strum(ascii_case_insensitive)]
pub enum BranchPolicy {
    DirectCommit,
    RequirePullRequest,
}

/// Credential type stored in the tenant repository binding.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, EnumString, Display,
)]
#[strum(ascii_case_insensitive)]
pub enum CredentialKind {
    None,
    PersonalAccessToken,
    SshKey,
    GitHubApp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountRef {
    pub id: String,
    pub slug: String,
    pub name: String,
}

/// Canonical account/customer-organization record persisted above tenants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountRecord {
    pub id: String,
    pub slug: String,
    pub name: String,
    #[schema(value_type = String, format = DateTime, example = "2026-04-30T15:34:00Z")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = String, format = DateTime, example = "2026-04-30T15:34:00Z")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = Option<String>, format = DateTime, example = "2026-04-30T15:34:00Z")]
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Canonical tenant record as persisted in the database.
///
/// Timestamps are RFC 3339 / ISO 8601 strings on the wire (e.g.
/// `"2026-04-30T15:34:00Z"`) so frontend `new Date(...)` parses them
/// correctly. Historically these were `i64` epoch *seconds* which
/// browsers misinterpreted as JS milliseconds → 1970 dates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TenantRecord {
    pub id: TenantId,
    pub slug: String,
    pub name: String,
    pub status: TenantStatus,
    pub source_owner: RecordSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<AccountRef>,
    #[schema(example = "prod")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[schema(value_type = String, format = DateTime, example = "2026-04-30T15:34:00Z")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = String, format = DateTime, example = "2026-04-30T15:34:00Z")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = Option<String>, format = DateTime, example = "2026-04-30T15:34:00Z")]
    pub deactivated_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Canonical per-tenant repository binding record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TenantRepositoryBinding {
    pub id: String,
    pub tenant_id: TenantId,
    pub kind: RepositoryKind,
    pub local_path: Option<String>,
    pub remote_url: Option<String>,
    pub branch: String,
    pub branch_policy: BranchPolicy,
    pub credential_kind: CredentialKind,
    pub credential_ref: Option<String>,
    pub github_owner: Option<String>,
    pub github_repo: Option<String>,
    pub source_owner: RecordSource,
    /// When set, the repository resolver looks up this platform-owned
    /// connection by ID instead of parsing `credential_ref` directly.
    /// Tenant visibility is enforced at resolution time.
    pub git_provider_connection_id: Option<String>,
    #[schema(value_type = String, format = DateTime, example = "2026-04-30T15:34:00Z")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = String, format = DateTime, example = "2026-04-30T15:34:00Z")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl TenantRepositoryBinding {
    fn is_secret_reference(value: &str) -> bool {
        value.starts_with("local/") || value.starts_with("secret/") || value.starts_with("arn:aws:")
    }

    /// Returns `true` when the binding has all required fields for its `kind`.
    #[must_use]
    pub fn is_structurally_valid(&self) -> bool {
        match self.kind {
            RepositoryKind::Local => self.local_path.is_some() && self.remote_url.is_none(),
            RepositoryKind::GitHub => {
                self.remote_url.is_some()
                    && self.github_owner.is_some()
                    && self.github_repo.is_some()
                    && self.local_path.is_none()
            }
            RepositoryKind::GitRemote => self.remote_url.is_some() && self.local_path.is_none(),
        }
    }

    /// Returns a JSON representation of the binding with `credential_ref` redacted.
    #[must_use]
    pub fn redacted(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "tenantId": self.tenant_id.as_str(),
            "kind": self.kind.to_string(),
            "localPath": self.local_path,
            "remoteUrl": self.remote_url,
            "branch": self.branch,
            "branchPolicy": self.branch_policy.to_string(),
            "credentialKind": self.credential_kind.to_string(),
            "credentialRef": self.credential_ref.as_deref().map(|_| "[redacted]"),
            "gitProviderConnectionId": self.git_provider_connection_id,
            "githubOwner": self.github_owner,
            "githubRepo": self.github_repo,
            "sourceOwner": self.source_owner.to_string(),
            "createdAt": self.created_at,
            "updatedAt": self.updated_at,
        })
    }

    /// Validates that a credential reference is provided when the credential kind requires one.
    pub fn validate_credential_ref(&self) -> Result<(), String> {
        match self.credential_kind {
            CredentialKind::None => Ok(()),
            CredentialKind::PersonalAccessToken | CredentialKind::SshKey => {
                let credential_ref = self.credential_ref.as_deref().unwrap_or_default();
                if credential_ref.is_empty() {
                    Err(format!(
                        "credential_ref is required for credential_kind={}",
                        self.credential_kind
                    ))
                } else if Self::is_secret_reference(credential_ref) {
                    Ok(())
                } else {
                    Err(format!(
                        "credential_ref must be a supported secret reference (local/, secret/, arn:aws:) for credential_kind={}",
                        self.credential_kind
                    ))
                }
            }
            CredentialKind::GitHubApp => {
                // When a platform-owned connection is referenced, credential_ref
                // is not required — the resolver will look up the connection registry.
                if self.git_provider_connection_id.is_some() {
                    return Ok(());
                }
                let credential_ref = self.credential_ref.as_deref().unwrap_or_default();
                if credential_ref.is_empty() {
                    return Err(format!(
                        "credential_ref or git_provider_connection_id is required for credential_kind={}",
                        self.credential_kind
                    ));
                }

                let parts: Vec<&str> = credential_ref.splitn(3, ':').collect();
                if parts.len() != 3 || parts[0].is_empty() || parts[1].is_empty() {
                    return Err(
                        "credential_ref must use app_id:installation_id:pem_ref format for credential_kind=GitHubApp"
                            .to_string(),
                    );
                }

                if !Self::is_secret_reference(parts[2]) {
                    return Err(
                        "credential_ref pem_ref must be a supported secret reference (local/, secret/, arn:aws:) for credential_kind=GitHubApp"
                            .to_string(),
                    );
                }

                Ok(())
            }
        }
    }
}

#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, EnumString, Display,
)]
#[strum(ascii_case_insensitive)]
pub enum TenantConfigOwnership {
    Platform,
    Tenant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TenantConfigField {
    pub ownership: TenantConfigOwnership,
    pub value: serde_json::Value,
}

/// Per-tenant secret reference stored inside a [`TenantConfigDocument`].
///
/// Pairs the ownership metadata (who administers this secret \u2014 platform vs
/// tenant) with the storage-agnostic [`crate::secret::SecretReference`]
/// that points at the actual ciphertext in the configured
/// `storage::secret_backend::SecretBackend`.
///
/// Kept as a struct (rather than inlining `SecretReference` directly into
/// `secret_references` map values) because ownership is a tenant-config-level
/// concern that the secret backend itself does not know about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TenantSecretReference {
    pub logical_name: String,
    pub ownership: TenantConfigOwnership,
    #[serde(flatten)]
    pub reference: crate::secret::SecretReference,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TenantConfigDocument {
    pub tenant_id: TenantId,
    pub fields: std::collections::BTreeMap<String, TenantConfigField>,
    pub secret_references: std::collections::BTreeMap<String, TenantSecretReference>,
}

/// Caller-supplied write-side secret payload. Accepted by
/// [`crate::traits::TenantConfigProvider::set_secret_entry`]; the provider
/// writes the bytes through `SecretBackend::put` and returns a
/// [`TenantSecretReference`] for storage in the tenant config document.
///
/// `secret_value` is [`crate::SecretBytes`] (not `String`) so it zeroizes on
/// drop and never prints in plaintext via `Debug` / `Display` / `Serialize`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TenantSecretEntry {
    pub logical_name: String,
    pub ownership: TenantConfigOwnership,
    pub secret_value: crate::SecretBytes,
}

impl TenantConfigDocument {
    #[must_use]
    pub fn contains_raw_secret_material(&self) -> bool {
        self.fields
            .iter()
            .any(|(key, field)| contains_raw_secret_material(key, &field.value))
    }
}

fn contains_raw_secret_material(field_name: &str, value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter().any(|(k, v)| {
            let composite = format!("{field_name}.{k}");
            contains_raw_secret_material(&composite, v)
        }),
        serde_json::Value::Array(values) => values
            .iter()
            .any(|v| contains_raw_secret_material(field_name, v)),
        serde_json::Value::String(text) => {
            let lower = field_name.to_ascii_lowercase();
            let suspicious_key = ["secret", "password", "token", "api_key", "private_key"]
                .iter()
                .any(|needle| lower.contains(needle));
            suspicious_key && !text.trim().is_empty()
        }
        _ => false,
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, EnumString, Display,
)]
#[strum(serialize_all = "camelCase")]
pub enum UnitType {
    Organization,
    Team,
    Project,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationalUnit {
    pub id: String,
    pub name: String,
    pub unit_type: UnitType,
    pub parent_id: Option<String>,
    pub tenant_id: TenantId,
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
    #[schema(value_type = String, format = DateTime, example = "2026-04-30T15:34:00Z")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = String, format = DateTime, example = "2026-04-30T15:34:00Z")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub source_owner: RecordSource,
}

#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, JsonSchema, PartialOrd, Ord,
)]
#[serde(transparent)]
pub struct TenantId(String);

impl TenantId {
    pub fn new(id: String) -> Option<Self> {
        if id.is_empty() || id.len() > 100 {
            None
        } else {
            Some(Self(id))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Display for TenantId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for TenantId {
    fn default() -> Self {
        Self("default".to_string())
    }
}

impl std::str::FromStr for TenantId {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s.to_string()).ok_or_else(|| anyhow::anyhow!("Invalid tenant ID"))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema, Default)]
pub struct TenantContext {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub agent_id: Option<String>,
    #[serde(default)]
    pub roles: Vec<RoleIdentifier>,
    pub target_tenant_id: Option<TenantId>,
}

#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, JsonSchema, PartialOrd, Ord,
)]
#[serde(transparent)]
pub struct UserId(String);

impl UserId {
    pub fn new(id: String) -> Option<Self> {
        if id.is_empty() || id.len() > 100 {
            None
        } else {
            Some(Self(id))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for UserId {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s.to_string()).ok_or_else(|| anyhow::anyhow!("Invalid user ID"))
    }
}

impl Default for UserId {
    fn default() -> Self {
        Self("default".to_string())
    }
}

impl TenantContext {
    pub fn new(tenant_id: TenantId, user_id: UserId) -> Self {
        Self {
            tenant_id,
            user_id,
            agent_id: None,
            roles: Vec::new(),
            target_tenant_id: None,
        }
    }

    pub fn with_agent(tenant_id: TenantId, user_id: UserId, agent_id: String) -> Self {
        Self {
            tenant_id,
            user_id,
            agent_id: Some(agent_id),
            roles: Vec::new(),
            target_tenant_id: None,
        }
    }

    pub fn with_role(mut self, role: impl Into<RoleIdentifier>) -> Self {
        self.roles.push(role.into());
        self
    }

    pub fn with_roles(mut self, roles: Vec<RoleIdentifier>) -> Self {
        self.roles = roles;
        self
    }

    pub fn has_role(&self, role: &RoleIdentifier) -> bool {
        self.roles.contains(role)
    }

    pub fn has_known_role(&self, role: &Role) -> bool {
        self.roles.contains(&RoleIdentifier::Known(role.clone()))
    }

    pub fn highest_precedence_role(&self) -> Option<&RoleIdentifier> {
        self.roles.iter().max_by_key(|r| match r {
            RoleIdentifier::Known(role) => role.precedence(),
            RoleIdentifier::Custom(_) => 0,
        })
    }

    /// Sentinel context for scheduler-owned, no-human-actor work.
    ///
    /// Used by scheduled cross-tenant jobs (audit compaction, global
    /// rate-limit sweeps, \u2026) that pass through
    /// `with_admin_context`. The context carries no tenant and no human
    /// user: `tenant_id` is the instance-scope sentinel `__root__`,
    /// `user_id` is the well-known `system` constant.
    ///
    /// Callers MUST NOT use this for per-tenant scheduled work — use
    /// [`Self::from_scheduled_job`] instead so the audit trail correctly
    /// attributes the tenant.
    ///
    /// See `openspec/changes/decide-rls-enforcement-model/design.md`
    /// §4.4 for the scheduled-jobs routing pattern.
    pub fn system_ctx() -> Self {
        Self {
            tenant_id: TenantId(INSTANCE_SCOPE_TENANT_ID.to_string()),
            user_id: UserId(SYSTEM_USER_ID.to_string()),
            agent_id: None,
            roles: Vec::new(),
            target_tenant_id: None,
        }
    }

    /// Context for scheduled per-tenant work.
    ///
    /// `tenant_id` is the tenant the job is running for. `user_id`
    /// encodes the job identifier as `system:<job_id>` so the audit
    /// log can distinguish different scheduled jobs touching the same
    /// tenant without needing a separate column.
    ///
    /// Used by the scheduler after it has enumerated its target tenants
    /// through `with_admin_context(&system_ctx, \u2026)`; each per-tenant
    /// step then runs through `with_tenant_context(&ctx, \u2026)` where
    /// `ctx` comes from this constructor.
    pub fn from_scheduled_job(tenant_id: TenantId, job_id: impl Into<String>) -> Self {
        let job_id = job_id.into();
        Self {
            tenant_id,
            user_id: UserId(format!("{}:{}", SYSTEM_USER_ID, job_id)),
            agent_id: None,
            roles: Vec::new(),
            target_tenant_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct HierarchyPath {
    pub tenant: String,
    pub org: Option<String>,
    pub team: Option<String>,
    pub project: Option<String>,
}

impl HierarchyPath {
    pub fn tenant(id: String) -> Self {
        Self {
            tenant: id,
            org: None,
            team: None,
            project: None,
        }
    }

    pub fn org(tenant: String, id: String) -> Self {
        Self {
            tenant,
            org: Some(id),
            team: None,
            project: None,
        }
    }

    pub fn team(tenant: String, org: String, id: String) -> Self {
        Self {
            tenant,
            org: Some(org),
            team: Some(id),
            project: None,
        }
    }

    pub fn project(tenant: String, org: String, team: String, id: String) -> Self {
        Self {
            tenant,
            org: Some(org),
            team: Some(team),
            project: Some(id),
        }
    }

    pub fn depth(&self) -> usize {
        if self.project.is_some() {
            4
        } else if self.team.is_some() {
            3
        } else if self.org.is_some() {
            2
        } else {
            1
        }
    }

    pub fn path_string(&self) -> String {
        let mut parts = vec![self.tenant.clone()];
        if let Some(o) = &self.org {
            parts.push(o.clone());
        }
        if let Some(t) = &self.team {
            parts.push(t.clone());
        }
        if let Some(p) = &self.project {
            parts.push(p.clone());
        }
        parts.join(" > ")
    }
}

/// Wraps [`Role`] (built-in) or an arbitrary string (custom/dynamic).
/// Stored as `TEXT` in the database; mapped to Cedar `Role::"<name>"` entities.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(untagged)]
pub enum RoleIdentifier {
    Known(Role),
    Custom(String),
}

impl From<Role> for RoleIdentifier {
    fn from(role: Role) -> Self {
        Self::Known(role)
    }
}

impl RoleIdentifier {
    pub fn from_str_flexible(s: &str) -> Self {
        match s.parse::<Role>() {
            Ok(role) => Self::Known(role),
            Err(_) => Self::Custom(s.to_string()),
        }
    }

    #[must_use]
    pub fn is_known(&self) -> bool {
        matches!(self, Self::Known(_))
    }

    #[must_use]
    pub fn is_custom(&self) -> bool {
        matches!(self, Self::Custom(_))
    }

    #[must_use]
    pub fn as_known(&self) -> Option<&Role> {
        match self {
            Self::Known(r) => Some(r),
            Self::Custom(_) => None,
        }
    }

    #[must_use]
    pub fn as_cedar_entity_id(&self) -> String {
        self.to_string()
    }
}

impl std::fmt::Display for RoleIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Known(role) => write!(f, "{role}"),
            Self::Custom(name) => write!(f, "{name}"),
        }
    }
}

impl PartialEq for RoleIdentifier {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Known(a), Self::Known(b)) => a == b,
            (Self::Custom(a), Self::Custom(b)) => a == b,
            (Self::Known(role), Self::Custom(s)) | (Self::Custom(s), Self::Known(role)) => {
                role.to_string().eq_ignore_ascii_case(s)
            }
        }
    }
}

impl Eq for RoleIdentifier {}

impl std::hash::Hash for RoleIdentifier {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // PartialEq treats Known("Admin") == Custom("admin"), so hash must normalize.
        self.to_string().to_ascii_lowercase().hash(state);
    }
}

impl std::str::FromStr for RoleIdentifier {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_str_flexible(s))
    }
}

impl Default for RoleIdentifier {
    fn default() -> Self {
        Self::Known(Role::Viewer)
    }
}

impl Role {
    #[must_use]
    pub fn precedence(&self) -> u8 {
        match self {
            Role::Viewer => 0,
            Role::Agent => 0,
            Role::Developer => 1,
            Role::TechLead => 2,
            Role::Architect => 3,
            Role::Admin => 4,
            Role::PlatformAdmin => 5,
            Role::TenantAdmin => 6,
        }
    }

    #[must_use]
    pub fn display_name(&self) -> &'static str {
        match self {
            Role::Viewer => "Viewer",
            Role::Developer => "Developer",
            Role::TechLead => "Tech Lead",
            Role::Architect => "Architect",
            Role::Admin => "Admin",
            Role::TenantAdmin => "Tenant Admin",
            Role::Agent => "Agent",
            Role::PlatformAdmin => "Platform Admin",
        }
    }
}

/// Knowledge types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, ToSchema)]
pub enum KnowledgeType {
    Adr,
    Policy,
    Pattern,
    Spec,
    Hindsight,
}

/// Knowledge status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, ToSchema)]
pub enum KnowledgeStatus {
    Draft,
    Proposed,
    Accepted,
    Deprecated,
    Superseded,
    Rejected,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
    ToSchema,
    Display,
    EnumString,
    Default,
)]
#[strum(ascii_case_insensitive)]
pub enum KnowledgeVariantRole {
    #[default]
    Canonical,
    Specialization,
    Applicability,
    Exception,
    Clarification,
    /// Task 10.4: marker for historically superseded or deprecated items during migration
    Superseded,
}

impl KnowledgeVariantRole {
    /// Lower rank value = higher retrieval priority.
    /// Used by the resolver to sort search results so Canonical surfaces first.
    #[must_use]
    pub fn rank(&self) -> u8 {
        match self {
            KnowledgeVariantRole::Canonical => 0,
            KnowledgeVariantRole::Specialization => 1,
            KnowledgeVariantRole::Applicability => 2,
            KnowledgeVariantRole::Clarification => 3,
            KnowledgeVariantRole::Exception => 4,
            KnowledgeVariantRole::Superseded => 5,
        }
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
    ToSchema,
    Display,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
pub enum KnowledgeRelationType {
    PromotedFrom,
    PromotedTo,
    Specializes,
    ApplicableFrom,
    ExceptionTo,
    Clarifies,
    Supersedes,
    SupersededBy,
    DerivedFrom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeRelation {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation_type: KnowledgeRelationType,
    pub tenant_id: TenantId,
    pub created_by: UserId,
    pub created_at: i64,
    #[serde(default)]
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
    ToSchema,
    Display,
    EnumString,
    Default,
)]
#[strum(ascii_case_insensitive)]
pub enum PromotionMode {
    Full,
    #[default]
    Partial,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
    ToSchema,
    Display,
    EnumString,
    Default,
)]
#[strum(ascii_case_insensitive)]
pub enum PromotionRequestStatus {
    Draft,
    #[default]
    PendingReview,
    Approved,
    Rejected,
    Applied,
    Cancelled,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
    ToSchema,
    Display,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
pub enum PromotionDecision {
    ApproveAsReplacement,
    ApproveAsSpecialization,
    ApproveAsApplicability,
    ApproveAsException,
    ApproveAsClarification,
    Reject,
    NeedsRefinement,
    RetargetLayer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromotionRequest {
    pub id: String,
    pub source_item_id: String,
    pub source_layer: KnowledgeLayer,
    pub source_status: KnowledgeStatus,
    pub target_layer: KnowledgeLayer,
    #[serde(default)]
    pub promotion_mode: PromotionMode,
    pub shared_content: String,
    pub residual_content: Option<String>,
    pub residual_role: Option<KnowledgeVariantRole>,
    pub justification: Option<String>,
    #[serde(default)]
    pub status: PromotionRequestStatus,
    pub requested_by: UserId,
    pub tenant_id: TenantId,
    pub source_version: String,
    pub latest_decision: Option<PromotionDecision>,
    pub promoted_item_id: Option<String>,
    pub residual_item_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
    PartialOrd,
    Ord,
    JsonSchema,
)]
pub enum KnowledgeLayer {
    Tenant,
    Org,
    Team,
    Project,
}

impl KnowledgeLayer {
    #[must_use]
    pub fn precedence(&self) -> u8 {
        match self {
            KnowledgeLayer::Project => 1,
            KnowledgeLayer::Team => 2,
            KnowledgeLayer::Org => 3,
            KnowledgeLayer::Tenant => 4,
        }
    }
}

impl PromotionRequest {
    pub fn validate_layer_direction(&self) -> Result<(), String> {
        if self.source_layer == KnowledgeLayer::Tenant {
            return Err("tenant-layer knowledge cannot be promoted higher".to_string());
        }

        if self.target_layer.precedence() <= self.source_layer.precedence() {
            return Err("target_layer must be strictly higher than source_layer".to_string());
        }

        if self.source_status != KnowledgeStatus::Accepted {
            return Err("only accepted knowledge can be promoted".to_string());
        }

        Ok(())
    }
}

impl From<MemoryLayer> for Option<KnowledgeLayer> {
    fn from(layer: MemoryLayer) -> Self {
        match layer {
            MemoryLayer::Tenant => Some(KnowledgeLayer::Tenant),
            MemoryLayer::Org => Some(KnowledgeLayer::Org),
            MemoryLayer::Team => Some(KnowledgeLayer::Team),
            MemoryLayer::Project => Some(KnowledgeLayer::Project),
            _ => None,
        }
    }
}

/// Constraint severity levels
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    strum::Display,
    strum::EnumString,
)]
pub enum ConstraintSeverity {
    Info,
    Warn,
    Block,
}

/// Constraint operators
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, ToSchema)]
pub enum ConstraintOperator {
    MustUse,
    MustNotUse,
    MustMatch,
    MustNotMatch,
    MustExist,
    MustNotExist,
}

/// Constraint targets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, ToSchema)]
pub enum ConstraintTarget {
    File,
    Code,
    Dependency,
    Import,
    Config,
}

/// Memory layers for hierarchical storage
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
    strum::EnumString,
    strum::Display,
    ToSchema,
)]
pub enum MemoryLayer {
    Agent,
    User,
    Session,
    Project,
    Team,
    Org,
    Tenant,
}

impl MemoryLayer {
    #[must_use]
    pub fn precedence(&self) -> u8 {
        match self {
            MemoryLayer::Agent => 1,
            MemoryLayer::User => 2,
            MemoryLayer::Session => 3,
            MemoryLayer::Project => 4,
            MemoryLayer::Team => 5,
            MemoryLayer::Org => 6,
            MemoryLayer::Tenant => 7,
        }
    }

    #[must_use]
    pub fn display_name(&self) -> &'static str {
        match self {
            MemoryLayer::Agent => "Agent",
            MemoryLayer::User => "User",
            MemoryLayer::Session => "Session",
            MemoryLayer::Project => "Project",
            MemoryLayer::Team => "Team",
            MemoryLayer::Org => "Organization",
            MemoryLayer::Tenant => "Tenant",
        }
    }
}

#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Validate, Default, ToSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct LayerIdentifiers {
    #[validate(custom(function = "validate_agent_id"))]
    pub agent_id: Option<String>,
    #[validate(custom(function = "validate_user_id"))]
    pub user_id: Option<String>,
    #[validate(custom(function = "validate_session_id"))]
    pub session_id: Option<String>,
    #[validate(custom(function = "validate_project_id"))]
    pub project_id: Option<String>,
    #[validate(custom(function = "validate_team_id"))]
    pub team_id: Option<String>,
    #[validate(custom(function = "validate_org_id"))]
    pub org_id: Option<String>,
    #[validate(custom(function = "validate_tenant_id"))]
    pub tenant_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, ToSchema)]
pub enum SummaryDepth {
    Sentence,
    Paragraph,
    Detailed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LayerSummary {
    pub depth: SummaryDepth,
    pub content: String,
    pub token_count: u32,
    pub generated_at: i64,
    pub source_hash: String,
    pub content_hash: Option<String>,
    pub personalized: bool,
    pub personalization_context: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SummaryConfig {
    pub layer: MemoryLayer,
    pub update_interval_secs: Option<u64>,
    pub update_on_changes: Option<u32>,
    pub skip_if_unchanged: bool,
    pub personalized: bool,
    pub depths: Vec<SummaryDepth>,
}

pub type ContextVector = Vec<f32>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorSignature {
    pub error_type: String,
    pub message_pattern: String,
    pub stack_patterns: Vec<String>,
    pub context_patterns: Vec<String>,
    pub embedding: Option<Vec<f32>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodeChange {
    pub file_path: String,
    pub diff: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    pub id: String,
    pub error_signature_id: String,
    pub description: String,
    pub changes: Vec<CodeChange>,
    pub success_rate: f32,
    pub application_count: u32,
    pub last_success_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HindsightNote {
    pub id: String,
    pub error_signature: ErrorSignature,
    pub resolutions: Vec<Resolution>,
    pub content: String,
    pub tags: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
    ToSchema,
    Display,
    EnumString,
)]
#[strum(serialize_all = "camelCase")]
pub enum ReasoningStrategy {
    Exhaustive,
    Targeted,
    SemanticOnly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningTrace {
    pub strategy: ReasoningStrategy,
    pub thought_process: String,
    pub refined_query: Option<String>,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub end_time: chrono::DateTime<chrono::Utc>,
    /// Indicates if reasoning was interrupted by timeout (partial results may
    /// be available)
    #[serde(default)]
    pub timed_out: bool,
    /// Duration of the reasoning step in milliseconds
    #[serde(default)]
    pub duration_ms: u64,
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    pub id: String,
    pub content: String,
    pub embedding: Option<Vec<f32>>,
    pub layer: MemoryLayer,
    pub summaries: std::collections::HashMap<SummaryDepth, LayerSummary>,
    pub context_vector: Option<ContextVector>,
    pub importance_score: Option<f32>,
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Default for MemoryEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            content: String::new(),
            embedding: None,
            layer: MemoryLayer::Project,
            summaries: std::collections::HashMap::new(),
            context_vector: None,
            importance_score: None,
            metadata: std::collections::HashMap::new(),
            created_at: 0,
            updated_at: 0,
        }
    }
}

impl MemoryEntry {
    pub fn needs_summary_update(&self, config: &SummaryConfig, current_time: i64) -> bool {
        use sha2::{Digest, Sha256};

        for depth in &config.depths {
            if let Some(summary) = self.summaries.get(depth) {
                let content_hash = hex::encode(Sha256::digest(self.content.as_bytes()));
                let content_changed = summary.source_hash != content_hash;

                if content_changed {
                    return true;
                }

                if config.skip_if_unchanged {
                    continue;
                }

                if let Some(interval_secs) = config.update_interval_secs {
                    let elapsed = current_time - summary.generated_at;
                    if elapsed >= interval_secs as i64 {
                        return true;
                    }
                }
            } else {
                return true;
            }
        }
        false
    }

    pub fn compute_content_hash(&self) -> String {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(self.content.as_bytes()))
    }

    pub fn compute_content_hash_xxh64(&self) -> String {
        compute_xxhash64(self.content.as_bytes())
    }
}

pub fn compute_xxhash64(data: &[u8]) -> String {
    use xxhash_rust::xxh64::xxh64;
    format!("{:016x}", xxh64(data, 0))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub enum MemoryOperation {
    Add,
    Update,
    Delete,
    Retrieve,
    Prune,
    Compress,
    Noop,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
    ToSchema,
    Display,
    EnumString,
)]
#[strum(serialize_all = "camelCase")]
pub enum RewardType {
    Helpful,
    Irrelevant,
    Outdated,
    Inaccurate,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RewardSignal {
    pub reward_type: RewardType,
    pub score: f32, // -1.0 to 1.0
    pub reasoning: Option<String>,
    pub agent_id: Option<String>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub text: String,
    pub target_layers: Vec<MemoryLayer>,
    pub filters: std::collections::HashMap<String, serde_json::Value>,
    pub limit: usize,
    pub threshold: f32,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            text: String::new(),
            target_layers: Vec::new(),
            filters: std::collections::HashMap::new(),
            limit: 10,
            threshold: 0.5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub memory_id: String,
    pub content: String,
    pub score: f32,
    pub layer: MemoryLayer,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryTrajectoryEvent {
    pub operation: MemoryOperation,
    pub entry_id: String,
    pub reward: Option<RewardSignal>,
    pub reasoning: Option<String>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Entity {
    pub id: String,
    pub name: String,
    pub entity_type: String,
    pub description: Option<String>,
    pub embedding: Option<Vec<f32>>,
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Relationship {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation_type: String,
    pub weight: f32,
    pub description: Option<String>,
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Community {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub level: u32,
    pub entity_ids: Vec<String>,
    pub relationship_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeEntry {
    pub path: String,
    pub content: String,
    pub layer: KnowledgeLayer,
    pub kind: KnowledgeType,
    pub status: KnowledgeStatus,
    pub summaries: std::collections::HashMap<SummaryDepth, LayerSummary>,
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
    pub commit_hash: Option<String>,
    pub author: Option<String>,
    pub updated_at: i64,
}

impl KnowledgeEntry {
    /// Extract the `variant_role` stored in `metadata["variant_role"]`.
    /// Falls back to `Canonical` when the field is absent or unparseable,
    /// matching the migration decision that all existing accepted items are
    /// treated as canonical.
    pub fn variant_role(&self) -> KnowledgeVariantRole {
        self.metadata
            .get("variant_role")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(KnowledgeVariantRole::Canonical)
    }

    /// Returns the numeric precedence weight for this entry's variant role.
    /// Higher is more authoritative and should appear earlier in search results.
    ///
    /// Order (descending priority):
    ///   Canonical (5) > Clarification (4) > Specialization (3)
    ///               > Applicability (2)  > Exception (1)
    pub fn variant_precedence(&self) -> u8 {
        match self.variant_role() {
            KnowledgeVariantRole::Canonical => 5,
            KnowledgeVariantRole::Clarification => 4,
            KnowledgeVariantRole::Specialization => 3,
            KnowledgeVariantRole::Applicability => 2,
            KnowledgeVariantRole::Exception => 1,
            // Superseded items are lowest priority in precedence — shown last
            KnowledgeVariantRole::Superseded => 0,
        }
    }
}

/// A knowledge entry together with its stored semantic relations.
///
/// Used as the enriched unit returned by relation-aware search / query paths
/// (task 6.3).  The `relations` vec is populated on a best-effort basis;
/// callers should treat an empty vec as "no relations loaded" rather than
/// "no relations exist".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeEntryWithRelations {
    #[serde(flatten)]
    pub entry: KnowledgeEntry,
    /// Semantic relations where this entry is either source or target.
    pub relations: Vec<KnowledgeRelation>,
}

impl KnowledgeEntryWithRelations {
    pub fn new(entry: KnowledgeEntry, relations: Vec<KnowledgeRelation>) -> Self {
        Self { entry, relations }
    }

    pub fn without_relations(entry: KnowledgeEntry) -> Self {
        Self {
            entry,
            relations: vec![],
        }
    }
}

/// The enriched result returned by `KnowledgeManager::query_enriched`.
///
/// Each item in the result set is grouped as a canonical entry (or a
/// standalone residual with no canonical parent) plus the local residual
/// items that are semantically related to it (task 6.2).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeQueryResult {
    /// The primary entry for this group.  For canonical entries this is the
    /// promoted item at the highest applicable layer.  For unrelated entries
    /// it is the entry itself.
    pub primary: KnowledgeEntryWithRelations,
    /// Local residual items related to `primary` via Specializes,
    /// ApplicableFrom, ExceptionTo, or Clarifies relations.
    pub local_residuals: Vec<(KnowledgeRelationType, KnowledgeEntryWithRelations)>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default, ToSchema,
)]
pub enum PolicyMode {
    #[default]
    Optional,
    Mandatory,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default, ToSchema,
)]
pub enum RuleMergeStrategy {
    #[default]
    Override,
    Merge,
    Intersect,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default, ToSchema,
)]
pub enum RuleType {
    #[default]
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub layer: KnowledgeLayer,
    #[serde(default)]
    pub mode: PolicyMode,
    #[serde(default)]
    pub merge_strategy: RuleMergeStrategy,
    pub rules: Vec<PolicyRule>,
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PolicyRule {
    pub id: String,
    #[serde(default)]
    pub rule_type: RuleType,
    pub target: ConstraintTarget,
    pub operator: ConstraintOperator,
    pub value: serde_json::Value,
    pub severity: ConstraintSeverity,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidationResult {
    pub is_valid: bool,
    pub violations: Vec<PolicyViolation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PolicyViolation {
    pub rule_id: String,
    pub policy_id: String,
    pub severity: ConstraintSeverity,
    pub message: String,
    pub context: std::collections::HashMap<String, serde_json::Value>,
}

/// Wire-safe snapshot of a single bootstrap phase.
///
/// Mirrors the struct produced by
/// `cli::server::bootstrap_tracker::BootstrapTracker::snapshot()` and
/// lives in `mk_core` so event consumers (Postgres event store, Redis
/// publisher, downstream audit tooling) can deserialize it without a
/// dependency on the CLI crate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapStepSnapshot {
    pub name: String,
    /// `running`, `success`, or `failure`.
    pub state: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Present only when `state == "failure"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Wire-safe snapshot of bootstrap progress, returned verbatim by
/// `GET /api/v1/admin/bootstrap/status` and embedded as the payload of
/// [`GovernanceEvent::BootstrapCompleted`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapStatusSnapshot {
    /// `running`, `completed`, or `failed`.
    pub state: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub steps: Vec<BootstrapStepSnapshot>,
}

/// Governance event types for auditing and real-time updates
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub enum GovernanceEvent {
    /// New organizational unit created
    UnitCreated {
        unit_id: String,
        unit_type: UnitType,
        tenant_id: TenantId,
        parent_id: Option<String>,
        timestamp: i64,
    },

    /// Organizational unit updated
    UnitUpdated {
        unit_id: String,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Organizational unit deleted
    UnitDeleted {
        unit_id: String,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Role assigned to a user for a specific unit
    RoleAssigned {
        user_id: UserId,
        unit_id: String,
        role: RoleIdentifier,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Role removed from a user
    RoleRemoved {
        user_id: UserId,
        unit_id: String,
        role: RoleIdentifier,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Policy created or updated
    PolicyUpdated {
        policy_id: String,
        layer: KnowledgeLayer,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Policy deleted
    PolicyDeleted {
        policy_id: String,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Drift detected in a project
    DriftDetected {
        project_id: String,
        tenant_id: TenantId,
        drift_score: f32,
        timestamp: i64,
    },

    /// Governance configuration updated
    ConfigUpdated {
        config_id: String,
        scope: String,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Governance approval request created
    RequestCreated {
        request_id: String,
        request_type: String,
        title: String,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Governance approval request approved
    RequestApproved {
        request_id: String,
        approver_id: String,
        fully_approved: bool,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Governance approval request rejected
    RequestRejected {
        request_id: String,
        rejector_id: String,
        reason: String,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Tenant lifecycle: tenant record created
    TenantCreated {
        record_id: String,
        slug: String,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Tenant lifecycle: tenant record updated
    TenantUpdated {
        record_id: String,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Tenant lifecycle: tenant deactivated
    TenantDeactivated {
        record_id: String,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Repository binding created for a tenant
    RepositoryBindingCreated {
        binding_id: String,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Repository binding updated for a tenant
    RepositoryBindingUpdated {
        binding_id: String,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Git provider connection created (platform-level event)
    GitProviderConnectionCreated {
        connection_id: String,
        /// Platform-level events use a synthetic sentinel TenantId.
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Git provider connection updated (platform-level event)
    GitProviderConnectionUpdated {
        connection_id: String,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Tenant granted visibility of a Git provider connection
    GitProviderConnectionTenantGranted {
        connection_id: String,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Tenant revoked from a Git provider connection
    GitProviderConnectionTenantRevoked {
        connection_id: String,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Knowledge promotion request submitted
    KnowledgePromotionRequested {
        promotion_id: String,
        source_item_id: String,
        source_layer: KnowledgeLayer,
        target_layer: KnowledgeLayer,
        /// Task 9.7: audit metadata — promotion split mode
        promotion_mode: PromotionMode,
        /// Task 9.7: audit metadata — proposer's justification text
        justification: Option<String>,
        requested_by: UserId,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Knowledge promotion approved
    KnowledgePromotionApproved {
        promotion_id: String,
        decision: PromotionDecision,
        approved_by: UserId,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Knowledge promotion rejected
    KnowledgePromotionRejected {
        promotion_id: String,
        reason: String,
        rejected_by: UserId,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Knowledge promotion retargeted to a new layer
    KnowledgePromotionRetargeted {
        promotion_id: String,
        new_target_layer: KnowledgeLayer,
        retargeted_by: UserId,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Knowledge promotion applied (promoted item stored at target layer)
    KnowledgePromotionApplied {
        promotion_id: String,
        promoted_item_id: String,
        residual_item_id: Option<String>,
        /// Task 9.7: audit metadata — split mode used (Full vs Partial)
        promotion_mode: PromotionMode,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Semantic relation created between knowledge items
    KnowledgeRelationCreated {
        relation_id: String,
        source_id: String,
        target_id: String,
        relation_type: KnowledgeRelationType,
        tenant_id: TenantId,
        timestamp: i64,
    },

    /// Platform bootstrap completed successfully on this pod.
    ///
    /// Emitted exactly once per pod boot, right after `bootstrap()`
    /// returns the fully-assembled `AppState` and the tracker has been
    /// finalized with `mark_ready()`. Consumers get the complete
    /// per-phase breakdown verbatim from `/api/v1/admin/bootstrap/status`
    /// without a follow-up call.
    ///
    /// This is a **platform-level** event: `tenant_id` is always the
    /// root sentinel [`INSTANCE_SCOPE_TENANT_ID`], because bootstrap is
    /// not tenant-scoped (it runs before any tenant context exists).
    ///
    /// Tracking: B2 task 6.4 in
    /// `openspec/changes/harden-tenant-provisioning/tasks.md`.
    BootstrapCompleted {
        /// Always [`INSTANCE_SCOPE_TENANT_ID`] (`__root__`).
        tenant_id: TenantId,
        /// Unix seconds — matches every other variant for wire
        /// consistency, even though `snapshot.completed_at` is the
        /// authoritative completion time (millisecond precision).
        timestamp: i64,
        /// Full per-phase snapshot, identical in shape to
        /// `/api/v1/admin/bootstrap/status`.
        snapshot: BootstrapStatusSnapshot,
    },
}

impl GovernanceEvent {
    #[must_use]
    pub fn tenant_id(&self) -> &TenantId {
        match self {
            GovernanceEvent::UnitCreated { tenant_id, .. } => tenant_id,
            GovernanceEvent::UnitUpdated { tenant_id, .. } => tenant_id,
            GovernanceEvent::UnitDeleted { tenant_id, .. } => tenant_id,
            GovernanceEvent::RoleAssigned { tenant_id, .. } => tenant_id,
            GovernanceEvent::RoleRemoved { tenant_id, .. } => tenant_id,
            GovernanceEvent::PolicyUpdated { tenant_id, .. } => tenant_id,
            GovernanceEvent::PolicyDeleted { tenant_id, .. } => tenant_id,
            GovernanceEvent::DriftDetected { tenant_id, .. } => tenant_id,
            GovernanceEvent::ConfigUpdated { tenant_id, .. } => tenant_id,
            GovernanceEvent::RequestCreated { tenant_id, .. } => tenant_id,
            GovernanceEvent::RequestApproved { tenant_id, .. } => tenant_id,
            GovernanceEvent::RequestRejected { tenant_id, .. } => tenant_id,
            GovernanceEvent::TenantCreated { tenant_id, .. } => tenant_id,
            GovernanceEvent::TenantUpdated { tenant_id, .. } => tenant_id,
            GovernanceEvent::TenantDeactivated { tenant_id, .. } => tenant_id,
            GovernanceEvent::RepositoryBindingCreated { tenant_id, .. } => tenant_id,
            GovernanceEvent::RepositoryBindingUpdated { tenant_id, .. } => tenant_id,
            GovernanceEvent::GitProviderConnectionCreated { tenant_id, .. } => tenant_id,
            GovernanceEvent::GitProviderConnectionUpdated { tenant_id, .. } => tenant_id,
            GovernanceEvent::GitProviderConnectionTenantGranted { tenant_id, .. } => tenant_id,
            GovernanceEvent::GitProviderConnectionTenantRevoked { tenant_id, .. } => tenant_id,
            GovernanceEvent::KnowledgePromotionRequested { tenant_id, .. } => tenant_id,
            GovernanceEvent::KnowledgePromotionApproved { tenant_id, .. } => tenant_id,
            GovernanceEvent::KnowledgePromotionRejected { tenant_id, .. } => tenant_id,
            GovernanceEvent::KnowledgePromotionRetargeted { tenant_id, .. } => tenant_id,
            GovernanceEvent::KnowledgePromotionApplied { tenant_id, .. } => tenant_id,
            GovernanceEvent::KnowledgeRelationCreated { tenant_id, .. } => tenant_id,
            GovernanceEvent::BootstrapCompleted { tenant_id, .. } => tenant_id,
        }
    }
}

/// Drift analysis result with confidence scoring
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DriftResult {
    pub project_id: String,
    pub tenant_id: TenantId,
    pub drift_score: f32,
    pub confidence: f32,
    pub violations: Vec<PolicyViolation>,
    pub suppressed_violations: Vec<PolicyViolation>,
    pub requires_manual_review: bool,
    pub timestamp: i64,
}

impl DriftResult {
    pub fn new(project_id: String, tenant_id: TenantId, violations: Vec<PolicyViolation>) -> Self {
        let drift_score = Self::calculate_score(&violations);
        Self {
            project_id,
            tenant_id,
            drift_score,
            confidence: 1.0,
            violations,
            suppressed_violations: Vec::new(),
            requires_manual_review: false,
            timestamp: chrono::Utc::now().timestamp(),
        }
    }

    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self.requires_manual_review = self.confidence < 0.7;
        self
    }

    pub fn with_suppressions(mut self, suppressed: Vec<PolicyViolation>) -> Self {
        self.suppressed_violations = suppressed;
        self
    }

    fn calculate_score(violations: &[PolicyViolation]) -> f32 {
        if violations.is_empty() {
            return 0.0;
        }
        violations
            .iter()
            .map(|v| match v.severity {
                ConstraintSeverity::Block => 1.0,
                ConstraintSeverity::Warn => 0.5,
                ConstraintSeverity::Info => 0.1,
            })
            .sum::<f32>()
            .min(1.0)
    }

    pub fn active_violation_count(&self) -> usize {
        self.violations.len()
    }

    pub fn suppressed_count(&self) -> usize {
        self.suppressed_violations.len()
    }
}

/// Drift suppression rule to ignore specific violations
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DriftSuppression {
    pub id: String,
    pub project_id: String,
    pub tenant_id: TenantId,
    pub policy_id: String,
    pub rule_pattern: Option<String>,
    pub reason: String,
    pub created_by: UserId,
    pub expires_at: Option<i64>,
    pub created_at: i64,
}

impl DriftSuppression {
    pub fn new(
        project_id: String,
        tenant_id: TenantId,
        policy_id: String,
        reason: String,
        created_by: UserId,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            project_id,
            tenant_id,
            policy_id,
            rule_pattern: None,
            reason,
            created_by,
            expires_at: None,
            created_at: chrono::Utc::now().timestamp(),
        }
    }

    pub fn with_pattern(mut self, pattern: String) -> Self {
        self.rule_pattern = Some(pattern);
        self
    }

    pub fn with_expiry(mut self, expires_at: i64) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    pub fn is_expired(&self) -> bool {
        if let Some(expires) = self.expires_at {
            chrono::Utc::now().timestamp() > expires
        } else {
            false
        }
    }

    pub fn matches(&self, violation: &PolicyViolation) -> bool {
        if self.policy_id != violation.policy_id {
            return false;
        }
        if let Some(pattern) = &self.rule_pattern
            && let Ok(re) = regex::Regex::new(pattern)
        {
            return re.is_match(&violation.message);
        }
        true
    }
}

/// Drift threshold configuration per project
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DriftConfig {
    pub project_id: String,
    pub tenant_id: TenantId,
    pub threshold: f32,
    pub low_confidence_threshold: f32,
    pub auto_suppress_info: bool,
    pub updated_at: i64,
}

impl Default for DriftConfig {
    fn default() -> Self {
        Self {
            project_id: String::new(),
            tenant_id: TenantId::default(),
            threshold: 0.2,
            low_confidence_threshold: 0.7,
            auto_suppress_info: false,
            updated_at: chrono::Utc::now().timestamp(),
        }
    }
}

impl DriftConfig {
    pub fn new(project_id: String, tenant_id: TenantId) -> Self {
        Self {
            project_id,
            tenant_id,
            ..Default::default()
        }
    }

    pub fn for_project(project_id: String, tenant_id: TenantId) -> Self {
        Self::new(project_id, tenant_id)
    }

    pub fn with_threshold(mut self, threshold: f32) -> Self {
        self.threshold = threshold.clamp(0.0, 1.0);
        self
    }
}

pub fn validate_user_id(id: &&String) -> Result<(), validator::ValidationError> {
    if id.is_empty() {
        return Err(validator::ValidationError::new("User ID cannot be empty"));
    }
    if id.len() > 100 {
        return Err(validator::ValidationError::new("User ID is too long"));
    }
    Ok(())
}

pub fn validate_session_id(id: &&String) -> Result<(), validator::ValidationError> {
    if id.is_empty() {
        return Err(validator::ValidationError::new(
            "Session ID cannot be empty",
        ));
    }
    Ok(())
}

pub fn validate_project_id(id: &&String) -> Result<(), validator::ValidationError> {
    if id.is_empty() {
        return Err(validator::ValidationError::new(
            "Project ID cannot be empty",
        ));
    }
    Ok(())
}

pub fn validate_team_id(id: &&String) -> Result<(), validator::ValidationError> {
    if id.is_empty() {
        return Err(validator::ValidationError::new("Team ID cannot be empty"));
    }
    Ok(())
}

pub fn validate_org_id(id: &&String) -> Result<(), validator::ValidationError> {
    if id.is_empty() {
        return Err(validator::ValidationError::new("Org ID cannot be empty"));
    }
    Ok(())
}

pub fn validate_tenant_id(id: &&String) -> Result<(), validator::ValidationError> {
    if id.is_empty() {
        return Err(validator::ValidationError::new("Tenant ID cannot be empty"));
    }
    Ok(())
}

pub fn validate_agent_id(id: &&String) -> Result<(), validator::ValidationError> {
    if id.is_empty() {
        return Err(validator::ValidationError::new("Agent ID cannot be empty"));
    }
    if id.len() > 100 {
        return Err(validator::ValidationError::new("Agent ID is too long"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Pending,
    Published,
    Acknowledged,
    DeadLettered,
}

impl std::fmt::Display for EventStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventStatus::Pending => write!(f, "pending"),
            EventStatus::Published => write!(f, "published"),
            EventStatus::Acknowledged => write!(f, "acknowledged"),
            EventStatus::DeadLettered => write!(f, "dead_lettered"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PersistentEvent {
    pub id: String,
    pub event_id: String,
    pub idempotency_key: String,
    pub tenant_id: TenantId,
    pub event_type: String,
    pub payload: GovernanceEvent,
    pub status: EventStatus,
    pub retry_count: i32,
    pub max_retries: i32,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub published_at: Option<i64>,
    pub acknowledged_at: Option<i64>,
    pub dead_lettered_at: Option<i64>,
}

impl PersistentEvent {
    pub fn new(event: GovernanceEvent) -> Self {
        let event_id = uuid::Uuid::new_v4().to_string();
        let timestamp = chrono::Utc::now().timestamp();
        let tenant_id = event.tenant_id().clone();
        let idempotency_key = Self::calculate_idempotency_key(&event_id, timestamp, &tenant_id);
        let event_type = Self::event_type_name(&event);

        Self {
            id: uuid::Uuid::new_v4().to_string(),
            event_id,
            idempotency_key,
            tenant_id,
            event_type,
            payload: event,
            status: EventStatus::Pending,
            retry_count: 0,
            max_retries: 3,
            last_error: None,
            created_at: timestamp,
            published_at: None,
            acknowledged_at: None,
            dead_lettered_at: None,
        }
    }

    fn calculate_idempotency_key(event_id: &str, timestamp: i64, tenant_id: &TenantId) -> String {
        use sha2::{Digest, Sha256};
        let input = format!("{}:{}:{}", event_id, timestamp, tenant_id.as_str());
        let hash = Sha256::digest(input.as_bytes());
        hex::encode(hash)
    }

    fn event_type_name(event: &GovernanceEvent) -> String {
        match event {
            GovernanceEvent::UnitCreated { .. } => "unit_created".to_string(),
            GovernanceEvent::UnitUpdated { .. } => "unit_updated".to_string(),
            GovernanceEvent::UnitDeleted { .. } => "unit_deleted".to_string(),
            GovernanceEvent::RoleAssigned { .. } => "role_assigned".to_string(),
            GovernanceEvent::RoleRemoved { .. } => "role_removed".to_string(),
            GovernanceEvent::PolicyUpdated { .. } => "policy_updated".to_string(),
            GovernanceEvent::PolicyDeleted { .. } => "policy_deleted".to_string(),
            GovernanceEvent::DriftDetected { .. } => "drift_detected".to_string(),
            GovernanceEvent::ConfigUpdated { .. } => "config_updated".to_string(),
            GovernanceEvent::RequestCreated { .. } => "request_created".to_string(),
            GovernanceEvent::RequestApproved { .. } => "request_approved".to_string(),
            GovernanceEvent::RequestRejected { .. } => "request_rejected".to_string(),
            GovernanceEvent::TenantCreated { .. } => "tenant_created".to_string(),
            GovernanceEvent::TenantUpdated { .. } => "tenant_updated".to_string(),
            GovernanceEvent::TenantDeactivated { .. } => "tenant_deactivated".to_string(),
            GovernanceEvent::RepositoryBindingCreated { .. } => {
                "repository_binding_created".to_string()
            }
            GovernanceEvent::RepositoryBindingUpdated { .. } => {
                "repository_binding_updated".to_string()
            }
            GovernanceEvent::GitProviderConnectionCreated { .. } => {
                "git_provider_connection_created".to_string()
            }
            GovernanceEvent::GitProviderConnectionUpdated { .. } => {
                "git_provider_connection_updated".to_string()
            }
            GovernanceEvent::GitProviderConnectionTenantGranted { .. } => {
                "git_provider_connection_tenant_granted".to_string()
            }
            GovernanceEvent::GitProviderConnectionTenantRevoked { .. } => {
                "git_provider_connection_tenant_revoked".to_string()
            }
            GovernanceEvent::KnowledgePromotionRequested { .. } => {
                "knowledge_promotion_requested".to_string()
            }
            GovernanceEvent::KnowledgePromotionApproved { .. } => {
                "knowledge_promotion_approved".to_string()
            }
            GovernanceEvent::KnowledgePromotionRejected { .. } => {
                "knowledge_promotion_rejected".to_string()
            }
            GovernanceEvent::KnowledgePromotionRetargeted { .. } => {
                "knowledge_promotion_retargeted".to_string()
            }
            GovernanceEvent::KnowledgePromotionApplied { .. } => {
                "knowledge_promotion_applied".to_string()
            }
            GovernanceEvent::KnowledgeRelationCreated { .. } => {
                "knowledge_relation_created".to_string()
            }
            GovernanceEvent::BootstrapCompleted { .. } => "bootstrap_completed".to_string(),
        }
    }

    pub fn mark_published(&mut self) {
        self.status = EventStatus::Published;
        self.published_at = Some(chrono::Utc::now().timestamp());
    }

    pub fn mark_acknowledged(&mut self) {
        self.status = EventStatus::Acknowledged;
        self.acknowledged_at = Some(chrono::Utc::now().timestamp());
    }

    pub fn mark_failed(&mut self, error: String) -> bool {
        self.retry_count += 1;
        self.last_error = Some(error);

        if self.retry_count >= self.max_retries {
            self.status = EventStatus::DeadLettered;
            self.dead_lettered_at = Some(chrono::Utc::now().timestamp());
            false
        } else {
            self.status = EventStatus::Pending;
            true
        }
    }

    pub fn is_retriable(&self) -> bool {
        self.retry_count < self.max_retries && self.status == EventStatus::Pending
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventDeliveryMetrics {
    pub tenant_id: TenantId,
    pub event_type: String,
    pub period_start: i64,
    pub period_end: i64,
    pub total_events: i64,
    pub delivered_events: i64,
    pub retried_events: i64,
    pub dead_lettered_events: i64,
    pub avg_delivery_time_ms: Option<f64>,
}

impl EventDeliveryMetrics {
    pub fn new(
        tenant_id: TenantId,
        event_type: String,
        period_start: i64,
        period_end: i64,
    ) -> Self {
        Self {
            tenant_id,
            event_type,
            period_start,
            period_end,
            total_events: 0,
            delivered_events: 0,
            retried_events: 0,
            dead_lettered_events: 0,
            avg_delivery_time_ms: None,
        }
    }

    pub fn delivery_success_rate(&self) -> f64 {
        if self.total_events == 0 {
            return 1.0;
        }
        self.delivered_events as f64 / self.total_events as f64
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsumerState {
    pub consumer_group: String,
    pub idempotency_key: String,
    pub tenant_id: TenantId,
    pub processed_at: i64,
}

impl ConsumerState {
    pub fn new(consumer_group: String, idempotency_key: String, tenant_id: TenantId) -> Self {
        Self {
            consumer_group,
            idempotency_key,
            tenant_id,
            processed_at: chrono::Utc::now().timestamp(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobCoordinationMetrics {
    pub job_name: String,
    pub tenant_id: TenantId,
    pub total_runs: u64,
    pub successful_runs: u64,
    pub failed_runs: u64,
    pub skipped_runs: u64,
    pub timeout_count: u64,
    pub total_duration_ms: u64,
    pub last_run_at: Option<i64>,
    pub last_success_at: Option<i64>,
}

impl JobCoordinationMetrics {
    pub fn new(job_name: String, tenant_id: TenantId) -> Self {
        Self {
            job_name,
            tenant_id,
            total_runs: 0,
            successful_runs: 0,
            failed_runs: 0,
            skipped_runs: 0,
            timeout_count: 0,
            total_duration_ms: 0,
            last_run_at: None,
            last_success_at: None,
        }
    }

    pub fn record_run(&mut self, duration_ms: u64, success: bool) {
        self.total_runs += 1;
        self.total_duration_ms += duration_ms;
        self.last_run_at = Some(chrono::Utc::now().timestamp());
        if success {
            self.successful_runs += 1;
            self.last_success_at = self.last_run_at;
        } else {
            self.failed_runs += 1;
        }
    }

    pub fn record_skip(&mut self) {
        self.skipped_runs += 1;
    }

    pub fn record_timeout(&mut self) {
        self.timeout_count += 1;
        self.failed_runs += 1;
    }

    pub fn avg_duration_ms(&self) -> Option<f64> {
        if self.total_runs == 0 {
            None
        } else {
            Some(self.total_duration_ms as f64 / self.total_runs as f64)
        }
    }

    pub fn success_rate(&self) -> f64 {
        if self.total_runs == 0 {
            1.0
        } else {
            self.successful_runs as f64 / self.total_runs as f64
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PartialJobResult {
    pub job_name: String,
    pub tenant_id: TenantId,
    pub checkpoint_id: String,
    pub processed_count: usize,
    pub total_count: Option<usize>,
    pub last_processed_id: Option<String>,
    pub partial_data: serde_json::Value,
    pub created_at: i64,
}

impl PartialJobResult {
    pub fn new(job_name: String, tenant_id: TenantId) -> Self {
        Self {
            job_name,
            tenant_id,
            checkpoint_id: uuid::Uuid::new_v4().to_string(),
            processed_count: 0,
            total_count: None,
            last_processed_id: None,
            partial_data: serde_json::Value::Null,
            created_at: chrono::Utc::now().timestamp(),
        }
    }

    pub fn with_progress(mut self, processed: usize, total: Option<usize>) -> Self {
        self.processed_count = processed;
        self.total_count = total;
        self
    }

    pub fn with_last_id(mut self, id: String) -> Self {
        self.last_processed_id = Some(id);
        self
    }

    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.partial_data = data;
        self
    }

    pub fn progress_percentage(&self) -> Option<f64> {
        self.total_count
            .map(|total| (self.processed_count as f64 / total as f64) * 100.0)
    }
}

// ---------------------------------------------------------------------------
// Remediation request system (human-in-the-loop approval for day-2 ops)
// ---------------------------------------------------------------------------

/// Risk tier for automated remediation actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemediationRiskTier {
    /// Execute immediately without approval (TTL cleanup, job cleanup).
    AutoExecute,
    /// Execute and notify operator (quota enforcement, importance decay).
    NotifyAndExecute,
    /// Wait for operator approval before executing (reconciliation deletes, tenant purge).
    RequireApproval,
}

/// Current status of a remediation request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemediationStatus {
    Pending,
    Approved,
    Rejected,
    Executed,
    Expired,
    Failed,
}

/// A remediation request created by lifecycle tasks for operator review.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemediationRequest {
    pub id: String,
    pub request_type: String,
    pub risk_tier: RemediationRiskTier,
    pub entity_type: String,
    pub entity_ids: Vec<String>,
    pub tenant_id: Option<String>,
    pub description: String,
    pub proposed_action: String,
    pub detected_by: String,
    pub status: RemediationStatus,
    pub created_at: i64,
    pub reviewed_by: Option<String>,
    pub reviewed_at: Option<i64>,
    pub resolution_notes: Option<String>,
    pub executed_at: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use validator::Validate;

    #[test]
    fn test_knowledge_type_serialization() {
        let adr = KnowledgeType::Adr;
        let json = serde_json::to_string(&adr).unwrap();
        assert_eq!(json, "\"Adr\"");

        let deserialized: KnowledgeType = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, KnowledgeType::Adr);
    }

    #[test]
    fn test_knowledge_layer_serialization() {
        let tenant = KnowledgeLayer::Tenant;
        let json = serde_json::to_string(&tenant).unwrap();
        assert_eq!(json, "\"Tenant\"");

        let deserialized: KnowledgeLayer = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, KnowledgeLayer::Tenant);
    }

    #[test]
    fn test_knowledge_layer_precedence() {
        assert_eq!(KnowledgeLayer::Project.precedence(), 1);
        assert_eq!(KnowledgeLayer::Team.precedence(), 2);
        assert_eq!(KnowledgeLayer::Org.precedence(), 3);
        assert_eq!(KnowledgeLayer::Tenant.precedence(), 4);
    }

    #[test]
    fn test_knowledge_status_serialization_includes_rejected() {
        let rejected = KnowledgeStatus::Rejected;
        let json = serde_json::to_string(&rejected).unwrap();
        assert_eq!(json, "\"Rejected\"");

        let deserialized: KnowledgeStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, KnowledgeStatus::Rejected);
    }

    #[test]
    fn test_promotion_request_validate_layer_direction_accepts_upward_promotion() {
        let request = PromotionRequest {
            id: "prom-1".to_string(),
            source_item_id: "item-1".to_string(),
            source_layer: KnowledgeLayer::Project,
            source_status: KnowledgeStatus::Accepted,
            target_layer: KnowledgeLayer::Team,
            promotion_mode: PromotionMode::Partial,
            shared_content: "shared".to_string(),
            residual_content: Some("local".to_string()),
            residual_role: Some(KnowledgeVariantRole::Specialization),
            justification: Some("reuse".to_string()),
            status: PromotionRequestStatus::PendingReview,
            requested_by: UserId::new("user-1".to_string()).unwrap(),
            tenant_id: TenantId::new("tenant-1".to_string()).unwrap(),
            source_version: "sha-1".to_string(),
            latest_decision: None,
            promoted_item_id: None,
            residual_item_id: None,
            created_at: 1,
            updated_at: 1,
        };

        assert!(request.validate_layer_direction().is_ok());
    }

    #[test]
    fn test_promotion_request_validate_layer_direction_rejects_non_upward_target() {
        let request = PromotionRequest {
            id: "prom-1".to_string(),
            source_item_id: "item-1".to_string(),
            source_layer: KnowledgeLayer::Team,
            source_status: KnowledgeStatus::Accepted,
            target_layer: KnowledgeLayer::Project,
            promotion_mode: PromotionMode::Partial,
            shared_content: "shared".to_string(),
            residual_content: None,
            residual_role: None,
            justification: None,
            status: PromotionRequestStatus::PendingReview,
            requested_by: UserId::new("user-1".to_string()).unwrap(),
            tenant_id: TenantId::new("tenant-1".to_string()).unwrap(),
            source_version: "sha-1".to_string(),
            latest_decision: None,
            promoted_item_id: None,
            residual_item_id: None,
            created_at: 1,
            updated_at: 1,
        };

        assert_eq!(
            request.validate_layer_direction().unwrap_err(),
            "target_layer must be strictly higher than source_layer"
        );
    }

    #[test]
    fn test_promotion_request_validate_layer_direction_rejects_company_source() {
        let request = PromotionRequest {
            id: "prom-1".to_string(),
            source_item_id: "item-1".to_string(),
            source_layer: KnowledgeLayer::Tenant,
            source_status: KnowledgeStatus::Accepted,
            target_layer: KnowledgeLayer::Tenant,
            promotion_mode: PromotionMode::Full,
            shared_content: "shared".to_string(),
            residual_content: None,
            residual_role: None,
            justification: None,
            status: PromotionRequestStatus::PendingReview,
            requested_by: UserId::new("user-1".to_string()).unwrap(),
            tenant_id: TenantId::new("tenant-1".to_string()).unwrap(),
            source_version: "sha-1".to_string(),
            latest_decision: None,
            promoted_item_id: None,
            residual_item_id: None,
            created_at: 1,
            updated_at: 1,
        };

        assert_eq!(
            request.validate_layer_direction().unwrap_err(),
            "tenant-layer knowledge cannot be promoted higher"
        );
    }

    #[test]
    fn test_promotion_request_validate_layer_direction_rejects_non_accepted_source() {
        let request = PromotionRequest {
            id: "prom-1".to_string(),
            source_item_id: "item-1".to_string(),
            source_layer: KnowledgeLayer::Project,
            source_status: KnowledgeStatus::Draft,
            target_layer: KnowledgeLayer::Team,
            promotion_mode: PromotionMode::Partial,
            shared_content: "shared".to_string(),
            residual_content: None,
            residual_role: None,
            justification: None,
            status: PromotionRequestStatus::PendingReview,
            requested_by: UserId::new("user-1".to_string()).unwrap(),
            tenant_id: TenantId::new("tenant-1".to_string()).unwrap(),
            source_version: "sha-1".to_string(),
            latest_decision: None,
            promoted_item_id: None,
            residual_item_id: None,
            created_at: 1,
            updated_at: 1,
        };

        assert_eq!(
            request.validate_layer_direction().unwrap_err(),
            "only accepted knowledge can be promoted"
        );
    }

    #[test]
    fn test_memory_layer_precedence() {
        assert_eq!(MemoryLayer::Agent.precedence(), 1);
        assert_eq!(MemoryLayer::User.precedence(), 2);
        assert_eq!(MemoryLayer::Session.precedence(), 3);
        assert_eq!(MemoryLayer::Project.precedence(), 4);
        assert_eq!(MemoryLayer::Team.precedence(), 5);
        assert_eq!(MemoryLayer::Org.precedence(), 6);
        assert_eq!(MemoryLayer::Tenant.precedence(), 7);
    }

    #[test]
    fn test_memory_layer_serialization() {
        let agent = MemoryLayer::Agent;
        let json = serde_json::to_string(&agent).unwrap();
        assert_eq!(json, "\"Agent\"");

        let deserialized: MemoryLayer = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, MemoryLayer::Agent);
    }

    #[test]
    fn test_constraint_severity_serialization() {
        let block = ConstraintSeverity::Block;
        let json = serde_json::to_string(&block).unwrap();
        assert_eq!(json, "\"Block\"");

        let deserialized: ConstraintSeverity = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ConstraintSeverity::Block);
    }

    #[test]
    fn test_constraint_operator_serialization() {
        let must_use = ConstraintOperator::MustUse;
        let json = serde_json::to_string(&must_use).unwrap();
        assert_eq!(json, "\"MustUse\"");

        let deserialized: ConstraintOperator = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ConstraintOperator::MustUse);
    }

    #[test]
    fn test_constraint_target_serialization() {
        let file = ConstraintTarget::File;
        let json = serde_json::to_string(&file).unwrap();
        assert_eq!(json, "\"File\"");

        let deserialized: ConstraintTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ConstraintTarget::File);
    }

    #[test]
    fn test_memory_entry_creation() {
        let entry = MemoryEntry {
            id: "test_id".to_string(),
            content: "Test content".to_string(),
            embedding: Some(vec![0.1, 0.2, 0.3]),
            layer: MemoryLayer::User,
            summaries: std::collections::HashMap::new(),
            context_vector: None,
            importance_score: None,
            metadata: std::collections::HashMap::new(),
            created_at: 1234567890,
            updated_at: 1234567890,
        };

        assert_eq!(entry.id, "test_id");
        assert_eq!(entry.content, "Test content");
        assert_eq!(entry.layer, MemoryLayer::User);
        assert_eq!(entry.embedding.unwrap().len(), 3);
    }

    #[test]
    fn test_knowledge_entry_creation() {
        let entry = KnowledgeEntry {
            path: "docs/adr/001.md".to_string(),
            content: "# ADR 001: Use Rust".to_string(),
            layer: KnowledgeLayer::Project,
            kind: KnowledgeType::Adr,
            summaries: std::collections::HashMap::new(),
            metadata: std::collections::HashMap::new(),
            commit_hash: Some("abc123".to_string()),
            author: Some("Alice".to_string()),
            status: KnowledgeStatus::Accepted,
            updated_at: 1234567890,
        };

        assert_eq!(entry.path, "docs/adr/001.md");
        assert_eq!(entry.layer, KnowledgeLayer::Project);
        assert_eq!(entry.kind, KnowledgeType::Adr);
        assert_eq!(entry.commit_hash.unwrap(), "abc123");
    }

    #[test]
    fn test_policy_creation() {
        let rule = PolicyRule {
            id: "rule_1".to_string(),
            rule_type: RuleType::Deny,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustNotUse,
            value: serde_json::json!("unsafe-lib"),
            severity: ConstraintSeverity::Block,
            message: "Do not use unsafe libraries".to_string(),
        };

        let policy = Policy {
            id: "policy_1".to_string(),
            name: "Security Policy".to_string(),
            description: Some("Security constraints".to_string()),
            layer: KnowledgeLayer::Tenant,
            mode: PolicyMode::Mandatory,
            merge_strategy: RuleMergeStrategy::Merge,
            rules: vec![rule],
            metadata: std::collections::HashMap::new(),
        };

        assert_eq!(policy.id, "policy_1");
        assert_eq!(policy.layer, KnowledgeLayer::Tenant);
        assert_eq!(policy.rules.len(), 1);
        assert_eq!(policy.rules[0].target, ConstraintTarget::Dependency);
    }

    #[test]
    fn test_validation_result_creation() {
        let violation = PolicyViolation {
            rule_id: "rule_1".to_string(),
            policy_id: "policy_1".to_string(),
            severity: ConstraintSeverity::Warn,
            message: "Warning message".to_string(),
            context: std::collections::HashMap::new(),
        };

        let result = ValidationResult {
            is_valid: false,
            violations: vec![violation],
        };

        assert!(!result.is_valid);
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].severity, ConstraintSeverity::Warn);
    }

    #[test]
    fn test_validate_user_id_valid() {
        let user_id = "user_123".to_string();
        let result = validate_user_id(&&user_id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_user_id_empty() {
        let user_id = "".to_string();
        let result = validate_user_id(&&user_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_user_id_too_long() {
        let user_id = "a".repeat(101);
        let result = validate_user_id(&&user_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_session_id_valid() {
        let session_id = "session_456".to_string();
        let result = validate_session_id(&&session_id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_project_id_valid() {
        let project_id = "project_789".to_string();
        let result = validate_project_id(&&project_id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_team_id_valid() {
        let team_id = "team_abc".to_string();
        let result = validate_team_id(&&team_id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_org_id_valid() {
        let org_id = "org_xyz".to_string();
        let result = validate_org_id(&&org_id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_tenant_id_valid() {
        let tenant_id = "tenant_123".to_string();
        let result = validate_tenant_id(&&tenant_id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_layer_identifiers_validation() {
        let identifiers = LayerIdentifiers {
            agent_id: Some("agent_1".to_string()),
            user_id: Some("user_123".to_string()),
            session_id: Some("session_456".to_string()),
            project_id: Some("project_789".to_string()),
            team_id: Some("team_abc".to_string()),
            org_id: Some("org_xyz".to_string()),
            tenant_id: Some("tenant_123".to_string()),
        };

        let result = identifiers.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_layer_identifiers_invalid_user_id() {
        let identifiers = LayerIdentifiers {
            agent_id: Some("agent_1".to_string()),
            user_id: Some("".to_string()),
            session_id: None,
            project_id: None,
            team_id: None,
            org_id: None,
            tenant_id: None,
        };

        let result = identifiers.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_memory_layer_display_name() {
        assert_eq!(MemoryLayer::Agent.display_name(), "Agent");
        assert_eq!(MemoryLayer::User.display_name(), "User");
        assert_eq!(MemoryLayer::Session.display_name(), "Session");
        assert_eq!(MemoryLayer::Project.display_name(), "Project");
        assert_eq!(MemoryLayer::Team.display_name(), "Team");
        assert_eq!(MemoryLayer::Org.display_name(), "Organization");
        assert_eq!(MemoryLayer::Tenant.display_name(), "Tenant");
    }

    #[test]
    fn test_validate_agent_id_valid() {
        let agent_id = "agent_123".to_string();
        let result = validate_agent_id(&&agent_id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_agent_id_empty() {
        let agent_id = "".to_string();
        let result = validate_agent_id(&&agent_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_agent_id_too_long() {
        let agent_id = "a".repeat(101);
        let result = validate_agent_id(&&agent_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_session_id_empty() {
        let id = "".to_string();
        assert!(validate_session_id(&&id).is_err());
    }

    #[test]
    fn test_validate_project_id_empty() {
        let id = "".to_string();
        assert!(validate_project_id(&&id).is_err());
    }

    #[test]
    fn test_validate_team_id_empty() {
        let id = "".to_string();
        assert!(validate_team_id(&&id).is_err());
    }

    #[test]
    fn test_validate_org_id_empty() {
        let id = "".to_string();
        assert!(validate_org_id(&&id).is_err());
    }

    #[test]
    fn test_validate_tenant_id_empty() {
        let id = "".to_string();
        assert!(validate_tenant_id(&&id).is_err());
    }

    #[test]
    fn test_memory_layer_from_str() {
        use std::str::FromStr;
        assert_eq!(MemoryLayer::from_str("Agent").unwrap(), MemoryLayer::Agent);
        assert_eq!(
            MemoryLayer::from_str("Session").unwrap(),
            MemoryLayer::Session
        );
        assert!(MemoryLayer::from_str("Invalid").is_err());
    }

    #[test]
    fn test_memory_layer_display() {
        assert_eq!(format!("{}", MemoryLayer::Agent), "Agent");
        assert_eq!(format!("{}", MemoryLayer::User), "User");
        assert_eq!(format!("{}", MemoryLayer::Session), "Session");
        assert_eq!(format!("{}", MemoryLayer::Project), "Project");
        assert_eq!(format!("{}", MemoryLayer::Team), "Team");
        assert_eq!(format!("{}", MemoryLayer::Org), "Org");
        assert_eq!(format!("{}", MemoryLayer::Tenant), "Tenant");
    }

    #[test]
    fn test_role_serialization() {
        let architect = Role::Architect;
        let json = serde_json::to_string(&architect).unwrap();
        assert_eq!(json, "\"Architect\"");

        let deserialized: Role = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, Role::Architect);
    }

    #[test]
    fn test_role_precedence() {
        assert_eq!(Role::Viewer.precedence(), 0);
        assert_eq!(Role::Agent.precedence(), 0);
        assert_eq!(Role::Developer.precedence(), 1);
        assert_eq!(Role::TechLead.precedence(), 2);
        assert_eq!(Role::Architect.precedence(), 3);
        assert_eq!(Role::Admin.precedence(), 4);
        assert_eq!(Role::PlatformAdmin.precedence(), 5);
        assert_eq!(Role::TenantAdmin.precedence(), 6);
    }

    #[test]
    fn test_reasoning_strategy_serialization() {
        let exhaustive = ReasoningStrategy::Exhaustive;
        let json = serde_json::to_string(&exhaustive).unwrap();
        assert_eq!(json, "\"Exhaustive\"");

        let deserialized: ReasoningStrategy = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ReasoningStrategy::Exhaustive);
    }

    #[test]
    fn test_reasoning_strategy_display() {
        assert_eq!(format!("{}", ReasoningStrategy::Exhaustive), "exhaustive");
        assert_eq!(format!("{}", ReasoningStrategy::Targeted), "targeted");
    }

    #[test]
    fn test_tenant_id_validation() {
        assert!(TenantId::new("comp_123".to_string()).is_some());
        assert!(TenantId::new("".to_string()).is_none());
        assert!(TenantId::new("a".repeat(101)).is_none());
    }

    #[test]
    fn test_user_id_validation() {
        assert!(UserId::new("user_456".to_string()).is_some());
        assert!(UserId::new("".to_string()).is_none());
        assert!(UserId::new("a".repeat(101)).is_none());
    }

    #[test]
    fn test_hierarchy_path_depth() {
        let tenant = HierarchyPath::tenant("t1".to_string());
        assert_eq!(tenant.depth(), 1);

        let org = HierarchyPath::org("t1".to_string(), "o1".to_string());
        assert_eq!(org.depth(), 2);

        let team = HierarchyPath::team("t1".to_string(), "o1".to_string(), "t1".to_string());
        assert_eq!(team.depth(), 3);

        let project = HierarchyPath::project(
            "t1".to_string(),
            "o1".to_string(),
            "t1".to_string(),
            "p1".to_string(),
        );
        assert_eq!(project.depth(), 4);
    }

    #[test]
    fn test_hierarchy_path_string() {
        let project = HierarchyPath::project(
            "t1".to_string(),
            "o1".to_string(),
            "t1".to_string(),
            "p1".to_string(),
        );
        assert_eq!(project.path_string(), "t1 > o1 > t1 > p1");
    }

    #[test]
    fn test_tenant_context_creation() {
        let tenant_id = TenantId::new("c1".to_string()).unwrap();
        let user_id = UserId::new("u1".to_string()).unwrap();
        let ctx = TenantContext::new(tenant_id, user_id);

        assert_eq!(ctx.tenant_id.as_str(), "c1");
        assert_eq!(ctx.user_id.as_str(), "u1");
        assert!(ctx.agent_id.is_none());
    }

    #[test]
    fn test_tenant_context_with_agent() {
        let tenant_id = TenantId::new("c1".to_string()).unwrap();
        let user_id = UserId::new("u1".to_string()).unwrap();
        let ctx = TenantContext::with_agent(tenant_id, user_id, "a1".to_string());

        assert_eq!(ctx.agent_id.unwrap(), "a1");
    }

    #[test]
    fn test_tenant_id_display() {
        let id = TenantId::new("c1".to_string()).unwrap();
        assert_eq!(format!("{}", id), "c1");
    }

    #[test]
    fn test_user_id_display() {
        let id = UserId::new("u1".to_string()).unwrap();
        assert_eq!(format!("{}", id), "u1");
    }

    #[test]
    fn test_tenant_id_from_str() {
        use std::str::FromStr;
        let id = TenantId::from_str("c1").unwrap();
        assert_eq!(id.as_str(), "c1");
        assert!(TenantId::from_str("").is_err());
    }

    #[test]
    fn test_user_id_from_str() {
        use std::str::FromStr;
        let id = UserId::from_str("u1").unwrap();
        assert_eq!(id.as_str(), "u1");
        assert!(UserId::from_str("").is_err());
    }

    #[test]
    fn test_tenant_id_into_inner() {
        let id = TenantId::new("c1".to_string()).unwrap();
        assert_eq!(id.into_inner(), "c1");
    }

    #[test]
    fn test_user_id_into_inner() {
        let id = UserId::new("u1".to_string()).unwrap();
        assert_eq!(id.into_inner(), "u1");
    }

    #[test]
    fn test_governance_event_tenant_id() {
        let tenant_id = TenantId::new("tenant-1".to_string()).unwrap();
        let user_id = UserId::new("user-1".to_string()).unwrap();

        let events = vec![
            GovernanceEvent::UnitCreated {
                unit_id: "u1".to_string(),
                unit_type: UnitType::Organization,
                tenant_id: tenant_id.clone(),
                parent_id: None,
                timestamp: 0,
            },
            GovernanceEvent::UnitUpdated {
                unit_id: "u1".to_string(),
                tenant_id: tenant_id.clone(),
                timestamp: 0,
            },
            GovernanceEvent::UnitDeleted {
                unit_id: "u1".to_string(),
                tenant_id: tenant_id.clone(),
                timestamp: 0,
            },
            GovernanceEvent::RoleAssigned {
                user_id: user_id.clone(),
                unit_id: "u1".to_string(),
                role: Role::Admin.into(),
                tenant_id: tenant_id.clone(),
                timestamp: 0,
            },
            GovernanceEvent::RoleRemoved {
                user_id: user_id.clone(),
                unit_id: "u1".to_string(),
                role: Role::Admin.into(),
                tenant_id: tenant_id.clone(),
                timestamp: 0,
            },
            GovernanceEvent::PolicyUpdated {
                policy_id: "p1".to_string(),
                layer: KnowledgeLayer::Tenant,
                tenant_id: tenant_id.clone(),
                timestamp: 0,
            },
            GovernanceEvent::PolicyDeleted {
                policy_id: "p1".to_string(),
                tenant_id: tenant_id.clone(),
                timestamp: 0,
            },
            GovernanceEvent::DriftDetected {
                project_id: "proj-1".to_string(),
                tenant_id: tenant_id.clone(),
                drift_score: 0.5,
                timestamp: 0,
            },
        ];

        for event in events {
            assert_eq!(event.tenant_id().as_str(), "tenant-1");
        }
    }

    #[test]
    fn test_role_display_name() {
        assert_eq!(Role::Viewer.display_name(), "Viewer");
        assert_eq!(Role::Developer.display_name(), "Developer");
        assert_eq!(Role::TechLead.display_name(), "Tech Lead");
        assert_eq!(Role::Architect.display_name(), "Architect");
        assert_eq!(Role::Admin.display_name(), "Admin");
        assert_eq!(Role::TenantAdmin.display_name(), "Tenant Admin");
        assert_eq!(Role::Agent.display_name(), "Agent");
        assert_eq!(Role::PlatformAdmin.display_name(), "Platform Admin");
    }

    #[test]
    fn test_drift_suppression_new() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let user_id = UserId::new("user1".to_string()).unwrap();

        let suppression = DriftSuppression::new(
            "proj-1".to_string(),
            tenant_id.clone(),
            "policy-1".to_string(),
            "False positive".to_string(),
            user_id.clone(),
        );

        assert_eq!(suppression.project_id, "proj-1");
        assert_eq!(suppression.tenant_id, tenant_id);
        assert_eq!(suppression.policy_id, "policy-1");
        assert_eq!(suppression.reason, "False positive");
        assert!(suppression.rule_pattern.is_none());
        assert!(suppression.expires_at.is_none());
        assert!(!suppression.id.is_empty());
    }

    #[test]
    fn test_drift_suppression_with_pattern() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let user_id = UserId::new("user1".to_string()).unwrap();

        let suppression = DriftSuppression::new(
            "proj-1".to_string(),
            tenant_id,
            "policy-1".to_string(),
            "Known issue".to_string(),
            user_id,
        )
        .with_pattern(".*test.*".to_string());

        assert_eq!(suppression.rule_pattern, Some(".*test.*".to_string()));
    }

    #[test]
    fn test_drift_suppression_with_expiry() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let user_id = UserId::new("user1".to_string()).unwrap();
        let future_time = chrono::Utc::now().timestamp() + 86400;

        let suppression = DriftSuppression::new(
            "proj-1".to_string(),
            tenant_id,
            "policy-1".to_string(),
            "Temporary".to_string(),
            user_id,
        )
        .with_expiry(future_time);

        assert_eq!(suppression.expires_at, Some(future_time));
    }

    #[test]
    fn test_drift_suppression_is_expired() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let user_id = UserId::new("user1".to_string()).unwrap();

        let not_expired = DriftSuppression::new(
            "proj-1".to_string(),
            tenant_id.clone(),
            "policy-1".to_string(),
            "Not expired".to_string(),
            user_id.clone(),
        );
        assert!(!not_expired.is_expired());

        let future_expiry = DriftSuppression::new(
            "proj-1".to_string(),
            tenant_id.clone(),
            "policy-1".to_string(),
            "Future".to_string(),
            user_id.clone(),
        )
        .with_expiry(chrono::Utc::now().timestamp() + 86400);
        assert!(!future_expiry.is_expired());

        let past_expiry = DriftSuppression::new(
            "proj-1".to_string(),
            tenant_id,
            "policy-1".to_string(),
            "Expired".to_string(),
            user_id,
        )
        .with_expiry(chrono::Utc::now().timestamp() - 86400);
        assert!(past_expiry.is_expired());
    }

    #[test]
    fn test_drift_suppression_matches() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let user_id = UserId::new("user1".to_string()).unwrap();

        let violation = PolicyViolation {
            rule_id: "rule-1".to_string(),
            policy_id: "policy-1".to_string(),
            severity: ConstraintSeverity::Warn,
            message: "Test violation message".to_string(),
            context: std::collections::HashMap::new(),
        };

        let suppression_match = DriftSuppression::new(
            "proj-1".to_string(),
            tenant_id.clone(),
            "policy-1".to_string(),
            "Match all".to_string(),
            user_id.clone(),
        );
        assert!(suppression_match.matches(&violation));

        let suppression_no_match = DriftSuppression::new(
            "proj-1".to_string(),
            tenant_id.clone(),
            "policy-2".to_string(),
            "Different policy".to_string(),
            user_id.clone(),
        );
        assert!(!suppression_no_match.matches(&violation));

        let suppression_pattern_match = DriftSuppression::new(
            "proj-1".to_string(),
            tenant_id.clone(),
            "policy-1".to_string(),
            "Pattern match".to_string(),
            user_id.clone(),
        )
        .with_pattern(".*violation.*".to_string());
        assert!(suppression_pattern_match.matches(&violation));

        let suppression_pattern_no_match = DriftSuppression::new(
            "proj-1".to_string(),
            tenant_id,
            "policy-1".to_string(),
            "Pattern no match".to_string(),
            user_id,
        )
        .with_pattern(".*xyz.*".to_string());
        assert!(!suppression_pattern_no_match.matches(&violation));
    }

    #[test]
    fn test_drift_config_default() {
        let config = DriftConfig::default();
        assert!(config.project_id.is_empty());
        assert_eq!(config.threshold, 0.2);
        assert_eq!(config.low_confidence_threshold, 0.7);
        assert!(!config.auto_suppress_info);
    }

    #[test]
    fn test_drift_config_new() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let config = DriftConfig::new("proj-1".to_string(), tenant_id.clone());

        assert_eq!(config.project_id, "proj-1");
        assert_eq!(config.tenant_id, tenant_id);
        assert_eq!(config.threshold, 0.2);
    }

    #[test]
    fn test_drift_config_for_project() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let config = DriftConfig::for_project("proj-2".to_string(), tenant_id.clone());

        assert_eq!(config.project_id, "proj-2");
        assert_eq!(config.tenant_id, tenant_id);
    }

    #[test]
    fn test_drift_config_with_threshold() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let config = DriftConfig::new("proj-1".to_string(), tenant_id).with_threshold(0.5);

        assert_eq!(config.threshold, 0.5);
    }

    #[test]
    fn test_drift_config_with_threshold_clamped() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();

        let config_low =
            DriftConfig::new("proj-1".to_string(), tenant_id.clone()).with_threshold(-0.5);
        assert_eq!(config_low.threshold, 0.0);

        let config_high = DriftConfig::new("proj-1".to_string(), tenant_id).with_threshold(1.5);
        assert_eq!(config_high.threshold, 1.0);
    }

    #[test]
    fn test_drift_result_active_violation_count() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let violations = vec![
            PolicyViolation {
                rule_id: "r1".to_string(),
                policy_id: "p1".to_string(),
                severity: ConstraintSeverity::Warn,
                message: "Warning".to_string(),
                context: std::collections::HashMap::new(),
            },
            PolicyViolation {
                rule_id: "r2".to_string(),
                policy_id: "p1".to_string(),
                severity: ConstraintSeverity::Block,
                message: "Blocking".to_string(),
                context: std::collections::HashMap::new(),
            },
        ];

        let result = DriftResult::new("proj-1".to_string(), tenant_id, violations);
        assert_eq!(result.active_violation_count(), 2);
    }

    #[test]
    fn test_drift_result_suppressed_count() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let mut result = DriftResult::new("proj-1".to_string(), tenant_id, vec![]);
        result.suppressed_violations = vec![PolicyViolation {
            rule_id: "r1".to_string(),
            policy_id: "p1".to_string(),
            severity: ConstraintSeverity::Info,
            message: "Suppressed".to_string(),
            context: std::collections::HashMap::new(),
        }];

        assert_eq!(result.suppressed_count(), 1);
    }

    #[test]
    fn test_job_coordination_metrics_new() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let metrics = JobCoordinationMetrics::new("drift_scan".to_string(), tenant_id.clone());

        assert_eq!(metrics.job_name, "drift_scan");
        assert_eq!(metrics.tenant_id, tenant_id);
        assert_eq!(metrics.total_runs, 0);
        assert_eq!(metrics.successful_runs, 0);
        assert_eq!(metrics.failed_runs, 0);
        assert_eq!(metrics.skipped_runs, 0);
        assert_eq!(metrics.timeout_count, 0);
        assert_eq!(metrics.total_duration_ms, 0);
        assert!(metrics.last_run_at.is_none());
        assert!(metrics.last_success_at.is_none());
    }

    #[test]
    fn test_job_coordination_metrics_record_run_success() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let mut metrics = JobCoordinationMetrics::new("test_job".to_string(), tenant_id);
        metrics.record_run(1000, true);

        assert_eq!(metrics.total_runs, 1);
        assert_eq!(metrics.successful_runs, 1);
        assert_eq!(metrics.failed_runs, 0);
        assert_eq!(metrics.total_duration_ms, 1000);
        assert!(metrics.last_run_at.is_some());
        assert!(metrics.last_success_at.is_some());
    }

    #[test]
    fn test_job_coordination_metrics_record_run_failure() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let mut metrics = JobCoordinationMetrics::new("test_job".to_string(), tenant_id);
        metrics.record_run(500, false);

        assert_eq!(metrics.total_runs, 1);
        assert_eq!(metrics.successful_runs, 0);
        assert_eq!(metrics.failed_runs, 1);
        assert_eq!(metrics.total_duration_ms, 500);
        assert!(metrics.last_run_at.is_some());
        assert!(metrics.last_success_at.is_none());
    }

    #[test]
    fn test_job_coordination_metrics_record_skip() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let mut metrics = JobCoordinationMetrics::new("test_job".to_string(), tenant_id);
        metrics.record_skip();

        assert_eq!(metrics.skipped_runs, 1);
        assert_eq!(metrics.total_runs, 0);
    }

    #[test]
    fn test_job_coordination_metrics_record_timeout() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let mut metrics = JobCoordinationMetrics::new("test_job".to_string(), tenant_id);
        metrics.record_timeout();

        assert_eq!(metrics.timeout_count, 1);
        assert_eq!(metrics.failed_runs, 1);
    }

    #[test]
    fn test_job_coordination_metrics_avg_duration_ms() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let mut metrics = JobCoordinationMetrics::new("test_job".to_string(), tenant_id);
        assert!(metrics.avg_duration_ms().is_none());

        metrics.record_run(1000, true);
        metrics.record_run(2000, true);
        assert_eq!(metrics.avg_duration_ms(), Some(1500.0));
    }

    #[test]
    fn test_job_coordination_metrics_success_rate() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let mut metrics = JobCoordinationMetrics::new("test_job".to_string(), tenant_id);
        assert_eq!(metrics.success_rate(), 1.0);

        metrics.record_run(100, true);
        metrics.record_run(100, true);
        metrics.record_run(100, false);
        let rate = metrics.success_rate();
        assert!((rate - 0.6666666666666666).abs() < 0.0001);
    }

    #[test]
    fn test_partial_job_result_new() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let result = PartialJobResult::new("drift_scan".to_string(), tenant_id.clone());

        assert_eq!(result.job_name, "drift_scan");
        assert_eq!(result.tenant_id, tenant_id);
        assert_eq!(result.processed_count, 0);
        assert!(result.total_count.is_none());
        assert!(result.last_processed_id.is_none());
        assert_eq!(result.partial_data, serde_json::Value::Null);
        assert!(!result.checkpoint_id.is_empty());
    }

    #[test]
    fn test_partial_job_result_with_progress() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let result =
            PartialJobResult::new("drift_scan".to_string(), tenant_id).with_progress(50, Some(100));

        assert_eq!(result.processed_count, 50);
        assert_eq!(result.total_count, Some(100));
    }

    #[test]
    fn test_partial_job_result_with_last_id() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let result = PartialJobResult::new("drift_scan".to_string(), tenant_id)
            .with_last_id("item-50".to_string());

        assert_eq!(result.last_processed_id, Some("item-50".to_string()));
    }

    #[test]
    fn test_partial_job_result_with_data() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let data = serde_json::json!({"key": "value"});
        let result =
            PartialJobResult::new("drift_scan".to_string(), tenant_id).with_data(data.clone());

        assert_eq!(result.partial_data, data);
    }

    #[test]
    fn test_partial_job_result_progress_percentage() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();

        let no_total = PartialJobResult::new("drift_scan".to_string(), tenant_id.clone());
        assert!(no_total.progress_percentage().is_none());

        let with_total =
            PartialJobResult::new("drift_scan".to_string(), tenant_id).with_progress(25, Some(100));
        assert_eq!(with_total.progress_percentage(), Some(25.0));
    }

    #[test]
    fn test_persistent_event_mark_published() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let event = GovernanceEvent::UnitCreated {
            unit_id: "u1".to_string(),
            unit_type: UnitType::Organization,
            tenant_id: tenant_id.clone(),
            parent_id: None,
            timestamp: 0,
        };

        let mut persistent = PersistentEvent::new(event);
        assert_eq!(persistent.status, EventStatus::Pending);
        assert!(persistent.published_at.is_none());

        persistent.mark_published();
        assert_eq!(persistent.status, EventStatus::Published);
        assert!(persistent.published_at.is_some());
    }

    #[test]
    fn test_persistent_event_mark_acknowledged() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let event = GovernanceEvent::UnitCreated {
            unit_id: "u1".to_string(),
            unit_type: UnitType::Organization,
            tenant_id: tenant_id.clone(),
            parent_id: None,
            timestamp: 0,
        };

        let mut persistent = PersistentEvent::new(event);
        persistent.mark_acknowledged();

        assert_eq!(persistent.status, EventStatus::Acknowledged);
        assert!(persistent.acknowledged_at.is_some());
    }

    #[test]
    fn test_persistent_event_mark_failed_retriable() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let event = GovernanceEvent::UnitCreated {
            unit_id: "u1".to_string(),
            unit_type: UnitType::Organization,
            tenant_id: tenant_id.clone(),
            parent_id: None,
            timestamp: 0,
        };

        let mut persistent = PersistentEvent::new(event);
        let can_retry = persistent.mark_failed("Connection timeout".to_string());

        assert!(can_retry);
        assert_eq!(persistent.retry_count, 1);
        assert_eq!(
            persistent.last_error,
            Some("Connection timeout".to_string())
        );
        assert_eq!(persistent.status, EventStatus::Pending);
    }

    #[test]
    fn test_persistent_event_mark_failed_dead_lettered() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let event = GovernanceEvent::UnitCreated {
            unit_id: "u1".to_string(),
            unit_type: UnitType::Organization,
            tenant_id: tenant_id.clone(),
            parent_id: None,
            timestamp: 0,
        };

        let mut persistent = PersistentEvent::new(event);
        persistent.mark_failed("Error 1".to_string());
        persistent.mark_failed("Error 2".to_string());
        let can_retry = persistent.mark_failed("Error 3".to_string());

        assert!(!can_retry);
        assert_eq!(persistent.status, EventStatus::DeadLettered);
        assert!(persistent.dead_lettered_at.is_some());
    }

    #[test]
    fn test_persistent_event_is_retriable() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let event = GovernanceEvent::UnitCreated {
            unit_id: "u1".to_string(),
            unit_type: UnitType::Organization,
            tenant_id: tenant_id.clone(),
            parent_id: None,
            timestamp: 0,
        };

        let mut persistent = PersistentEvent::new(event);
        assert!(persistent.is_retriable());

        persistent.mark_failed("Error 1".to_string());
        assert!(persistent.is_retriable());

        persistent.mark_failed("Error 2".to_string());
        assert!(persistent.is_retriable());

        persistent.mark_failed("Error 3".to_string());
        assert!(!persistent.is_retriable());
    }

    #[test]
    fn test_event_delivery_metrics_new() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let metrics =
            EventDeliveryMetrics::new(tenant_id.clone(), "drift_detected".to_string(), 1000, 2000);

        assert_eq!(metrics.tenant_id, tenant_id);
        assert_eq!(metrics.event_type, "drift_detected");
        assert_eq!(metrics.period_start, 1000);
        assert_eq!(metrics.period_end, 2000);
        assert_eq!(metrics.total_events, 0);
        assert_eq!(metrics.delivered_events, 0);
    }

    #[test]
    fn test_event_delivery_metrics_success_rate() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let mut metrics =
            EventDeliveryMetrics::new(tenant_id, "drift_detected".to_string(), 1000, 2000);

        assert_eq!(metrics.delivery_success_rate(), 1.0);

        metrics.total_events = 10;
        metrics.delivered_events = 8;
        assert_eq!(metrics.delivery_success_rate(), 0.8);
    }

    #[test]
    fn test_consumer_state_new() {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let state = ConsumerState::new(
            "drift_processor".to_string(),
            "idempotency-key-123".to_string(),
            tenant_id.clone(),
        );

        assert_eq!(state.consumer_group, "drift_processor");
        assert_eq!(state.idempotency_key, "idempotency-key-123");
        assert_eq!(state.tenant_id, tenant_id);
        assert!(state.processed_at > 0);
    }

    // ==========================================================================
    // CCA Types Tests (Phase 1.1.5)
    // ==========================================================================

    #[test]
    fn test_summary_depth_serialization() {
        let sentence = SummaryDepth::Sentence;
        let json = serde_json::to_string(&sentence).unwrap();
        assert_eq!(json, "\"Sentence\"");

        let paragraph = SummaryDepth::Paragraph;
        let json = serde_json::to_string(&paragraph).unwrap();
        assert_eq!(json, "\"Paragraph\"");

        let detailed = SummaryDepth::Detailed;
        let json = serde_json::to_string(&detailed).unwrap();
        assert_eq!(json, "\"Detailed\"");
    }

    #[test]
    fn test_summary_depth_deserialization() {
        let sentence: SummaryDepth = serde_json::from_str("\"Sentence\"").unwrap();
        assert_eq!(sentence, SummaryDepth::Sentence);

        let paragraph: SummaryDepth = serde_json::from_str("\"Paragraph\"").unwrap();
        assert_eq!(paragraph, SummaryDepth::Paragraph);

        let detailed: SummaryDepth = serde_json::from_str("\"Detailed\"").unwrap();
        assert_eq!(detailed, SummaryDepth::Detailed);
    }

    #[test]
    fn test_summary_depth_hash_key() {
        let mut map = std::collections::HashMap::new();
        map.insert(SummaryDepth::Sentence, "short");
        map.insert(SummaryDepth::Paragraph, "medium");
        map.insert(SummaryDepth::Detailed, "long");

        assert_eq!(map.get(&SummaryDepth::Sentence), Some(&"short"));
        assert_eq!(map.get(&SummaryDepth::Paragraph), Some(&"medium"));
        assert_eq!(map.get(&SummaryDepth::Detailed), Some(&"long"));
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn test_layer_summary_creation() {
        let summary = LayerSummary {
            depth: SummaryDepth::Sentence,
            content: "This is a one-sentence summary.".to_string(),
            token_count: 8,
            generated_at: 1705500000,
            source_hash: "abc123def456".to_string(),
            content_hash: None,
            personalized: false,
            personalization_context: None,
        };

        assert_eq!(summary.depth, SummaryDepth::Sentence);
        assert_eq!(summary.content, "This is a one-sentence summary.");
        assert_eq!(summary.token_count, 8);
        assert_eq!(summary.source_hash, "abc123def456");
        assert!(!summary.personalized);
        assert!(summary.personalization_context.is_none());
    }

    #[test]
    fn test_layer_summary_with_personalization() {
        let summary = LayerSummary {
            depth: SummaryDepth::Detailed,
            content: "Detailed summary for backend developers...".to_string(),
            token_count: 150,
            generated_at: 1705500000,
            source_hash: "hash789".to_string(),
            content_hash: None,
            personalized: true,
            personalization_context: Some("backend developer, Rust experience".to_string()),
        };

        assert!(summary.personalized);
        assert_eq!(
            summary.personalization_context,
            Some("backend developer, Rust experience".to_string())
        );
    }

    #[test]
    fn test_layer_summary_serialization_roundtrip() {
        let summary = LayerSummary {
            depth: SummaryDepth::Paragraph,
            content: "A paragraph-length summary explaining the concept.".to_string(),
            token_count: 42,
            generated_at: 1705500000,
            source_hash: "source_hash_value".to_string(),
            content_hash: None,
            personalized: true,
            personalization_context: Some("security focus".to_string()),
        };

        let json = serde_json::to_string(&summary).unwrap();
        let deserialized: LayerSummary = serde_json::from_str(&json).unwrap();

        assert_eq!(summary, deserialized);
    }

    #[test]
    fn test_layer_summary_json_structure() {
        let summary = LayerSummary {
            depth: SummaryDepth::Sentence,
            content: "Summary".to_string(),
            token_count: 1,
            generated_at: 1000,
            source_hash: "hash".to_string(),
            content_hash: None,
            personalized: false,
            personalization_context: None,
        };

        let json: serde_json::Value = serde_json::to_value(&summary).unwrap();

        assert!(json.get("depth").is_some());
        assert!(json.get("content").is_some());
        assert!(json.get("tokenCount").is_some());
        assert!(json.get("generatedAt").is_some());
        assert!(json.get("sourceHash").is_some());
        assert!(json.get("personalized").is_some());
        assert!(json.get("personalizationContext").is_some());
    }

    #[test]
    fn test_summary_config_creation() {
        let config = SummaryConfig {
            layer: MemoryLayer::Project,
            update_interval_secs: Some(3600),
            update_on_changes: Some(10),
            skip_if_unchanged: true,
            personalized: false,
            depths: vec![SummaryDepth::Sentence, SummaryDepth::Paragraph],
        };

        assert_eq!(config.layer, MemoryLayer::Project);
        assert_eq!(config.update_interval_secs, Some(3600));
        assert_eq!(config.update_on_changes, Some(10));
        assert!(config.skip_if_unchanged);
        assert!(!config.personalized);
        assert_eq!(config.depths.len(), 2);
    }

    #[test]
    fn test_summary_config_time_based_trigger() {
        let config = SummaryConfig {
            layer: MemoryLayer::Session,
            update_interval_secs: Some(300),
            update_on_changes: None,
            skip_if_unchanged: true,
            personalized: false,
            depths: vec![SummaryDepth::Sentence],
        };

        assert!(config.update_interval_secs.is_some());
        assert!(config.update_on_changes.is_none());
    }

    #[test]
    fn test_summary_config_change_based_trigger() {
        let config = SummaryConfig {
            layer: MemoryLayer::Team,
            update_interval_secs: None,
            update_on_changes: Some(5),
            skip_if_unchanged: false,
            personalized: true,
            depths: vec![SummaryDepth::Detailed],
        };

        assert!(config.update_interval_secs.is_none());
        assert_eq!(config.update_on_changes, Some(5));
    }

    #[test]
    fn test_summary_config_serialization_roundtrip() {
        let config = SummaryConfig {
            layer: MemoryLayer::Tenant,
            update_interval_secs: Some(86400),
            update_on_changes: Some(100),
            skip_if_unchanged: true,
            personalized: true,
            depths: vec![
                SummaryDepth::Sentence,
                SummaryDepth::Paragraph,
                SummaryDepth::Detailed,
            ],
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: SummaryConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config, deserialized);
    }

    #[test]
    fn test_summary_config_json_structure() {
        let config = SummaryConfig {
            layer: MemoryLayer::User,
            update_interval_secs: Some(600),
            update_on_changes: None,
            skip_if_unchanged: false,
            personalized: true,
            depths: vec![SummaryDepth::Sentence],
        };

        let json: serde_json::Value = serde_json::to_value(&config).unwrap();

        assert!(json.get("layer").is_some());
        assert!(json.get("updateIntervalSecs").is_some());
        assert!(json.get("updateOnChanges").is_some());
        assert!(json.get("skipIfUnchanged").is_some());
        assert!(json.get("personalized").is_some());
        assert!(json.get("depths").is_some());
    }

    #[test]
    fn test_context_vector_type() {
        let vector: ContextVector = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        assert_eq!(vector.len(), 5);
        assert_eq!(vector[0], 0.1);
    }

    #[test]
    fn test_memory_entry_with_summaries() {
        let mut summaries = std::collections::HashMap::new();
        summaries.insert(
            SummaryDepth::Sentence,
            LayerSummary {
                depth: SummaryDepth::Sentence,
                content: "Short summary.".to_string(),
                token_count: 3,
                generated_at: 1705500000,
                source_hash: "hash1".to_string(),
                content_hash: None,
                personalized: false,
                personalization_context: None,
            },
        );
        summaries.insert(
            SummaryDepth::Paragraph,
            LayerSummary {
                depth: SummaryDepth::Paragraph,
                content: "This is a longer paragraph summary with more details.".to_string(),
                token_count: 25,
                generated_at: 1705500000,
                source_hash: "hash1".to_string(),
                content_hash: None,
                personalized: false,
                personalization_context: None,
            },
        );

        let entry = MemoryEntry {
            id: "mem_001".to_string(),
            content: "Original content that was summarized.".to_string(),
            embedding: Some(vec![0.1, 0.2, 0.3]),
            layer: MemoryLayer::Project,
            summaries,
            context_vector: Some(vec![0.4, 0.5, 0.6]),
            importance_score: Some(0.85),
            metadata: std::collections::HashMap::new(),
            created_at: 1705500000,
            updated_at: 1705500000,
        };

        assert_eq!(entry.summaries.len(), 2);
        assert!(entry.summaries.contains_key(&SummaryDepth::Sentence));
        assert!(entry.summaries.contains_key(&SummaryDepth::Paragraph));
        assert!(entry.context_vector.is_some());
        assert_eq!(entry.context_vector.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn test_memory_entry_without_summaries() {
        let entry = MemoryEntry {
            id: "mem_002".to_string(),
            content: "Content without summaries yet.".to_string(),
            embedding: None,
            layer: MemoryLayer::Session,
            summaries: std::collections::HashMap::new(),
            context_vector: None,
            importance_score: None,
            metadata: std::collections::HashMap::new(),
            created_at: 1705500000,
            updated_at: 1705500000,
        };

        assert!(entry.summaries.is_empty());
        assert!(entry.context_vector.is_none());
    }

    #[test]
    fn test_memory_entry_serialization_with_summaries() {
        let mut summaries = std::collections::HashMap::new();
        summaries.insert(
            SummaryDepth::Detailed,
            LayerSummary {
                depth: SummaryDepth::Detailed,
                content: "Detailed summary content.".to_string(),
                token_count: 50,
                generated_at: 1705500000,
                source_hash: "hash_abc".to_string(),
                content_hash: None,
                personalized: true,
                personalization_context: Some("developer".to_string()),
            },
        );

        let entry = MemoryEntry {
            id: "test".to_string(),
            content: "Test content".to_string(),
            embedding: None,
            layer: MemoryLayer::User,
            summaries,
            context_vector: Some(vec![1.0, 2.0]),
            importance_score: None,
            metadata: std::collections::HashMap::new(),
            created_at: 0,
            updated_at: 0,
        };

        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: MemoryEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(entry.id, deserialized.id);
        assert_eq!(entry.summaries.len(), deserialized.summaries.len());
        assert_eq!(entry.context_vector, deserialized.context_vector);
    }

    #[test]
    fn test_summary_depth_all_layers_supported() {
        let layers = vec![
            MemoryLayer::Agent,
            MemoryLayer::User,
            MemoryLayer::Session,
            MemoryLayer::Project,
            MemoryLayer::Team,
            MemoryLayer::Org,
            MemoryLayer::Tenant,
        ];

        for layer in layers {
            let config = SummaryConfig {
                layer,
                update_interval_secs: Some(3600),
                update_on_changes: None,
                skip_if_unchanged: true,
                personalized: false,
                depths: vec![SummaryDepth::Sentence],
            };

            let json = serde_json::to_string(&config).unwrap();
            let deserialized: SummaryConfig = serde_json::from_str(&json).unwrap();
            assert_eq!(config.layer, deserialized.layer);
        }
    }

    #[test]
    fn test_memory_entry_needs_summary_update_missing_summary() {
        let entry = MemoryEntry {
            id: "test".to_string(),
            content: "Test content".to_string(),
            embedding: None,
            layer: MemoryLayer::Project,
            summaries: std::collections::HashMap::new(),
            context_vector: None,
            importance_score: None,
            metadata: std::collections::HashMap::new(),
            created_at: 1000,
            updated_at: 1000,
        };

        let config = SummaryConfig {
            layer: MemoryLayer::Project,
            update_interval_secs: Some(3600),
            update_on_changes: None,
            skip_if_unchanged: true,
            personalized: false,
            depths: vec![SummaryDepth::Sentence],
        };

        assert!(entry.needs_summary_update(&config, 2000));
    }

    #[test]
    fn test_memory_entry_needs_summary_update_stale_time() {
        let content = "Test content";
        let content_hash = {
            use sha2::{Digest, Sha256};
            hex::encode(Sha256::digest(content.as_bytes()))
        };

        let mut summaries = std::collections::HashMap::new();
        summaries.insert(
            SummaryDepth::Sentence,
            LayerSummary {
                depth: SummaryDepth::Sentence,
                content: "Summary".to_string(),
                token_count: 1,
                generated_at: 1000,
                source_hash: content_hash,
                content_hash: None,
                personalized: false,
                personalization_context: None,
            },
        );

        let entry = MemoryEntry {
            id: "test".to_string(),
            content: content.to_string(),
            embedding: None,
            layer: MemoryLayer::Project,
            summaries,
            context_vector: None,
            importance_score: None,
            metadata: std::collections::HashMap::new(),
            created_at: 1000,
            updated_at: 1000,
        };

        let config = SummaryConfig {
            layer: MemoryLayer::Project,
            update_interval_secs: Some(3600),
            update_on_changes: None,
            skip_if_unchanged: false,
            personalized: false,
            depths: vec![SummaryDepth::Sentence],
        };

        assert!(!entry.needs_summary_update(&config, 2000));
        assert!(entry.needs_summary_update(&config, 5000));
    }

    #[test]
    fn test_memory_entry_needs_summary_update_content_changed() {
        let mut summaries = std::collections::HashMap::new();
        summaries.insert(
            SummaryDepth::Sentence,
            LayerSummary {
                depth: SummaryDepth::Sentence,
                content: "Summary".to_string(),
                token_count: 1,
                generated_at: 1000,
                source_hash: "old_hash".to_string(),
                content_hash: None,
                personalized: false,
                personalization_context: None,
            },
        );

        let entry = MemoryEntry {
            id: "test".to_string(),
            content: "New content".to_string(),
            embedding: None,
            layer: MemoryLayer::Project,
            summaries,
            context_vector: None,
            importance_score: None,
            metadata: std::collections::HashMap::new(),
            created_at: 1000,
            updated_at: 2000,
        };

        let config = SummaryConfig {
            layer: MemoryLayer::Project,
            update_interval_secs: Some(3600),
            update_on_changes: None,
            skip_if_unchanged: true,
            personalized: false,
            depths: vec![SummaryDepth::Sentence],
        };

        assert!(entry.needs_summary_update(&config, 1500));
    }

    #[test]
    fn test_memory_entry_needs_summary_update_no_update_needed() {
        let content = "Same content";
        let content_hash = {
            use sha2::{Digest, Sha256};
            hex::encode(Sha256::digest(content.as_bytes()))
        };

        let mut summaries = std::collections::HashMap::new();
        summaries.insert(
            SummaryDepth::Sentence,
            LayerSummary {
                depth: SummaryDepth::Sentence,
                content: "Summary".to_string(),
                token_count: 1,
                generated_at: 1000,
                source_hash: content_hash,
                content_hash: None,
                personalized: false,
                personalization_context: None,
            },
        );

        let entry = MemoryEntry {
            id: "test".to_string(),
            content: content.to_string(),
            embedding: None,
            layer: MemoryLayer::Project,
            summaries,
            context_vector: None,
            importance_score: None,
            metadata: std::collections::HashMap::new(),
            created_at: 1000,
            updated_at: 1000,
        };

        let config = SummaryConfig {
            layer: MemoryLayer::Project,
            update_interval_secs: Some(3600),
            update_on_changes: None,
            skip_if_unchanged: true,
            personalized: false,
            depths: vec![SummaryDepth::Sentence],
        };

        assert!(!entry.needs_summary_update(&config, 2000));
    }

    #[test]
    fn test_memory_entry_compute_content_hash() {
        let entry = MemoryEntry {
            id: "test".to_string(),
            content: "Test content".to_string(),
            embedding: None,
            layer: MemoryLayer::User,
            summaries: std::collections::HashMap::new(),
            context_vector: None,
            importance_score: None,
            metadata: std::collections::HashMap::new(),
            created_at: 0,
            updated_at: 0,
        };

        let hash = entry.compute_content_hash();
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64);

        let hash2 = entry.compute_content_hash();
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_memory_entry_compute_content_hash_different_content() {
        let entry1 = MemoryEntry {
            id: "test1".to_string(),
            content: "Content A".to_string(),
            embedding: None,
            layer: MemoryLayer::User,
            summaries: std::collections::HashMap::new(),
            context_vector: None,
            importance_score: None,
            metadata: std::collections::HashMap::new(),
            created_at: 0,
            updated_at: 0,
        };

        let entry2 = MemoryEntry {
            id: "test2".to_string(),
            content: "Content B".to_string(),
            embedding: None,
            layer: MemoryLayer::User,
            summaries: std::collections::HashMap::new(),
            context_vector: None,
            importance_score: None,
            metadata: std::collections::HashMap::new(),
            created_at: 0,
            updated_at: 0,
        };

        assert_ne!(entry1.compute_content_hash(), entry2.compute_content_hash());
    }

    #[test]
    fn test_knowledge_type_hindsight_variant() {
        let hindsight = KnowledgeType::Hindsight;
        let json = serde_json::to_string(&hindsight).unwrap();
        assert_eq!(json, "\"Hindsight\"");

        let parsed: KnowledgeType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, KnowledgeType::Hindsight);
    }

    #[test]
    fn test_error_signature_creation() {
        let sig = ErrorSignature {
            error_type: "NullPointerException".to_string(),
            message_pattern: "Cannot read property '.*' of undefined".to_string(),
            stack_patterns: vec!["at UserService".to_string(), "at AuthHandler".to_string()],
            context_patterns: vec!["typescript".to_string(), "react".to_string()],
            embedding: Some(vec![0.1, 0.2, 0.3]),
        };

        assert_eq!(sig.error_type, "NullPointerException");
        assert_eq!(sig.stack_patterns.len(), 2);
        assert!(sig.embedding.is_some());
    }

    #[test]
    fn test_error_signature_serialization() {
        let sig = ErrorSignature {
            error_type: "TypeError".to_string(),
            message_pattern: ".*is not a function".to_string(),
            stack_patterns: vec![],
            context_patterns: vec!["javascript".to_string()],
            embedding: None,
        };

        let json = serde_json::to_string(&sig).unwrap();
        assert!(json.contains("\"errorType\""));
        assert!(json.contains("\"messagePattern\""));

        let parsed: ErrorSignature = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.error_type, sig.error_type);
    }

    #[test]
    fn test_code_change_creation() {
        let change = CodeChange {
            file_path: "src/auth.rs".to_string(),
            diff: "+ if let Some(token) = token_option {\n+     // handle\n+ }".to_string(),
            description: Some("Added null check for token".to_string()),
        };

        assert_eq!(change.file_path, "src/auth.rs");
        assert!(change.description.is_some());
    }

    #[test]
    fn test_resolution_creation() {
        let resolution = Resolution {
            id: "res_001".to_string(),
            error_signature_id: "sig_001".to_string(),
            description: "Add null check before accessing token fields".to_string(),
            changes: vec![CodeChange {
                file_path: "src/auth.rs".to_string(),
                diff: "+ if token.is_some()".to_string(),
                description: None,
            }],
            success_rate: 0.95,
            application_count: 12,
            last_success_at: 1705500000,
        };

        assert_eq!(resolution.success_rate, 0.95);
        assert_eq!(resolution.application_count, 12);
        assert_eq!(resolution.changes.len(), 1);
    }

    #[test]
    fn test_resolution_serialization() {
        let resolution = Resolution {
            id: "res_002".to_string(),
            error_signature_id: "sig_002".to_string(),
            description: "Fix".to_string(),
            changes: vec![],
            success_rate: 1.0,
            application_count: 5,
            last_success_at: 1705600000,
        };

        let json = serde_json::to_string(&resolution).unwrap();
        assert!(json.contains("\"successRate\""));
        assert!(json.contains("\"applicationCount\""));

        let parsed: Resolution = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, resolution.id);
    }

    #[test]
    fn test_hindsight_note_creation() {
        let note = HindsightNote {
            id: "hn_001".to_string(),
            error_signature: ErrorSignature {
                error_type: "CompileError".to_string(),
                message_pattern: "missing lifetime specifier".to_string(),
                stack_patterns: vec![],
                context_patterns: vec!["rust".to_string()],
                embedding: None,
            },
            resolutions: vec![Resolution {
                id: "res_001".to_string(),
                error_signature_id: "sig_001".to_string(),
                description: "Add explicit lifetime annotation".to_string(),
                changes: vec![],
                success_rate: 0.88,
                application_count: 8,
                last_success_at: 1705500000,
            }],
            content: "# Rust Lifetime Error\n\nWhen encountering...".to_string(),
            tags: vec![
                "rust".to_string(),
                "lifetimes".to_string(),
                "borrow-checker".to_string(),
            ],
            created_at: 1705400000,
            updated_at: 1705500000,
        };

        assert_eq!(note.id, "hn_001");
        assert_eq!(note.resolutions.len(), 1);
        assert_eq!(note.tags.len(), 3);
    }

    #[test]
    fn test_hindsight_note_serialization_roundtrip() {
        let note = HindsightNote {
            id: "hn_002".to_string(),
            error_signature: ErrorSignature {
                error_type: "RuntimeError".to_string(),
                message_pattern: "index out of bounds".to_string(),
                stack_patterns: vec!["at main".to_string()],
                context_patterns: vec!["rust".to_string()],
                embedding: Some(vec![0.5, 0.6]),
            },
            resolutions: vec![],
            content: "# Array Bounds Error".to_string(),
            tags: vec!["rust".to_string()],
            created_at: 1705400000,
            updated_at: 1705400000,
        };

        let json = serde_json::to_string(&note).unwrap();
        let parsed: HindsightNote = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, note.id);
        assert_eq!(
            parsed.error_signature.error_type,
            note.error_signature.error_type
        );
        assert_eq!(parsed.tags, note.tags);
    }

    mod role_identifier_tests {
        use super::*;
        use std::collections::HashSet;
        use std::hash::{Hash, Hasher};
        use std::str::FromStr;

        #[test]
        fn test_role_identifier_serde_roundtrip_known_variant_expected() {
            let role = RoleIdentifier::Known(Role::Admin);
            let json = serde_json::to_string(&role).unwrap();
            assert_eq!(json, "\"Admin\"");

            let deserialized: RoleIdentifier = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, RoleIdentifier::Known(Role::Admin));
        }

        #[test]
        fn test_role_identifier_serde_roundtrip_custom_variant_expected() {
            let role = RoleIdentifier::Custom("billingOwner".to_string());
            let json = serde_json::to_string(&role).unwrap();
            assert_eq!(json, "\"billingOwner\"");

            let deserialized: RoleIdentifier = serde_json::from_str(&json).unwrap();
            assert_eq!(
                deserialized,
                RoleIdentifier::Custom("billingOwner".to_string())
            );
        }

        #[test]
        fn test_role_identifier_partial_eq_cross_variant_case_insensitive_expected() {
            assert_eq!(
                RoleIdentifier::Known(Role::Admin),
                RoleIdentifier::Custom("admin".to_string())
            );
            assert_eq!(
                RoleIdentifier::Known(Role::Admin),
                RoleIdentifier::Custom("AdMiN".to_string())
            );
        }

        #[test]
        fn test_role_identifier_hash_consistency_equal_values_same_hash_expected() {
            let known = RoleIdentifier::Known(Role::Admin);
            let custom = RoleIdentifier::Custom("admin".to_string());

            let mut hasher_known = std::collections::hash_map::DefaultHasher::new();
            known.hash(&mut hasher_known);
            let known_hash = hasher_known.finish();

            let mut hasher_custom = std::collections::hash_map::DefaultHasher::new();
            custom.hash(&mut hasher_custom);
            let custom_hash = hasher_custom.finish();

            assert_eq!(known_hash, custom_hash);
        }

        #[test]
        fn test_role_identifier_display_known_and_custom_expected() {
            assert_eq!(RoleIdentifier::Known(Role::Admin).to_string(), "Admin");
            assert_eq!(
                RoleIdentifier::Custom("billingOwner".to_string()).to_string(),
                "billingOwner"
            );
        }

        #[test]
        fn test_role_identifier_from_str_flexible_known_and_custom_expected() {
            assert_eq!(
                RoleIdentifier::from_str_flexible("admin"),
                RoleIdentifier::Known(Role::Admin)
            );
            assert_eq!(
                RoleIdentifier::from_str_flexible("BillingOwner"),
                RoleIdentifier::Custom("BillingOwner".to_string())
            );
        }

        #[test]
        fn test_role_identifier_as_cedar_entity_id_known_and_custom_expected() {
            assert_eq!(
                RoleIdentifier::Known(Role::Admin).as_cedar_entity_id(),
                "Admin"
            );
            assert_eq!(
                RoleIdentifier::Custom("billingOwner".to_string()).as_cedar_entity_id(),
                "billingOwner"
            );
        }

        #[test]
        fn test_role_identifier_from_role_conversion_expected() {
            let role_identifier: RoleIdentifier = Role::TechLead.into();
            assert_eq!(role_identifier, RoleIdentifier::Known(Role::TechLead));
        }

        #[test]
        fn test_role_identifier_helpers_known_and_custom_expected() {
            let known = RoleIdentifier::Known(Role::Architect);
            assert!(known.is_known());
            assert!(!known.is_custom());
            assert_eq!(known.as_known(), Some(&Role::Architect));

            let custom = RoleIdentifier::Custom("billingOwner".to_string());
            assert!(!custom.is_known());
            assert!(custom.is_custom());
            assert_eq!(custom.as_known(), None);
        }

        #[test]
        fn test_tenant_context_role_helpers_with_role_and_with_roles_expected() {
            let tenant_id = TenantId::new("tenant-1".to_string()).unwrap();
            let user_id = UserId::new("user-1".to_string()).unwrap();

            let ctx = TenantContext::new(tenant_id.clone(), user_id.clone())
                .with_role(Role::Developer)
                .with_role(RoleIdentifier::Custom("BillingOwner".to_string()));

            assert!(ctx.has_role(&RoleIdentifier::Known(Role::Developer)));
            assert!(ctx.has_known_role(&Role::Developer));
            assert!(ctx.has_role(&RoleIdentifier::Custom("BillingOwner".to_string())));

            let replaced = TenantContext::new(tenant_id, user_id).with_roles(vec![
                RoleIdentifier::Known(Role::Viewer),
                RoleIdentifier::Custom("Support".to_string()),
            ]);

            assert!(replaced.has_known_role(&Role::Viewer));
            assert!(replaced.has_role(&RoleIdentifier::Custom("Support".to_string())));
            assert!(!replaced.has_known_role(&Role::Developer));
        }

        #[test]
        fn test_tenant_context_highest_precedence_role_mixed_roles_expected() {
            let ctx = TenantContext::new(
                TenantId::new("tenant-1".to_string()).unwrap(),
                UserId::new("user-1".to_string()).unwrap(),
            )
            .with_roles(vec![
                RoleIdentifier::Custom("customRole".to_string()),
                RoleIdentifier::Known(Role::Admin),
                RoleIdentifier::Known(Role::TenantAdmin),
            ]);

            assert_eq!(
                ctx.highest_precedence_role(),
                Some(&RoleIdentifier::Known(Role::TenantAdmin))
            );
        }

        #[test]
        fn test_role_identifier_backward_compat_old_json_known_role_expected() {
            let json = r#"{"tenant_id":"tenant-1","user_id":"user-1","roles":["admin"]}"#;
            let ctx: TenantContext = serde_json::from_str(json).unwrap();

            assert_eq!(ctx.roles, vec![RoleIdentifier::Known(Role::Admin)]);
        }

        #[test]
        fn test_role_identifier_backward_compat_unknown_string_custom_expected() {
            let json = r#"{"tenant_id":"tenant-1","user_id":"user-1","roles":["billingOwner"]}"#;
            let ctx: TenantContext = serde_json::from_str(json).unwrap();

            assert_eq!(
                ctx.roles,
                vec![RoleIdentifier::Custom("billingOwner".to_string())]
            );
        }

        #[test]
        fn test_role_identifier_all_known_variants_roundtrip_expected() {
            let known_roles = [
                Role::Viewer,
                Role::Developer,
                Role::TechLead,
                Role::Architect,
                Role::Admin,
                Role::TenantAdmin,
                Role::Agent,
                Role::PlatformAdmin,
            ];

            for role in known_roles {
                let wrapped = RoleIdentifier::Known(role);
                let json = serde_json::to_string(&wrapped).unwrap();
                let deserialized: RoleIdentifier = serde_json::from_str(&json).unwrap();
                assert_eq!(deserialized, wrapped);
            }
        }

        #[test]
        fn test_role_identifier_hash_set_dedup_known_and_custom_case_expected() {
            let mut set = HashSet::new();
            set.insert(RoleIdentifier::Known(Role::Admin));
            set.insert(RoleIdentifier::Custom("admin".to_string()));

            assert_eq!(set.len(), 1);
        }

        #[test]
        fn test_role_identifier_from_str_trait_known_and_custom_expected() {
            assert_eq!(
                RoleIdentifier::from_str("admin").unwrap(),
                RoleIdentifier::Known(Role::Admin)
            );
            assert_eq!(
                RoleIdentifier::from_str("billingOwner").unwrap(),
                RoleIdentifier::Custom("billingOwner".to_string())
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Git provider connection types (task 3.4)
// ---------------------------------------------------------------------------

/// The kind of Git provider backing a platform-owned connection.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, EnumString, Display,
)]
#[strum(ascii_case_insensitive)]
pub enum GitProviderKind {
    GitHubApp,
}

/// A platform-owned GitHub App connection record.
///
/// The PEM private key is never stored inline; `pem_secret_ref` holds a
/// secret-provider reference (e.g. `local/...`, `secret/...`, `arn:aws:...`).
/// One connection can be shared with multiple tenants via `allowed_tenant_ids`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GitProviderConnection {
    /// Stable identifier for this connection.
    pub id: String,
    /// Human-readable label.
    pub name: String,
    /// Provider kind (currently only GitHubApp).
    pub provider_kind: GitProviderKind,
    /// GitHub App ID.
    pub app_id: u64,
    /// GitHub App installation ID.
    pub installation_id: u64,
    /// Secret-provider reference to the PEM private key.
    pub pem_secret_ref: String,
    /// Optional secret-provider reference to the webhook secret.
    pub webhook_secret_ref: Option<String>,
    /// Tenants allowed to reference this connection in their repository binding.
    pub allowed_tenant_ids: Vec<TenantId>,
    #[schema(value_type = String, format = DateTime, example = "2026-04-30T15:34:00Z")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = String, format = DateTime, example = "2026-04-30T15:34:00Z")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl GitProviderConnection {
    /// Returns `true` if the connection ID is safe to use in manifests, CLI flags,
    /// and chart values.
    #[must_use]
    pub fn has_valid_id(&self) -> bool {
        !self.id.is_empty()
            && !self.id.starts_with('-')
            && !self.id.ends_with('-')
            && self
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    }

    /// Returns `true` if the PEM reference uses a supported secret-provider prefix.
    #[must_use]
    pub fn has_valid_pem_ref(&self) -> bool {
        self.pem_secret_ref.starts_with("local/")
            || self.pem_secret_ref.starts_with("secret/")
            || self.pem_secret_ref.starts_with("arn:aws:")
    }

    /// Returns `true` when `tenant_id` is in the allow-list.
    #[must_use]
    pub fn is_visible_to(&self, tenant_id: &TenantId) -> bool {
        self.allowed_tenant_ids.contains(tenant_id)
    }

    /// Returns a redacted JSON view of this connection (PEM ref masked).
    #[must_use]
    pub fn redacted(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "providerKind": self.provider_kind.to_string(),
            "appId": self.app_id,
            "installationId": self.installation_id,
            "pemSecretRef": "[redacted]",
            "webhookSecretRef": self.webhook_secret_ref.as_deref().map(|_| "[redacted]"),
            "allowedTenantIds": self.allowed_tenant_ids.iter().map(|t| t.as_str()).collect::<Vec<_>>(),
            "createdAt": self.created_at,
            "updatedAt": self.updated_at,
        })
    }
}
