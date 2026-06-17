//! Reverse-render a persisted tenant's state as a `TenantManifest`-shaped
//! document. B3 tasks 2.2 + 2.3.
//!
//! # What this is for
//!
//! GitOps workflows need a read-side counterpart to `POST provision_tenant`:
//! "show me the manifest I could re-apply to recreate this tenant's
//! current state". This module produces that document, with two modes:
//!
//! * **Full** (`redact=false`): includes field values, secret-reference
//!   logical names, and repository credentials. Gated at the HTTP layer
//!   by the `PlatformAdmin` role.
//! * **Redacted** (`redact=true`): secret-reference logical names are
//!   replaced with opaque placeholders (`secret-0`, `secret-1`, ...) and
//!   repository `credentialRef` is elided. Safe to share with callers
//!   holding `tenant:read` only.
//!
//! # What this is NOT for
//!
//! Not a round-trip guarantee. Several manifest sections are persisted
//! through different pipelines (hierarchy via governance, roles via
//! role_grants, providers via the provider_registry) and reading them
//! back cleanly would require threading through authorization contexts
//! this module does not own. Sections we cannot currently reverse-render
//! are enumerated in [`RenderedManifest::not_rendered`] so callers can
//! tell the difference between "this section is empty" and "this section
//! was not read". The full-fidelity renderer is a later task (§2.4 diff
//! needs it to be trustworthy; until then, only the listed sections
//! participate in a meaningful diff).

use std::collections::BTreeMap;
use std::sync::Arc;

use uuid::Uuid;

use mk_core::traits::TenantConfigProvider;
use mk_core::types::{
    TenantConfigField, TenantConfigOwnership, TenantId, TenantRecord, TenantSecretReference,
};
use serde::Serialize;
use serde_json::{Value, json};

use super::AppState;

/// Sections this renderer knows how to reverse-render today.
///
/// `domainMappings` lives under `tenant.domainMappings` in the manifest
/// shape; it is listed here at the top level because that is where the
/// API documentation exposes it and where `notRendered` previously
/// tracked it. Added in PR #128.
///
/// `providers` (llm + embedding) was added in §2.2-A — see
/// `render_providers` below and the `#[test] render_providers_*` cases.
pub const RENDERED_SECTIONS: &[&str] = &[
    "tenant",
    "metadata",
    "config",
    "repository",
    "providers",
    "domainMappings",
    "hierarchy",
    "roles",
];

/// Sections a full `TenantManifest` can carry but this renderer does
/// not yet cover. Reflected into [`RenderedManifest::not_rendered`].
///
/// Intentionally excluded from this list (they are NOT gaps):
///
/// - **`secrets`** — the top-level `secrets:` array on the manifest
///   INPUT shape carries plaintext values used for one-shot ingestion
///   (`ManifestSecret { logical_name, ownership, secret_value }`). The
///   server never retains plaintext, so there is no current state to
///   re-emit. This is the same wire-only semantic as
///   `SecretReference::Inline`: it can arrive, but it cannot leave.
///   The durable form of those secrets is visible in
///   `config.secretReferences` (already rendered). (PR #128)
///
/// - **`tenant.domainMappings`** moved to [`RENDERED_SECTIONS`]; it is
///   emitted as a sorted `Vec<String>` and elided entirely when the
///   tenant has no mappings (round-trip friendly with the input shape,
///   which allows the field to be absent). (PR #128)
///
/// - **`providers`** (top-level) moved to [`RENDERED_SECTIONS`] in
///   §2.2-A. Its `memoryLayers` sub-section closes in §2.2-D (this
///   change) — layer keys now live under
///   `memory_layer.{layer}.{provider,model,api_key,…}` in the flat
///   config namespace, reverse-recovered by `render_memory_layers`.
///
/// - **`hierarchy`** moved to [`RENDERED_SECTIONS`] in §2.2-B (B3+B4).
///   Forward-apply writes the modern `organizations`/`teams` tables via
///   `HierarchyStore::upsert_hierarchy`; reverse-render reads them back
///   via `HierarchyStore::get_hierarchy` and transforms to
///   [`RenderedOrg`]/[`RenderedTeam`] through [`render_hierarchy`]. OU
///   writes remain in place for backup/gdpr/cascade back-compat but are
///   *not* the reverse-render source.
///
/// - **`roles`** closed in §2.2-C (PR #145). Forward-apply has always
///   written `user_roles`; reverse-render uses [`render_roles`] + a
///   name-resolution query in [`render_current_manifest`] that prefers
///   the modern hierarchy (`v_hierarchy`) and falls back to legacy
///   `organizational_units` rows while `user_roles.unit_id` still
///   references OU ids. `unit_id == tenant_id` is the sentinel for
///   "tenant-wide" (matching the forward-apply convention in
///   `provision_tenant` Step 7) and maps to `unit: None` on render.
///
/// With §2.2-D (this change) every manifest section now round-trips;
/// `NOT_RENDERED_SECTIONS` is empty and the structural-diff work in
/// §2.4 is unblocked.
pub const NOT_RENDERED_SECTIONS: &[&str] = &[];

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("tenant not found: {0}")]
    NotFound(String),
    #[error("storage error: {0}")]
    Storage(String),
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RenderedManifest {
    pub api_version: String,
    pub kind: String,
    pub metadata: RenderedMetadata,
    pub tenant: RenderedTenant,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<RenderedConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<Value>,
    /// Reconstructed from `config.fields` + `config.secret_references`
    /// by `render_providers`. Elided entirely when no provider has been
    /// declared for this tenant (neither `llm_provider` nor
    /// `embedding_provider` field present) so the rendered manifest
    /// stays byte-identical to the input for tenants that don't use
    /// the providers block.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub providers: Option<RenderedProviders>,
    /// Reconstructed from tenant-root `organizations` / `teams`
    /// (via `HierarchyStore::get_hierarchy` + `render_hierarchy`).
    /// Elided entirely when the tenant has no organizations declared,
    /// so the rendered manifest stays byte-identical to an input
    /// manifest whose `hierarchy:` field was absent or empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hierarchy: Option<Vec<RenderedOrg>>,
    /// §2.2-C reverse-render of `user_roles`. `None` (and therefore
    /// elided from the rendered JSON) for tenants with no role
    /// assignments, so the rendered manifest stays byte-identical to
    /// an input manifest whose `roles:` was absent or empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<RenderedRoleAssignment>>,
    pub not_rendered: Vec<&'static str>,
}

