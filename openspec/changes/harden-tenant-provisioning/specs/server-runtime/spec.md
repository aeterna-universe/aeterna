## ADDED Requirements

### Requirement: Per-Tenant Provider Wiring Invariant
At server startup, for every tenant record loaded from persistent storage, the runtime SHALL resolve all declared providers (memory layers, LLM, embedding, repository binding) before marking the tenant available to serve requests. A tenant whose provider wiring fails SHALL be placed in `loading_failed` state with a structured reason, and its requests SHALL return HTTP 503 with body `{error: "tenant_unavailable", tenantSlug, reason}` rather than HTTP 500 from downstream code paths.

#### Scenario: Missing memory provider blocks the tenant
- **WHEN** a tenant manifest declares `providers.memoryLayers` for layers `[Tenant, Org, Team, User, Session, Project, Agent]`
- **AND** at startup the runtime cannot register a provider for one of those layers
- **THEN** the tenant SHALL be marked `loading_failed` with `reason: "memory_provider_missing"` and the missing layer name
- **AND** requests to that tenant's `/api/v1/memory/*` endpoints SHALL return HTTP 503
- **AND** requests to other tenants SHALL continue to succeed

#### Scenario: Failed LLM credential resolution blocks the tenant
- **WHEN** a tenant's `providers.llm.credentialRef` points to a `SecretReference` that cannot be resolved at startup
- **THEN** the tenant SHALL be marked `loading_failed` with `reason: "llm_credential_unavailable"`
- **AND** its LLM-dependent endpoints SHALL return HTTP 503

#### Scenario: Successful wiring marks tenant available
- **WHEN** all declared providers for a tenant wire successfully
- **THEN** the tenant SHALL be marked `available`
- **AND** its endpoints SHALL serve normally

### Requirement: Readiness Probe Reflects Bootstrap and Tenant Wiring
The `/ready` endpoint SHALL return HTTP 200 only when (a) first-run bootstrap (if applicable) has completed successfully and (b) every tenant record has either reached `available` state or has been explicitly marked `loading_failed` with a reason. A pod that is still actively attempting to wire tenants SHALL return HTTP 503.

#### Scenario: Ready while bootstrap in progress
- **WHEN** first-run bootstrap has not yet completed
- **THEN** `/ready` SHALL return HTTP 503 with `reason: "bootstrap_incomplete"`

#### Scenario: Ready once all tenants resolve
- **WHEN** every tenant has reached either `available` or `loading_failed`
- **AND** bootstrap has completed (or is not applicable)
- **THEN** `/ready` SHALL return HTTP 200

#### Scenario: Per-tenant status endpoint surfaces failure reasons
- **WHEN** a caller GETs `/api/v1/admin/tenants/{slug}/status`
- **AND** the caller has PlatformAdmin or `tenant:read` scope
- **THEN** the response SHALL include `state` (`available` | `loading_failed` | `loading`), `reason` (if any), `providersWired` (list), and `providersFailed` (list with per-provider error messages)

### Requirement: Client-Kind Header Propagation
The HTTP router SHALL extract the `X-Aeterna-Client-Kind` request header and attach it to the request's audit context. Values other than `cli`, `ui`, or `api` SHALL be treated as `api`.

#### Scenario: Valid client kind propagates to audit
- **WHEN** a request carries `X-Aeterna-Client-Kind: ui`
- **THEN** the audit context for that request SHALL record `via: "ui"`

#### Scenario: Invalid client kind normalized to api
- **WHEN** a request carries `X-Aeterna-Client-Kind: something-else`
- **THEN** the audit context SHALL record `via: "api"`
- **AND** the original header value SHALL be included in `client_kind_raw` for observability
