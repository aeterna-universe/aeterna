## ADDED Requirements

### Requirement: Manifest Generation and Hashing
The system SHALL compute a canonical SHA-256 hash (`manifest_hash`) over every applied `TenantManifest`, persist it on the tenant record, and echo it in the provisioning response. The manifest SHALL carry a monotonic `metadata.generation` integer that the server uses for optimistic concurrency control.

#### Scenario: Apply echoes and persists manifest_hash
- **WHEN** a caller POSTs a valid `TenantManifest` to `/api/v1/admin/tenants/provision`
- **THEN** the server SHALL canonicalize the manifest (YAML canonical form, inline secret values redacted)
- **AND** compute `manifest_hash = sha256(canonical_bytes)`
- **AND** store it as `last_applied_manifest_hash` on the tenant record
- **AND** return `manifestHash` and the applied `generation` in the response

#### Scenario: Re-apply of unchanged manifest is a no-op
- **WHEN** a manifest whose `manifest_hash` matches the tenant's `last_applied_manifest_hash` is submitted
- **AND** the `generation` is greater than or equal to the stored generation
- **THEN** the server SHALL return `overallOk: true` with zero executed mutation steps
- **AND** SHALL NOT emit a `TenantProvisioned` governance event
- **AND** SHALL record an audit entry with `dry_run: false` and `steps: []`

#### Scenario: Apply with stale generation is rejected
- **WHEN** a manifest with `metadata.generation` less than the tenant's stored generation is submitted
- **THEN** the server SHALL reject the request with HTTP 409
- **AND** the response body SHALL include the current server-side generation so the caller can re-render and retry

### Requirement: Dry-Run and Diff
The system SHALL expose a non-mutating preview mode for any manifest, and a diff endpoint that computes the set of mutations a given manifest would produce against the current tenant state.

#### Scenario: Dry-run does not mutate state
- **WHEN** a caller POSTs a manifest to `/api/v1/admin/tenants/provision?dryRun=true`
- **THEN** the server SHALL run every validation and planning step
- **AND** SHALL return the planned per-step status as if applied
- **AND** SHALL NOT create or modify any tenant, config, secret, hierarchy, or role record
- **AND** SHALL NOT emit any governance event
- **AND** the audit log entry SHALL record `dry_run: true`

#### Scenario: Diff returns planned delta against current state
- **WHEN** a caller POSTs a manifest to `/api/v1/admin/tenants/diff`
- **THEN** the server SHALL compare the submitted manifest to the current rendered state of the tenant identified by `tenant.slug`
- **AND** SHALL return a structured delta describing additions, removals, and field-level changes
- **AND** if the tenant does not exist the delta SHALL describe full creation

### Requirement: Manifest Render (Export)
The system SHALL expose an endpoint that returns the current state of a tenant as a `TenantManifest`, suitable for backup, version control, and round-trip re-application.

#### Scenario: Render returns a replayable manifest
- **WHEN** a caller GETs `/api/v1/admin/tenants/{slug}/manifest`
- **AND** the caller has `tenant:read` scope or PlatformAdmin role
- **THEN** the server SHALL return a `TenantManifest` reflecting the tenant's current persisted state
- **AND** the manifest SHALL include `metadata.generation` equal to the current generation
- **AND** secret values SHALL NOT appear; only `secretReferences` SHALL be returned

#### Scenario: Redacted render for restricted callers
- **WHEN** the caller has only `tenant:read` scope
- **AND** requests the manifest with `?redact=true`
- **THEN** secret reference names SHALL be returned as opaque placeholders (`"***redacted-ref***"`)
- **AND** config field values marked sensitive in schema SHALL be replaced with `"***"`

### Requirement: Secret Reference Types
The system SHALL accept typed `SecretReference` entries in `config.secretReferences` of the form `K8sSecretRef`, `FileRef`, `EnvRef`, or `VaultRef`, and SHALL resolve them to values at runtime rather than at apply time.

#### Scenario: K8sSecretRef resolution at runtime
- **WHEN** a tenant config references `kind: K8sSecretRef` with `name: tenant-acme-llm`
- **AND** a runtime component calls `secrets.get("llmCredentials")`
- **THEN** the server SHALL read the named Kubernetes Secret via the pod's service-account credentials
- **AND** return the value for the specified `key`
- **AND** SHALL NOT cache plaintext longer than the request scope