/// §2.2-B reverse-render shape mirroring the tenant-root manifest
/// hierarchy (`ManifestOrg`).
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RenderedOrg {
    pub slug: String,
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub teams: Vec<RenderedTeam>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RenderedTeam {
    pub slug: String,
    pub name: String,
}

/// §2.2-C reverse-render shape mirroring `ManifestRoleAssignment`.
///
/// `unit` is `None` when the role is tenant-wide (forward-apply in
/// `provision_tenant` Step 7 stores this as `unit_id = tenant_id`).
/// Otherwise it's the resolved hierarchy unit's *name*
/// (operator-facing identifier), falling back to the raw `unit_id`
/// when neither the modern hierarchy nor the legacy OU row can be
/// found. That fallback is deliberate: emitting the stored id keeps
/// the rendered manifest losslessly round-trippable even over orphaned
/// rows, and surfaces the integrity drift to the operator rather than
/// silently dropping the grant.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RenderedRoleAssignment {
    pub user_id: String,
    pub role: mk_core::types::RoleIdentifier,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

/// Raw row shape emitted by the [`render_current_manifest`] roles
/// query — exposed publicly so unit tests can cover the transform in
/// isolation without a live DB.
#[derive(Debug, Clone, PartialEq)]
pub struct RoleRow {
    pub user_id: String,
    pub unit_id: String,
    pub role: String,
    /// `Some(name)` when the matching hierarchy row exists (modern
    /// `v_hierarchy` first, legacy `organizational_units` fallback);
    /// `None` when the lookup missed entirely (either because
    /// `unit_id == tenant_id` and there is no unit row for that
    /// sentinel, or because the backing hierarchy row was deleted and
    /// only the grant remains).
    pub unit_name: Option<String>,
}

/// Convert the `HierarchyStore` read shape into the manifest-render
/// surface. Pure transform — no DB access, no redact-mode toggles
/// (hierarchy carries no secrets).
pub fn render_hierarchy(orgs: Vec<storage::hierarchy_store::Org>) -> Vec<RenderedOrg> {
    orgs.into_iter()
        .map(|o| RenderedOrg {
            slug: o.slug,
            name: o.name,
            teams: o
                .teams
                .into_iter()
                .map(|t| RenderedTeam {
                    slug: t.slug,
                    name: t.name,
                })
                .collect(),
        })
        .collect()
}

/// Transform `user_roles` rows (with hierarchy names already resolved)
/// into the manifest reverse-render shape.
///
/// Pure; no DB access, no redact toggles (roles carry no secrets).
///
/// * `tenant_id_str` — the tenant id as stored in `user_roles.tenant_id`.
///   Used to detect the tenant-wide sentinel (`unit_id == tenant_id`).
/// * `rows` — consumed to avoid clones for fields we're moving into
///   `RenderedRoleAssignment`.
pub fn render_roles(tenant_id_str: &str, rows: Vec<RoleRow>) -> Vec<RenderedRoleAssignment> {
    rows.into_iter()
        .map(|r| {
            let unit = if r.unit_id == tenant_id_str {
                None
            } else {
                Some(r.unit_name.unwrap_or(r.unit_id))
            };
            RenderedRoleAssignment {
                user_id: r.user_id,
                role: mk_core::types::RoleIdentifier::from_str_flexible(&r.role),
                unit,
            }
        })
        .collect()
}

/// Mirror of `ManifestProviders` used by the input side
/// (`tenant_api::ManifestProviders`). As of §2.2-D, round-trip is
/// complete: `llm`, `embedding`, and `memoryLayers` all forward-apply
/// and reverse-render.
///
/// We redeclare the type here instead of reusing the input type
/// because the input type is `Deserialize`-only (no `Serialize`) and
/// changing its derive surface would ripple through validator tests.
/// The two must stay field-compatible for round-trip to work; a
/// dedicated `round_trip_manifest_providers` test in tenant_api.rs
/// locks that contract end-to-end.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RenderedProviders {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm: Option<RenderedProvider>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<RenderedProvider>,
    /// Reconstructed from `memory_layer.{layer}.*` fields by
    /// [`render_memory_layers`]. Elided entirely when no layer has
    /// been declared for this tenant (keeps round-trip byte-identical
    /// for tenants that don't use memoryLayers).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub memory_layers: BTreeMap<String, RenderedProvider>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RenderedProvider {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The name the operator used in `config.secretReferences` when
    /// they originally declared this provider. Recovered by matching
    /// the canonical `{llm,embedding}_api_key` alias back to the
    /// operator-named entry by reference-equality on the underlying
    /// `SecretReference`. Falls back to the canonical name itself
    /// if no other entry matches (e.g. when the provider was
    /// configured via the dedicated `PUT .../providers/{llm,embedding}`
    /// handler which only writes the canonical name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_ref: Option<String>,
    /// Provider-specific non-sensitive config (e.g. `projectId` for
    /// google, `region` for bedrock). Elided when empty so round-trip
    /// output stays minimal.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RenderedMetadata {
    pub generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_hash: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RenderedTenant {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub status: String,
    pub source_owner: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Domains mapped to this tenant, sorted lexicographically.
    /// Omitted entirely when the tenant has no mappings, matching the
    /// input shape (`ManifestTenant.domain_mappings` is `Option<Vec<_>>`)
    /// and keeping the rendered manifest byte-identical to what a caller
    /// would write for an unmapped tenant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_mappings: Option<Vec<String>>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RenderedConfig {
    pub fields: BTreeMap<String, TenantConfigField>,
    pub secret_references: BTreeMap<String, RenderedSecretRef>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RenderedSecretRef {
    pub logical_name: String,
    pub ownership: TenantConfigOwnership,
    #[serde(flatten)]
    pub reference: Value,
}

pub async fn render_current_manifest(
    state: &Arc<AppState>,
    tenant_ref: &str,
    redact: bool,
) -> Result<RenderedManifest, RenderError> {
    let record: TenantRecord = state
        .tenant_store
        .get_tenant(tenant_ref)
        .await
        .map_err(|e| RenderError::Storage(e.to_string()))?
        .ok_or_else(|| RenderError::NotFound(tenant_ref.to_string()))?;

    let tenant_id: TenantId = record.id.clone();

    // `get_manifest_state` takes a slug (not a `TenantId`) and returns
    // a flat `(Option<hash>, generation_i64)` tuple. The generation
    // column is `BIGINT NOT NULL DEFAULT 0` in migration 027, so zero
    // is the schema default for a tenant that has never been applied.
    let (manifest_hash, generation_i64) = state
        .tenant_store
        .get_manifest_state(&record.slug)
        .await
        .map_err(|e| RenderError::Storage(e.to_string()))?;
    // u64 at the wire. Negative generations are a schema-invariant
    // violation; we clamp with `max(0)` rather than panic so the
    // renderer stays read-only.
    let generation: u64 = generation_i64.max(0) as u64;

    let config_doc = state
        .tenant_config_provider
        .get_config(&tenant_id)
        .await
        .map_err(|e| RenderError::Storage(e.to_string()))?;

    // Providers reverse-render reads the SAME fields/secret_references
    // the apply helper writes, so we compute it BEFORE handing those
    // maps to `render_config` (which consumes them). Redact mode
    // elides `secret_ref` because the redacted rendering replaces
    // operator names with opaque `secret-N` placeholders; exposing the
    // placeholder name in `secret_ref` would leak the set ordering
    // without giving the consumer anything actionable.
    let rendered_providers = config_doc
        .as_ref()
        .and_then(|doc| render_providers(&doc.fields, &doc.secret_references, redact));

    let rendered_config =
        config_doc.map(|doc| render_config(doc.fields, doc.secret_references, redact));

    // Repository binding lives on its own store (see AppState):
    // `TenantRepositoryBindingStore`, not `TenantStore`. Kept separate
    // in storage because the sync-owner guard logic there is orthogonal
    // to tenant lifecycle.
    let binding = state
        .tenant_repository_binding_store
        .get_binding(&tenant_id)
        .await
        .map_err(|e| RenderError::Storage(e.to_string()))?;

    // Domain mappings are listed via the store (which also resolves the
    // tenant ref; we pass the slug we just loaded above, so the result
    // set is guaranteed non-`NotFound`). Empty => None so the rendered
    // manifest matches an unmapped-tenant input byte-for-byte.
    let domain_mappings = state
        .tenant_store
        .list_domain_mappings(&record.slug)
        .await
        .map_err(|e| RenderError::Storage(e.to_string()))?;
    let domain_mappings = if domain_mappings.is_empty() {
        None
    } else {
        Some(domain_mappings)
    };

    let rendered_repository = binding.map(|b| {
        if redact {
            b.redacted()
        } else {
            json!({
                "id": b.id,
                "tenantId": b.tenant_id.as_str(),
                "kind": b.kind.to_string(),
                "localPath": b.local_path,
                "remoteUrl": b.remote_url,
                "branch": b.branch,
                "branchPolicy": b.branch_policy.to_string(),
                "credentialKind": b.credential_kind.to_string(),
                "credentialRef": b.credential_ref,
                "gitProviderConnectionId": b.git_provider_connection_id,
                "githubOwner": b.github_owner,
                "githubRepo": b.github_repo,
                "sourceOwner": b.source_owner.to_string(),
                "createdAt": b.created_at,
                "updatedAt": b.updated_at,
            })
        }
    });

    // B4 §2.2-B — reverse-render hierarchy from the modern tables.
    // Tenants whose id doesn't parse as UUID (should never happen on a
    // correctly-migrated deployment) get `None` rather than an error:
    // the renderer is intentionally read-only and tolerant of
    // pre-migration rows so operators can still diff/render broken
    // states while investigating.
    let hierarchy = match Uuid::parse_str(record.id.as_str()) {
        Ok(tenant_uuid) => {
            let store =
                storage::hierarchy_store::HierarchyStore::new(state.postgres.pool().clone());
            let org_roots = store
                .get_hierarchy(tenant_uuid)
                .await
                .map_err(|e| RenderError::Storage(e.to_string()))?;
            if org_roots.is_empty() {
                None
            } else {
                Some(render_hierarchy(org_roots))
            }
        }
        Err(_) => None,
    };

    // §2.2-C reverse-render: query `user_roles`, resolve friendly
    // unit names from the modern hierarchy (`v_hierarchy`) first, and
    // fall back to legacy `organizational_units` while `user_roles`
    // still stores OU-backed ids. Tenant-wide grants (`unit_id ==
    // tenant_id`) surface as `unit: None` in the rendered manifest,
    // matching the forward-apply convention in `provision_tenant`
    // Step 7. Empty result → `roles: None` (elided from JSON) so
    // tenants with no assignments render byte-identically to a
    // manifest whose `roles:` was absent.
    //
    // Runs on the raw pool (admin-like). Render is called on both
    // admin provision paths and on the tenant-scoped
    // `GET /api/v1/admin/tenants/{slug}` path; in both cases the
    // caller has already authorized, so no RLS double-check is
    // needed here. If we ever expose render to a non-admin surface,
    // this query is the place to add `with_tenant_context`.
    let tenant_id_str = record.id.as_str();
    let role_rows_raw: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "WITH hierarchy_units AS (
             SELECT DISTINCT tenant_id::text AS tenant_id, org_id::text AS unit_id, org_name AS unit_name
               FROM v_hierarchy
              WHERE org_id IS NOT NULL
             UNION
             SELECT DISTINCT tenant_id::text AS tenant_id, team_id::text AS unit_id, team_name AS unit_name
               FROM v_hierarchy
              WHERE team_id IS NOT NULL
             UNION
             SELECT DISTINCT tenant_id::text AS tenant_id, project_id::text AS unit_id, project_name AS unit_name
               FROM v_hierarchy
              WHERE project_id IS NOT NULL
         )
         SELECT ur.user_id,
                ur.unit_id,
                ur.role,
                COALESCE(hu.unit_name, ou.name) AS unit_name
           FROM user_roles ur
      LEFT JOIN hierarchy_units hu
             ON hu.tenant_id = ur.tenant_id
            AND hu.unit_id = ur.unit_id
      LEFT JOIN organizational_units ou
             ON hu.unit_id IS NULL
            AND ou.id = ur.unit_id
            AND ou.tenant_id = ur.tenant_id
          WHERE ur.tenant_id = $1
       ORDER BY ur.user_id, ur.unit_id, ur.role",
    )
    .bind(tenant_id_str)
    .fetch_all(state.postgres.pool())
    .await
    .map_err(|e| RenderError::Storage(e.to_string()))?;

    let role_rows: Vec<RoleRow> = role_rows_raw
        .into_iter()
        .map(|(user_id, unit_id, role, unit_name)| RoleRow {
            user_id,
            unit_id,
            role,
            unit_name,
        })
        .collect();
    let rendered_roles = if role_rows.is_empty() {
        None
    } else {
        Some(render_roles(tenant_id_str, role_rows))
    };

    Ok(RenderedManifest {
        api_version: "aeterna.io/v1".to_string(),
        kind: "TenantManifest".to_string(),
        metadata: RenderedMetadata {
            generation,
            manifest_hash,
        },
        tenant: RenderedTenant {
            id: record.id.as_str().to_string(),
            slug: record.slug,
            name: record.name,
            status: record.status.to_string(),
            source_owner: record.source_owner.to_string(),
            account_ref: record.account.as_ref().map(|a| a.slug.clone()),
            environment: record.environment.clone(),
            // RenderedTenant uses i64 epoch seconds for manifest hash stability.
            // TenantRecord upgraded to DateTime<Utc> for wire-format correctness;
            // here we project back to i64 to keep manifest bytes deterministic.
            created_at: record.created_at.timestamp(),
            updated_at: record.updated_at.timestamp(),
            domain_mappings,
        },
        config: rendered_config,
        repository: rendered_repository,
        providers: rendered_providers,
        hierarchy,
        roles: rendered_roles,
        not_rendered: NOT_RENDERED_SECTIONS.to_vec(),
    })
}

/// Reverse of `apply_manifest_providers_to_config`. Reconstructs a
/// `RenderedProviders` block from `config.fields` +
/// `config.secret_references`.
///
/// Returns `None` when neither `llm_provider` nor `embedding_provider`
/// is present, so tenants that don't use the providers block produce
/// a manifest with no `providers:` key at all (byte-identical to the
/// input manifest on round-trip for that subset).
///
/// `secret_ref` resolution strategy: look for an entry in
/// `secret_references` whose `reference` equals the canonical
/// `{llm,embedding}_api_key` entry's reference but whose logical name
/// is NOT the canonical name. Deterministic because `BTreeMap` is
/// sorted; we take the first matching operator-named entry
/// alphabetically. Falls back to the canonical name itself when no
/// other entry matches (provider was configured via the dedicated
/// PUT handler which only writes the canonical key).
pub(crate) fn render_providers(
    fields: &BTreeMap<String, TenantConfigField>,
    secret_references: &BTreeMap<String, TenantSecretReference>,
    redact: bool,
) -> Option<RenderedProviders> {
    use memory::provider_registry::config_keys;

    let llm = render_one_provider(
        fields,
        secret_references,
        redact,
        config_keys::LLM_PROVIDER,
        config_keys::LLM_MODEL,
        config_keys::LLM_API_KEY,
        &[
            ("projectId", config_keys::LLM_GOOGLE_PROJECT_ID),
            ("location", config_keys::LLM_GOOGLE_LOCATION),
            ("region", config_keys::LLM_BEDROCK_REGION),
        ],
    );
    let embedding = render_one_provider(
        fields,
        secret_references,
        redact,
        config_keys::EMBEDDING_PROVIDER,
        config_keys::EMBEDDING_MODEL,
        config_keys::EMBEDDING_API_KEY,
        &[
            ("projectId", config_keys::EMBEDDING_GOOGLE_PROJECT_ID),
            ("location", config_keys::EMBEDDING_GOOGLE_LOCATION),
            ("region", config_keys::EMBEDDING_BEDROCK_REGION),
        ],
    );

    let memory_layers = render_memory_layers(fields, secret_references, redact);

    if llm.is_none() && embedding.is_none() && memory_layers.is_empty() {
        None
    } else {
        Some(RenderedProviders {
            llm,
            embedding,
            memory_layers,
        })
    }
}