#### Scenario: FileRef rejects insecure file modes
- **WHEN** a `FileRef` points to a file whose mode is looser than `0600`
- **THEN** the server SHALL refuse to resolve the reference
- **AND** the tenant's runtime status SHALL show `loading_failed` with reason `"insecure file mode"`

#### Scenario: Unknown reference kind is a validation error
- **WHEN** a manifest includes a reference with an unknown `kind`
- **THEN** manifest validation SHALL fail before any step executes
- **AND** the response SHALL list the unknown kind and the offending path

### Requirement: Inline Secret Values Gated
The system SHALL accept inline `secrets[].secretValue` entries only when the server was started with an explicit opt-in flag AND the caller explicitly requests inline mode AND the build is not a release build. All three conditions must hold.

#### Scenario: Inline secret rejected in release build
- **WHEN** a manifest includes a `secrets[]` entry with a non-empty `secretValue`
- **AND** the server is a release build
- **THEN** manifest validation SHALL fail
- **AND** the response SHALL direct the caller to use `config.secretReferences`

#### Scenario: Inline secret accepted in dev with all opt-ins
- **WHEN** the server was started with `--allow-inline-secret`
- **AND** the caller passes `?allowInline=true`
- **AND** the build is a dev build
- **THEN** the inline value SHALL be stored via the tenant secrets provider
- **AND** the inline value SHALL NOT appear in any audit log, response body, or render output

### Requirement: Audit Parity Across Surfaces
Every mutation through the provisioning handler SHALL record a `via` discriminator identifying the calling surface (`cli`, `ui`, `api`), the `client_version`, the `manifest_hash`, the `generation`, and a `dry_run` boolean. The discriminator SHALL be derived from a request header `X-Aeterna-Client-Kind`.

#### Scenario: CLI mutation records via=cli
- **WHEN** `aeterna tenant apply` POSTs a manifest
- **AND** the request carries `X-Aeterna-Client-Kind: cli`
- **THEN** the audit entry SHALL record `via: "cli"` and the CLI's user-agent in `client_version`

#### Scenario: Missing header defaults to api
- **WHEN** a request omits `X-Aeterna-Client-Kind`
- **THEN** the audit entry SHALL record `via: "api"`

### Requirement: First-Run Bootstrap Reports Status
The first-run bootstrap SHALL be idempotent, produce a machine-readable status report for each step, and SHALL NOT allow the pod to report ready unless every required bootstrap step succeeded.

#### Scenario: Bootstrap success makes pod ready
- **WHEN** bootstrap runs at pod start with valid environment inputs
- **AND** creates the PlatformAdmin identity, the bootstrap tenant / org / team, and initial tenant record
- **THEN** `/ready` SHALL return 200 once all steps report success
- **AND** `/api/v1/admin/bootstrap/status` SHALL return `{ok: true, steps: [...]}`

#### Scenario: Bootstrap failure keeps pod not-ready
- **WHEN** any bootstrap step fails (e.g., cannot resolve PlatformAdmin identity provider)
- **THEN** `/ready` SHALL continue to return 503 with reason `bootstrap_incomplete`
- **AND** the pod SHALL NOT be marked ready by Kubernetes
- **AND** subsequent pod restarts SHALL re-attempt bootstrap idempotently

## MODIFIED Requirements

### Requirement: Idempotent Tenant Provisioning
The system SHALL support idempotent re-application of a tenant manifest. Re-application of a manifest whose canonical hash matches the last applied hash SHALL be a structured no-op; re-application of a modified manifest SHALL produce exactly the delta required to converge state.

#### Scenario: Re-provision existing tenant updates config
- **WHEN** a manifest with a new hash is submitted for an existing tenant slug
- **THEN** the system SHALL update configuration fields, secret references, and role assignments to match the manifest
- **AND** the system SHALL NOT create duplicate hierarchy units
- **AND** the response SHALL include the new `manifestHash` and incremented `generation`

#### Scenario: Provisioning step failure reports partial status
- **WHEN** a provisioning step fails
- **THEN** the response SHALL indicate which steps succeeded and which failed
- **AND** successfully completed steps SHALL NOT be rolled back
- **AND** the operator SHALL be able to fix the underlying cause and re-submit the manifest
- **AND** the audit log SHALL record the partial-success with `overall_ok: false`