/// B3 §2.2-D — reverse-recover `providers.memoryLayers` from the flat
/// config-field namespace.
///
/// Walks `fields` once looking for keys of the shape
/// `memory_layer.{LAYER}.provider`; each one defines a layer that is
/// then reconstructed through [`render_one_provider`] with dynamically
/// computed key names. Uses the same `.provider` sentinel as the
/// forward path — a layer missing the sentinel (e.g. someone wrote
/// `memory_layer.foo.model` without `memory_layer.foo.provider`) is
/// silently ignored, matching the forward-apply contract that `kind`
/// (stored as `{prefix}provider`) is mandatory.
///
/// The returned `BTreeMap` is sorted by layer name (inherited from
/// `BTreeMap`'s ordered iteration). Callers don't have to sort for
/// deterministic output.
fn render_memory_layers(
    fields: &BTreeMap<String, TenantConfigField>,
    secret_references: &BTreeMap<String, TenantSecretReference>,
    redact: bool,
) -> BTreeMap<String, RenderedProvider> {
    const PREFIX: &str = "memory_layer.";
    const PROVIDER_SUFFIX: &str = ".provider";

    let mut out = BTreeMap::new();
    for key in fields.keys() {
        // Extract `{LAYER}` from `memory_layer.{LAYER}.provider`. We
        // use `.provider` as the sentinel rather than any other
        // suffix because (a) it is the only *mandatory* field written
        // by `apply_one` so its presence uniquely defines a layer,
        // and (b) it avoids double-counting layers that happen to
        // have several optional fields set.
        let Some(rest) = key.strip_prefix(PREFIX) else {
            continue;
        };
        let Some(layer) = rest.strip_suffix(PROVIDER_SUFFIX) else {
            continue;
        };
        // Defence in depth: the validator forbids `.` in layer names
        // so the stripped middle must not contain a dot. Skip any
        // malformed legacy row rather than misgrouping fields.
        if layer.is_empty() || layer.contains('.') {
            continue;
        }

        let layer_prefix = format!("{PREFIX}{layer}");
        // Reuse the same apply_one contract: `{prefix}provider`,
        // `{prefix}model`, `{prefix}google_project_id`, …. The dot
        // separator lives in PREFIX; the inner suffixes still use
        // flat snake_case as written by the forward path.
        let provider_key = format!("{layer_prefix}.provider");
        let model_key = format!("{layer_prefix}.model");
        let api_key_key = format!("{layer_prefix}.api_key");
        let gproj_key = format!("{layer_prefix}.google_project_id");
        let gloc_key = format!("{layer_prefix}.google_location");
        let breg_key = format!("{layer_prefix}.bedrock_region");

        if let Some(rendered) = render_one_provider(
            fields,
            secret_references,
            redact,
            &provider_key,
            &model_key,
            &api_key_key,
            &[
                ("projectId", gproj_key.as_str()),
                ("location", gloc_key.as_str()),
                ("region", breg_key.as_str()),
            ],
        ) {
            out.insert(layer.to_string(), rendered);
        }
    }
    out
}

fn render_one_provider(
    fields: &BTreeMap<String, TenantConfigField>,
    secret_references: &BTreeMap<String, TenantSecretReference>,
    redact: bool,
    provider_key: &str,
    model_key: &str,
    api_key_key: &str,
    // (manifest-camelCase, config-snake_case) pairs for provider-specific
    // extras. We walk all of them and emit the ones whose config-side
    // key is present; it's cheap and avoids branching on kind.
    extras: &[(&str, &str)],
) -> Option<RenderedProvider> {
    let kind = field_as_string(fields, provider_key)?;
    let model = field_as_string(fields, model_key);

    let mut config = BTreeMap::new();
    for (manifest_name, storage_name) in extras {
        if let Some(v) = field_as_string(fields, storage_name) {
            config.insert((*manifest_name).to_string(), v);
        }
    }

    // secret_ref recovery: only in full mode. In redact mode we elide
    // the field entirely — the redacted renderer has already rewritten
    // operator names to `secret-N` placeholders, and surfacing one
    // here would leak nothing useful while implying a contract.
    let secret_ref = if redact {
        None
    } else {
        resolve_operator_secret_ref(secret_references, api_key_key)
    };

    Some(RenderedProvider {
        kind,
        model,
        secret_ref,
        config,
    })
}

/// Read a config field as a `String`. The apply helper writes
/// `serde_json::json!(provider.kind)` for a `String` which produces a
/// JSON string, so `as_str()` succeeds for valid apply output.
/// Returns `None` for missing fields and for fields whose value is
/// not a JSON string (defensive: a hand-edited DB row with a number
/// for `llm_model` won't crash the renderer).
fn field_as_string(fields: &BTreeMap<String, TenantConfigField>, key: &str) -> Option<String> {
    fields
        .get(key)
        .and_then(|f| f.value.as_str())
        .map(|s| s.to_string())
}

/// Find the operator-given name for a secret whose canonical alias is
/// `canonical_key` (e.g. `llm_api_key`). Walks the map once in sorted
/// order and returns the first non-canonical entry whose `reference`
/// matches the canonical entry's. Falls back to `canonical_key` itself
/// when the canonical entry is the only one pointing at its secret.
fn resolve_operator_secret_ref(
    secret_references: &BTreeMap<String, TenantSecretReference>,
    canonical_key: &str,
) -> Option<String> {
    let canonical = secret_references.get(canonical_key)?;
    // BTreeMap iteration is sorted by key; deterministic pick.
    for (name, sref) in secret_references.iter() {
        if name == canonical_key {
            continue;
        }
        if sref.reference == canonical.reference {
            return Some(name.clone());
        }
    }
    // Fall back: provider was set via the dedicated PUT handler, not
    // via manifest.providers — there's no operator name, so emit the
    // canonical name. Round-trip through apply will re-register the
    // canonical key as its own alias, which is a fixed point.
    Some(canonical_key.to_string())
}

pub(crate) fn render_config(
    fields: BTreeMap<String, TenantConfigField>,
    secret_references: BTreeMap<String, TenantSecretReference>,
    redact: bool,
) -> RenderedConfig {
    let secret_references = if redact {
        redact_secret_references(secret_references)
    } else {
        secret_references
            .into_iter()
            .map(|(key, sref)| {
                let reference_value =
                    serde_json::to_value(&sref.reference).unwrap_or_else(|_| json!(null));
                (
                    key,
                    RenderedSecretRef {
                        logical_name: sref.logical_name,
                        ownership: sref.ownership,
                        reference: reference_value,
                    },
                )
            })
            .collect()
    };

    RenderedConfig {
        fields,
        secret_references,
    }
}

/// Replace every secret reference with an opaque placeholder.
/// Indexing follows `BTreeMap` sorted-key iteration so the placeholder
/// assignment is deterministic across calls and pods.
fn redact_secret_references(
    input: BTreeMap<String, TenantSecretReference>,
) -> BTreeMap<String, RenderedSecretRef> {
    let mut out = BTreeMap::new();
    for (index, (_original_key, sref)) in input.into_iter().enumerate() {
        let placeholder = format!("secret-{index}");
        out.insert(
            placeholder.clone(),
            RenderedSecretRef {
                logical_name: placeholder,
                ownership: sref.ownership,
                reference: json!({"kind": "redacted"}),
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mk_core::secret::SecretReference;
    use uuid::Uuid;

    fn field(val: &str) -> TenantConfigField {
        TenantConfigField {
            ownership: TenantConfigOwnership::Tenant,
            value: serde_json::Value::String(val.to_string()),
        }
    }

    fn sref(logical_name: &str) -> TenantSecretReference {
        TenantSecretReference {
            logical_name: logical_name.to_string(),
            ownership: TenantConfigOwnership::Tenant,
            reference: SecretReference::Postgres {
                secret_id: Uuid::from_u128(0xDEAD_BEEF),
            },
        }
    }

    #[test]
    fn render_config_passes_fields_through_unchanged() {
        let mut fields = BTreeMap::new();
        fields.insert("region".to_string(), field("eu-west-1"));
        fields.insert("model".to_string(), field("claude-opus-4-7"));

        let out = render_config(fields.clone(), BTreeMap::new(), false);
        assert_eq!(out.fields, fields);
        assert!(out.secret_references.is_empty());
    }

    #[test]
    fn render_config_full_mode_preserves_secret_logical_names() {
        let mut refs = BTreeMap::new();
        refs.insert("openaiKey".to_string(), sref("openaiKey"));
        refs.insert("anthropicKey".to_string(), sref("anthropicKey"));

        let out = render_config(BTreeMap::new(), refs, false);
        assert_eq!(out.secret_references.len(), 2);
        assert_eq!(out.secret_references["openaiKey"].logical_name, "openaiKey");
        assert_eq!(
            out.secret_references["openaiKey"].reference["kind"],
            "postgres"
        );
    }

    #[test]
    fn render_config_redact_mode_replaces_names_deterministically() {
        let mut refs = BTreeMap::new();
        refs.insert("openaiKey".to_string(), sref("openaiKey"));
        refs.insert("anthropicKey".to_string(), sref("anthropicKey"));
        refs.insert("githubToken".to_string(), sref("githubToken"));

        let out = render_config(BTreeMap::new(), refs, true);
        let keys: Vec<&String> = out.secret_references.keys().collect();
        assert_eq!(keys, vec!["secret-0", "secret-1", "secret-2"]);
        for v in out.secret_references.values() {
            assert_eq!(v.reference, json!({"kind": "redacted"}));
            assert!(v.logical_name.starts_with("secret-"));
        }
    }

    #[test]
    fn redact_is_stable_across_calls() {
        let mut refs = BTreeMap::new();
        refs.insert("b".to_string(), sref("b"));
        refs.insert("a".to_string(), sref("a"));
        refs.insert("c".to_string(), sref("c"));

        let out1 = render_config(BTreeMap::new(), refs.clone(), true);
        let out2 = render_config(BTreeMap::new(), refs, true);
        assert_eq!(out1.secret_references, out2.secret_references);
    }

    #[test]
    fn rendered_manifest_serializes_as_camel_case() {
        let m = RenderedManifest {
            api_version: "aeterna.io/v1".into(),
            kind: "TenantManifest".into(),
            metadata: RenderedMetadata {
                generation: 3,
                manifest_hash: Some("sha256:abc".into()),
            },
            tenant: RenderedTenant {
                id: "id".into(),
                slug: "acme".into(),
                name: "Acme".into(),
                status: "active".into(),
                source_owner: "admin".into(),
                account_ref: None,
                environment: None,
                created_at: 1,
                updated_at: 2,
                domain_mappings: None,
            },
            config: None,
            repository: None,
            providers: None,
            hierarchy: None,
            roles: None,
            not_rendered: NOT_RENDERED_SECTIONS.to_vec(),
        };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["apiVersion"], "aeterna.io/v1");
        assert_eq!(v["kind"], "TenantManifest");
        assert_eq!(v["metadata"]["generation"], 3);
        assert_eq!(v["metadata"]["manifestHash"], "sha256:abc");
        assert_eq!(v["tenant"]["sourceOwner"], "admin");
        assert!(v["notRendered"].is_array());
        // After §2.2-A+D: neither `providers` nor
        // `providers.memoryLayers` should be in the gap list.
        let not_rendered = v["notRendered"].as_array().unwrap();
        assert!(
            !not_rendered.contains(&json!("providers")),
            "providers should no longer be a top-level gap — \
             reverse-render landed in §2.2-A"
        );
        assert!(
            !not_rendered.contains(&json!("providers.memoryLayers")),
            "memoryLayers storage convention landed in §2.2-D"
        );
    }

    #[test]
    fn rendered_manifest_omits_absent_optional_sections() {
        let m = RenderedManifest {
            api_version: "aeterna.io/v1".into(),
            kind: "TenantManifest".into(),
            metadata: RenderedMetadata {
                generation: 0,
                manifest_hash: None,
            },
            tenant: RenderedTenant {
                id: "id".into(),
                slug: "acme".into(),
                name: "Acme".into(),
                status: "active".into(),
                source_owner: "admin".into(),
                account_ref: None,
                environment: None,
                created_at: 1,
                updated_at: 2,
                domain_mappings: None,
            },
            config: None,
            repository: None,
            providers: None,
            hierarchy: None,
            roles: None,
            not_rendered: vec![],
        };
        let v = serde_json::to_value(&m).unwrap();
        assert!(v.get("config").is_none());
        assert!(v.get("repository").is_none());
        assert!(
            v.get("providers").is_none(),
            "absent providers must be elided, not null-serialized"
        );
        assert!(v["metadata"].get("manifestHash").is_none());
        // domainMappings = None ⇒ absent on the wire (matches input shape
        // where `tenant.domainMappings` is `Option<Vec<_>>`).
        assert!(
            v["tenant"].get("domainMappings").is_none(),
            "tenant.domainMappings must be omitted when None, got: {}",
            v["tenant"]
        );
    }

    #[test]
    fn rendered_tenant_emits_sorted_domain_mappings_when_present() {
        // The store contract guarantees sorted output; the renderer does
        // not re-sort. This test locks the round-trip byte shape so a
        // future well-meaning refactor that wraps the result in a
        // HashSet / BTreeMap iteration doesn't silently change ordering.
        let m = RenderedManifest {
            api_version: "aeterna.io/v1".into(),
            kind: "TenantManifest".into(),
            metadata: RenderedMetadata {
                generation: 1,
                manifest_hash: None,
            },
            tenant: RenderedTenant {
                id: "id".into(),
                slug: "acme".into(),
                name: "Acme".into(),
                status: "active".into(),
                source_owner: "admin".into(),
                account_ref: None,
                environment: None,
                created_at: 1,
                updated_at: 2,
                domain_mappings: Some(vec![
                    "acme.com".into(),
                    "beta.acme.com".into(),
                    "zulu.acme.com".into(),
                ]),
            },
            config: None,
            repository: None,
            providers: None,
            hierarchy: None,
            roles: None,
            not_rendered: vec![],
        };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(
            v["tenant"]["domainMappings"],
            json!(["acme.com", "beta.acme.com", "zulu.acme.com"])
        );
    }

    #[test]
    fn not_rendered_list_contains_every_expected_section() {
        // Post §2.2-B (hierarchy, PR #143/#148) + §2.2-C (roles,
        // PR #145) + §2.2-D (providers.memoryLayers, this PR), every
        // manifest section round-trips and `NOT_RENDERED_SECTIONS`
        // is empty. This test is kept as a regression guard so that
        // any future PR reintroducing a gap has to update the
        // assertions deliberately (and trip the §2.4 diff contract).
        assert!(
            NOT_RENDERED_SECTIONS.is_empty(),
            "NOT_RENDERED_SECTIONS must be empty after §2.2-B/C/D; found {:?}",
            NOT_RENDERED_SECTIONS
        );
        // Explicit regression guards for each closed gap — these
        // must NEVER reappear in NOT_RENDERED.
        assert!(
            !NOT_RENDERED_SECTIONS.contains(&"hierarchy"),
            "hierarchy must not be a gap after §2.2-B"
        );
        assert!(
            !NOT_RENDERED_SECTIONS.contains(&"roles"),
            "roles must not be a gap after §2.2-C"
        );
        assert!(
            !NOT_RENDERED_SECTIONS.contains(&"providers.memoryLayers"),
            "providers.memoryLayers must not be a gap after §2.2-D"
        );
        for section in RENDERED_SECTIONS {
            assert!(
                !NOT_RENDERED_SECTIONS.contains(section),
                "{section} is in both RENDERED and NOT_RENDERED"
            );
        }
        // Regression: the bare string "providers" must NOT appear in
        // NOT_RENDERED after §2.2-A. The sub-section form
        // `providers.memoryLayers` is the honest shape going forward.
        assert!(
            !NOT_RENDERED_SECTIONS.contains(&"providers"),
            "top-level `providers` must no longer be in the gap list"
        );
        // PR #128 regression: `secrets` (top-level input-only ingestion
        // form) and `domainMappings` (now rendered under
        // `tenant.domainMappings`) MUST NOT reappear in NOT_RENDERED.
        // If they do, a future PR accidentally re-introduced them as
        // gaps; this test catches it.
        for gone in ["secrets", "domainMappings"] {
            assert!(
                !NOT_RENDERED_SECTIONS.contains(&gone),
                "{gone} must not be in NOT_RENDERED_SECTIONS: secrets is \
                 wire-only input; domainMappings is rendered under tenant."
            );
        }
        // §2.2-B regression: `hierarchy` must NOT be in NOT_RENDERED
        // going forward. `render_hierarchy` owns it via
        // `HierarchyStore::get_hierarchy`.
        assert!(
            !NOT_RENDERED_SECTIONS.contains(&"hierarchy"),
            "hierarchy is now rendered by render_hierarchy (§2.2-B)"
        );
    }

    // ── §2.2-B render_hierarchy unit tests ───────────────────────────────

    #[test]
    fn render_hierarchy_empty_input_produces_empty_vec() {
        let out = render_hierarchy(vec![]);
        assert!(out.is_empty());
    }

    #[test]
    fn render_hierarchy_preserves_slug_name_and_nesting() {
        use storage::hierarchy_store::{Org, Team};
        let input = vec![Org {
            id: Uuid::from_u128(2),
            tenant_id: Uuid::from_u128(100),
            slug: "platform".into(),
            name: "Platform".into(),
            teams: vec![Team {
                id: Uuid::from_u128(3),
                org_id: Uuid::from_u128(2),
                slug: "sre".into(),
                name: "SRE".into(),
            }],
        }];
        let out = render_hierarchy(input);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].slug, "platform");
        assert_eq!(out[0].name, "Platform");
        assert_eq!(out[0].teams.len(), 1);
        assert_eq!(out[0].teams[0].slug, "sre");
        assert_eq!(out[0].teams[0].name, "SRE");
    }

    #[test]
    fn render_hierarchy_serializes_without_uuids() {
        // Critical invariant: reverse-render MUST NOT leak internal
        // UUIDs to the manifest surface. Operators identify orgs
        // by slug + name only.
        use storage::hierarchy_store::Org;
        let input = vec![Org {
            id: Uuid::from_u128(42),
            tenant_id: Uuid::from_u128(7),
            slug: "x".into(),
            name: "X".into(),
            teams: vec![],
        }];
        let out = render_hierarchy(input);
        let v = serde_json::to_value(&out).unwrap();
        let s = v.to_string();
        assert!(!s.contains("00000000"), "no UUIDs in rendered output: {s}");
        assert!(!s.contains("\"id\""), "no `id` key in rendered output: {s}");
        assert!(
            !s.contains("tenantId"),
            "no `tenantId` key in rendered output: {s}"
        );
    }

    #[test]
    fn render_hierarchy_empty_orgs_elides_field_in_json() {
        // `teams` skipped when empty so the wire shape matches a
        // manifest input that omitted `teams:`.
        use storage::hierarchy_store::Org;
        let input = vec![Org {
            id: Uuid::from_u128(1),
            tenant_id: Uuid::from_u128(100),
            slug: "solo".into(),
            name: "Solo".into(),
            teams: vec![],
        }];
        let out = render_hierarchy(input);
        let v = serde_json::to_value(&out).unwrap();
        assert!(v[0].get("teams").is_none(), "empty teams must be elided");
    }

    // ── §2.2-C render_roles unit tests ───────────────────────────────────

    fn role_row(user: &str, unit_id: &str, role: &str, unit_name: Option<&str>) -> RoleRow {
        RoleRow {
            user_id: user.into(),
            unit_id: unit_id.into(),
            role: role.into(),
            unit_name: unit_name.map(|s| s.into()),
        }
    }

    #[test]
    fn render_roles_empty_input_produces_empty_vec() {
        let out = render_roles("t-1", vec![]);
        assert!(out.is_empty());
    }

    #[test]
    fn render_roles_tenant_wide_grant_maps_to_unit_none() {
        // Forward-apply convention: when the manifest has no `unit`,
        // the row is stored with `unit_id = tenant_id`. Reverse-render
        // must detect that sentinel and emit `unit: None` so the
        // rendered manifest is byte-identical to the input.
        let tenant = "t-42";
        let rows = vec![role_row("u-1", tenant, "admin", None)];
        let out = render_roles(tenant, rows);
        assert_eq!(out.len(), 1);
        assert!(
            out[0].unit.is_none(),
            "tenant-wide grant must render with unit=None"
        );
        assert_eq!(out[0].user_id, "u-1");
    }

    #[test]
    fn render_roles_scoped_grant_emits_friendly_unit_name() {
        // When the grant scopes to a real OU and the LEFT JOIN
        // resolved a name, render emits the *name* (operator-facing
        // identifier) — not the internal OU id.
        let tenant = "t-1";
        let rows = vec![role_row(
            "u-1",
            "ou-platform-uuid",
            "member",
            Some("Platform"),
        )];
        let out = render_roles(tenant, rows);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].unit.as_deref(), Some("Platform"));
    }

    #[test]
    fn render_roles_orphaned_grant_falls_back_to_unit_id() {
        // OU was deleted, but the `user_roles` row survived. We emit
        // the raw unit_id rather than silently dropping the grant —
        // the rendered manifest stays losslessly round-trippable and
        // surfaces the integrity drift to the operator.
        let tenant = "t-1";
        let rows = vec![role_row("u-1", "ou-ghost-uuid", "admin", None)];
        let out = render_roles(tenant, rows);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].unit.as_deref(),
            Some("ou-ghost-uuid"),
            "orphaned grant must fall back to raw unit_id, not None"
        );
    }

    #[test]
    fn render_roles_preserves_order_and_handles_mixed_grants() {
        // Query layer ORDER BYs user_id, unit_id, role. We assert the
        // transform preserves that ordering end-to-end across a mix
        // of tenant-wide, scoped-with-name, and orphaned grants.
        let tenant = "t-42";
        let rows = vec![
            role_row("u-1", "ou-a", "admin", Some("Alpha")),
            role_row("u-1", tenant, "member", None),
            role_row("u-2", "ou-ghost", "admin", None),
        ];
        let out = render_roles(tenant, rows);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].user_id, "u-1");
        assert_eq!(out[0].unit.as_deref(), Some("Alpha"));
        assert_eq!(out[1].user_id, "u-1");
        assert!(out[1].unit.is_none(), "tenant-wide sentinel");
        assert_eq!(out[2].user_id, "u-2");
        assert_eq!(out[2].unit.as_deref(), Some("ou-ghost"));
    }

    #[test]
    fn render_roles_custom_role_string_preserved() {
        // Unknown role strings flow through `RoleIdentifier::from_str_flexible`
        // as `Custom(..)` — the renderer must not reject them.
        let tenant = "t-1";
        let rows = vec![role_row("u-1", tenant, "custom_role_foo_bar", None)];
        let out = render_roles(tenant, rows);
        assert_eq!(out.len(), 1);
        // Round-trip through serde to verify the `untagged` enum emits
        // the raw string, i.e. the wire shape matches the input form.
        let v = serde_json::to_value(&out[0].role).unwrap();
        assert_eq!(v, serde_json::json!("custom_role_foo_bar"));
    }

    #[test]
    fn render_roles_elides_empty_unit_in_json() {
        let tenant = "t-1";
        let rows = vec![role_row("u-1", tenant, "admin", None)];
        let out = render_roles(tenant, rows);
        let v = serde_json::to_value(&out[0]).unwrap();
        assert!(v.get("unit").is_none(), "unit=None must be elided, got {v}");
    }

    // ── §2.2-A render_providers unit tests ───────────────────────────────

    use memory::provider_registry::config_keys;

    fn platform_field(val: &str) -> TenantConfigField {
        TenantConfigField {
            ownership: TenantConfigOwnership::Platform,
            value: serde_json::Value::String(val.to_string()),
        }
    }

    fn postgres_ref(logical_name: &str, secret_id: u128) -> TenantSecretReference {
        TenantSecretReference {
            logical_name: logical_name.to_string(),
            ownership: TenantConfigOwnership::Tenant,
            reference: SecretReference::Postgres {
                secret_id: Uuid::from_u128(secret_id),
            },
        }
    }

    #[test]
    fn render_providers_none_when_no_provider_fields() {
        // A tenant with config fields that have nothing to do with
        // providers must produce no `providers:` block at all.
        let mut fields = BTreeMap::new();
        fields.insert("some_app_setting".to_string(), platform_field("v"));
        let out = render_providers(&fields, &BTreeMap::new(), false);
        assert!(out.is_none());
    }

    #[test]
    fn render_providers_reconstructs_llm_openai() {
        let mut fields = BTreeMap::new();
        fields.insert(
            config_keys::LLM_PROVIDER.to_string(),
            platform_field("openai"),
        );
        fields.insert(config_keys::LLM_MODEL.to_string(), platform_field("gpt-4o"));
        let out = render_providers(&fields, &BTreeMap::new(), false).expect("llm should render");
        let llm = out.llm.expect("llm branch");
        assert_eq!(llm.kind, "openai");
        assert_eq!(llm.model.as_deref(), Some("gpt-4o"));
        assert!(llm.secret_ref.is_none(), "no api_key alias set");
        assert!(llm.config.is_empty());
        assert!(out.embedding.is_none());
    }

    #[test]
    fn render_providers_reconstructs_google_extras() {
        let mut fields = BTreeMap::new();
        fields.insert(
            config_keys::LLM_PROVIDER.to_string(),
            platform_field("google"),
        );
        fields.insert(
            config_keys::LLM_MODEL.to_string(),
            platform_field("gemini-1.5-pro"),
        );
        fields.insert(
            config_keys::LLM_GOOGLE_PROJECT_ID.to_string(),
            platform_field("my-proj"),
        );
        fields.insert(
            config_keys::LLM_GOOGLE_LOCATION.to_string(),
            platform_field("europe-west1"),
        );
        let out = render_providers(&fields, &BTreeMap::new(), false).expect("llm should render");
        let llm = out.llm.unwrap();
        assert_eq!(llm.kind, "google");
        assert_eq!(
            llm.config.get("projectId").map(String::as_str),
            Some("my-proj")
        );
        assert_eq!(
            llm.config.get("location").map(String::as_str),
            Some("europe-west1")
        );
        assert!(
            !llm.config.contains_key("region"),
            "region is bedrock-only; google provider must not carry it"
        );
    }

    #[test]
    fn render_providers_prefers_operator_secret_name_over_canonical() {
        // When both the operator name AND the canonical alias point
        // at the same underlying secret_id (the shape
        // `apply_manifest_providers_to_config` produces), the
        // rendered `secret_ref` must be the operator's name so
        // round-trip through apply is byte-identical.
        let same_id = 0xC0FFEE;
        let mut fields = BTreeMap::new();
        fields.insert(
            config_keys::LLM_PROVIDER.to_string(),
            platform_field("openai"),
        );
        let mut refs = BTreeMap::new();
        refs.insert(
            "openai_key".to_string(),
            postgres_ref("openai_key", same_id),
        );
        refs.insert(
            config_keys::LLM_API_KEY.to_string(),
            postgres_ref(config_keys::LLM_API_KEY, same_id),
        );
        let out = render_providers(&fields, &refs, false).unwrap();
        assert_eq!(out.llm.unwrap().secret_ref.as_deref(), Some("openai_key"));
    }

    #[test]
    fn render_providers_falls_back_to_canonical_when_alone() {
        // Provider was configured via `PUT .../providers/llm` only:
        // `llm_api_key` exists but no operator-named alias does.
        // Render the canonical name — re-applying is a fixed point
        // because apply will re-register llm_api_key → llm_api_key.
        let mut fields = BTreeMap::new();
        fields.insert(
            config_keys::LLM_PROVIDER.to_string(),
            platform_field("openai"),
        );
        let mut refs = BTreeMap::new();
        refs.insert(
            config_keys::LLM_API_KEY.to_string(),
            postgres_ref(config_keys::LLM_API_KEY, 0xFEED),
        );
        let out = render_providers(&fields, &refs, false).unwrap();
        assert_eq!(
            out.llm.unwrap().secret_ref.as_deref(),
            Some(config_keys::LLM_API_KEY)
        );
    }

    #[test]
    fn render_providers_elides_secret_ref_in_redact_mode() {
        let same_id = 0xC0FFEE;
        let mut fields = BTreeMap::new();
        fields.insert(
            config_keys::LLM_PROVIDER.to_string(),
            platform_field("openai"),
        );
        let mut refs = BTreeMap::new();
        refs.insert(
            "openai_key".to_string(),
            postgres_ref("openai_key", same_id),
        );
        refs.insert(
            config_keys::LLM_API_KEY.to_string(),
            postgres_ref(config_keys::LLM_API_KEY, same_id),
        );
        let out = render_providers(&fields, &refs, true).unwrap();
        assert!(
            out.llm.unwrap().secret_ref.is_none(),
            "redact mode must not surface operator-named secret refs"
        );
    }

    #[test]
    fn render_providers_handles_embedding_only() {
        // Edge case: tenant configured only an embedding provider,
        // no LLM. The renderer must emit a providers block with
        // llm=None and embedding=Some.
        let mut fields = BTreeMap::new();
        fields.insert(
            config_keys::EMBEDDING_PROVIDER.to_string(),
            platform_field("bedrock"),
        );
        fields.insert(
            config_keys::EMBEDDING_BEDROCK_REGION.to_string(),
            platform_field("us-east-1"),
        );
        let out = render_providers(&fields, &BTreeMap::new(), false).unwrap();
        assert!(out.llm.is_none());
        let emb = out.embedding.unwrap();
        assert_eq!(emb.kind, "bedrock");
        assert_eq!(
            emb.config.get("region").map(String::as_str),
            Some("us-east-1")
        );
    }

    #[test]
    fn render_providers_defensive_ignores_non_string_field() {
        // Hand-edited DB row with a JSON number for `llm_model`
        // must not crash the renderer — field_as_string returns None,
        // model is emitted as absent, kind still surfaces.
        let mut fields = BTreeMap::new();
        fields.insert(
            config_keys::LLM_PROVIDER.to_string(),
            platform_field("openai"),
        );
        fields.insert(
            config_keys::LLM_MODEL.to_string(),
            TenantConfigField {
                ownership: TenantConfigOwnership::Platform,
                value: json!(42), // wrong shape on purpose
            },
        );
        let out = render_providers(&fields, &BTreeMap::new(), false).unwrap();
        let llm = out.llm.unwrap();
        assert_eq!(llm.kind, "openai");
        assert!(
            llm.model.is_none(),
            "non-string model value must not be silently serialized as a number"
        );
    }

    // ── B3 §2.2-D — memoryLayers reverse render ───────────────────────────

    #[test]
    fn render_memory_layers_none_when_no_layer_fields() {
        // Tenants that never declared a layer must produce an empty
        // BTreeMap — `skip_serializing_if = "BTreeMap::is_empty"` on
        // the field makes that elide from JSON entirely, preserving
        // byte-identity with the input manifest shape.
        let layers = render_memory_layers(&BTreeMap::new(), &BTreeMap::new(), false);
        assert!(layers.is_empty());
    }

    #[test]
    fn render_memory_layers_reconstructs_single_layer() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "memory_layer.episodic.provider".to_string(),
            platform_field("qdrant"),
        );
        fields.insert(
            "memory_layer.episodic.model".to_string(),
            platform_field("text-embedding-3-small"),
        );
        let layers = render_memory_layers(&fields, &BTreeMap::new(), false);
        assert_eq!(layers.len(), 1);
        let epi = layers.get("episodic").unwrap();
        assert_eq!(epi.kind, "qdrant");
        assert_eq!(epi.model.as_deref(), Some("text-embedding-3-small"));
    }

    #[test]
    fn render_memory_layers_groups_multiple_layers_sorted() {
        // BTreeMap ordering guarantees alphabetical output, which
        // matters for deterministic manifest hashing.
        let mut fields = BTreeMap::new();
        for layer in ["semantic", "episodic", "procedural"] {
            fields.insert(
                format!("memory_layer.{layer}.provider"),
                platform_field("qdrant"),
            );
        }
        let layers = render_memory_layers(&fields, &BTreeMap::new(), false);
        let keys: Vec<&String> = layers.keys().collect();
        assert_eq!(keys, vec!["episodic", "procedural", "semantic"]);
    }

    #[test]
    fn render_memory_layers_skips_orphaned_model_without_provider() {
        // Apply-side contract: a layer IS the `.provider` sentinel.
        // Legacy / hand-edited rows that have `.model` without
        // `.provider` must not materialise a bogus half-populated
        // layer.
        let mut fields = BTreeMap::new();
        fields.insert(
            "memory_layer.ghost.model".to_string(),
            platform_field("text-embedding-3-small"),
        );
        let layers = render_memory_layers(&fields, &BTreeMap::new(), false);
        assert!(layers.is_empty());
    }

    #[test]
    fn render_memory_layers_ignores_malformed_dotted_middle() {
        // Defence-in-depth against the validator being bypassed:
        // a key like `memory_layer.bad.name.provider` could either
        // be layer `bad.name` (illegal) or layer `bad` with
        // suffix `name.provider` (also illegal). The renderer
        // skips both rather than misgrouping — the validator is
        // the authoritative gate, this is just the fallback.
        let mut fields = BTreeMap::new();
        fields.insert(
            "memory_layer.bad.name.provider".to_string(),
            platform_field("qdrant"),
        );
        let layers = render_memory_layers(&fields, &BTreeMap::new(), false);
        assert!(layers.is_empty());
    }

    #[test]
    fn render_memory_layers_recovers_operator_secret_ref() {
        // The canonical api_key alias for a layer is
        // `memory_layer.{layer}.api_key`. Reverse recovery should
        // return the operator-friendly name
        // (`episodic_key` in this fixture) by reference-equality on
        // the underlying SecretReference, same as llm/embedding.
        let env_ref = TenantSecretReference {
            logical_name: "episodic_key".to_string(),
            ownership: TenantConfigOwnership::Platform,
            reference: mk_core::secret::SecretReference::Env {
                var: "AETERNA_EPISODIC".to_string(),
            },
        };
        let canonical_ref = TenantSecretReference {
            logical_name: "memory_layer.episodic.api_key".to_string(),
            ownership: TenantConfigOwnership::Platform,
            reference: env_ref.reference.clone(),
        };
        let mut secret_references = BTreeMap::new();
        secret_references.insert("episodic_key".to_string(), env_ref);
        secret_references.insert("memory_layer.episodic.api_key".to_string(), canonical_ref);

        let mut fields = BTreeMap::new();
        fields.insert(
            "memory_layer.episodic.provider".to_string(),
            platform_field("bedrock"),
        );

        let layers = render_memory_layers(&fields, &secret_references, false);
        let epi = layers.get("episodic").unwrap();
        assert_eq!(epi.secret_ref.as_deref(), Some("episodic_key"));
    }

    #[test]
    fn render_memory_layers_elides_secret_ref_in_redact_mode() {
        let env_ref = TenantSecretReference {
            logical_name: "episodic_key".to_string(),
            ownership: TenantConfigOwnership::Platform,
            reference: mk_core::secret::SecretReference::Env {
                var: "AETERNA_EPISODIC".to_string(),
            },
        };
        let mut secret_references = BTreeMap::new();
        secret_references.insert("memory_layer.episodic.api_key".to_string(), env_ref.clone());
        secret_references.insert("episodic_key".to_string(), env_ref);

        let mut fields = BTreeMap::new();
        fields.insert(
            "memory_layer.episodic.provider".to_string(),
            platform_field("bedrock"),
        );

        let layers = render_memory_layers(&fields, &secret_references, true);
        assert!(layers.get("episodic").unwrap().secret_ref.is_none());
    }

    #[test]
    fn render_providers_emits_memory_layers_alongside_llm() {
        // End-to-end: llm + memoryLayers coexist, both surface in
        // the single `RenderedProviders` block, `memory_layers` map
        // stays separate from the two optional singletons.
        let mut fields = BTreeMap::new();
        fields.insert(
            config_keys::LLM_PROVIDER.to_string(),
            platform_field("openai"),
        );
        fields.insert(
            "memory_layer.episodic.provider".to_string(),
            platform_field("qdrant"),
        );
        let out = render_providers(&fields, &BTreeMap::new(), false).unwrap();
        assert!(out.llm.is_some());
        assert!(out.embedding.is_none());
        assert_eq!(out.memory_layers.len(), 1);
        assert_eq!(out.memory_layers.get("episodic").unwrap().kind, "qdrant");
    }
}
