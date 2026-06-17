---
title: Knowledge Repository Specification
status: draft
version: 0.1.0
created: 2025-01-07
authors:
  - AI Systems Architecture Team
related:
  - 01-core-concepts.md
  - 04-memory-knowledge-sync.md
  - 05-adapter-architecture.md
---

# Knowledge Repository Specification

This document specifies the Knowledge Repository component: a versioned, Git-based store for organizational decisions, policies, patterns, and specifications.

## Purpose

The Knowledge Repository provides a versioned, Git-based store for organizational decisions, policies, patterns, and specifications. It ensures that critical institutional knowledge is captured, indexed, and available for automated constraint enforcement.
## Requirements
### Requirement: Knowledge Type Storage
The repository SHALL store knowledge items categorized by type (ADR, policy, pattern, spec) with specific metadata requirements for each type.

#### Scenario: Store an ADR
- **WHEN** a new ADR is proposed with context, decision, and consequences
- **THEN** it SHALL be stored as a versioned Markdown file in the appropriate layer directory

### Requirement: Hierarchical Scoping
The system SHALL support multiple knowledge layers (Tenant, Org, Team, Project) with explicit precedence rules and tenant isolation.

#### Scenario: Project-specific override with tenant context
- **WHEN** a Project-level policy conflicts with a Tenant-level policy within the same tenant
- **THEN** the Project-level policy SHALL take precedence during evaluation for that project
- **AND** precedence SHALL be evaluated within tenant boundaries only

#### Scenario: Cross-tenant hierarchy access attempt
- **WHEN** attempting to access hierarchy levels from another tenant
- **THEN** system SHALL return empty hierarchy
- **AND** system SHALL NOT reveal cross-tenant structure

### Requirement: Git-based Versioning
Every change to the repository SHALL result in an immutable Git commit with a full audit trail. For remote-enabled repositories, commits on the main branch SHALL be synchronized with the remote, and governance-track changes SHALL be committed to feature branches.

#### Scenario: Trace item history
- **WHEN** an item is updated
- **THEN** the system SHALL allow retrieving the full commit history for that specific item

#### Scenario: Fast-track commit pushed to remote
- **WHEN** a fast-track write (Project-layer Draft) is committed
- **THEN** the commit SHALL be pushed to the remote main branch
- **AND** the push SHALL use fast-forward only

#### Scenario: Governance-track commit on branch
- **WHEN** a governance-track write is committed
- **THEN** the commit SHALL be created on a governance branch, not on main
- **AND** the commit SHALL only reach main after PR merge

### Requirement: Manifest Indexing
The repository SHALL maintain a `manifest.json` index of all items for efficient querying and change detection.

#### Scenario: Query by tag
- **WHEN** a query is made for items with specific tags
- **THEN** the system SHALL return matching items using the manifest index

### Requirement: Multi-tenant Federation
The system SHALL support syncing knowledge from upstream repositories while managing local overrides and conflicts.

#### Scenario: Sync from upstream
- **WHEN** an upstream repository is synchronized
- **THEN** new and updated items from acceptable layers SHALL be merged into the local repository

### Requirement: Lifecycle Management
Knowledge items SHALL follow a defined lifecycle (Draft -> Proposed -> Accepted -> Deprecated/Superseded). Status transitions beyond Draft SHALL require governance review when a remote repository is configured.

#### Scenario: Supersede an item
- **WHEN** a new item supersedes an existing one
- **THEN** the status of the old item SHALL be updated to 'superseded' and link to the new item

#### Scenario: Status transition requires governance review
- **WHEN** a status transition from Draft to Proposed or Accepted is requested with a remote URL configured
- **THEN** the system SHALL route the transition through the governance track (branch + PR)
- **AND** the status SHALL only be updated on main after the PR is merged

#### Scenario: Status transition in local-only mode
- **WHEN** a status transition is requested without a remote URL configured
- **THEN** the system SHALL update the status directly via local commit

### Requirement: Knowledge Item Creation
The system SHALL provide a method to propose new knowledge items with automatic ID generation and governance validation.

#### Scenario: Create knowledge item with valid data and tenant context
- **WHEN** proposing a knowledge item with valid type, title, summary, content, and TenantContext
- **THEN** system SHALL validate TenantContext authorization
- **AND** system SHALL validate knowledge content against governance policies
- **AND** system SHALL generate a unique ID
- **AND** system SHALL set initial status to 'draft'
- **AND** system SHALL create Git commit with type='create' and tenant metadata
- **AND** system SHALL return the created item

#### Scenario: Create knowledge item with invalid type
- **WHEN** proposing a knowledge item with invalid type and TenantContext
- **THEN** system SHALL return INVALID_TYPE error
- **AND** error SHALL list valid types (adr, policy, pattern, spec)

#### Scenario: Create knowledge item without tenant context
- **WHEN** proposing a knowledge item without TenantContext
- **THEN** system SHALL return MISSING_TENANT_CONTEXT error

### Requirement: Knowledge Query Operation
The system SHALL provide a method to query knowledge items with flexible filtering and tenant isolation.

#### Scenario: Query all knowledge items with tenant context
- **WHEN** querying knowledge without filters but with TenantContext
- **THEN** system SHALL validate TenantContext authorization
- **AND** system SHALL return all accessible items within the tenant
- **AND** system SHALL include item summaries (not full content)
- **AND** system SHALL include totalCount

#### Scenario: Query with type filter
- **WHEN** querying knowledge with type='adr' and TenantContext
- **THEN** system SHALL only return ADR items within the tenant

#### Scenario: Query with layer filter
- **WHEN** querying knowledge with layer='project' and TenantContext
- **THEN** system SHALL only return project-level knowledge within the tenant

#### Scenario: Query with status filter
- **WHEN** querying knowledge with status=['accepted'] and TenantContext
- **THEN** system SHALL only return accepted items within the tenant
- **AND** system SHALL default to ['accepted'] if not specified

### Requirement: Knowledge Get Operation
The system SHALL provide a method to retrieve a knowledge item by ID with tenant isolation.

#### Scenario: Get existing knowledge item with tenant context
- **WHEN** getting a knowledge item with valid ID and TenantContext
- **THEN** system SHALL validate TenantContext authorization
- **AND** system SHALL verify the item belongs to the same tenant
- **AND** system SHALL return the full item content

#### Scenario: Get non-existent knowledge item
- **WHEN** getting a knowledge item with invalid ID and TenantContext
- **THEN** system SHALL return null without error

#### Scenario: Get knowledge item from different tenant
- **WHEN** getting a knowledge item that belongs to a different tenant
- **THEN** system SHALL return null without revealing cross-tenant information

### Requirement: Constraint Check Operation
The system SHALL validate knowledge items against defined constraints with tenant-specific policy enforcement.

#### Scenario: Check constraint with tenant context
- **WHEN** checking a knowledge item against constraints with TenantContext
- **THEN** system SHALL validate TenantContext authorization
- **AND** system SHALL apply tenant-specific policy constraints
- **AND** system SHALL return constraint violations if any

#### Scenario: Check constraint without tenant context
- **WHEN** checking a knowledge item without TenantContext
- **THEN** system SHALL return MISSING_TENANT_CONTEXT error

### Requirement: Status Update Operation
The system SHALL provide a method to update knowledge item status with governance approval workflows. When a remote repository is configured, status updates to non-Draft states SHALL be routed through the governance track.

#### Scenario: Update status with tenant context and authorization
- **WHEN** updating knowledge item status with TenantContext
- **THEN** system SHALL validate TenantContext authorization
- **AND** system SHALL verify user has appropriate role (TechLead, Architect, Admin)
- **AND** system SHALL enforce governance approval workflow
- **AND** system SHALL create Git commit with status change
- **AND** system SHALL emit governance event (KnowledgeApproved/KnowledgeRejected)

#### Scenario: Update status without required role
- **WHEN** updating knowledge item status with insufficient role permissions
- **THEN** system SHALL return INSUFFICIENT_PERMISSIONS error
- **AND** status SHALL NOT be changed

#### Scenario: Update status routed through governance track
- **WHEN** updating status to Accepted with a remote URL configured
- **THEN** the system SHALL create a governance branch with the status change
- **AND** the system SHALL open a PR for review
- **AND** the status SHALL only be updated on main after PR merge

### Requirement: Manifest Generation
The system SHALL maintain an index of all knowledge items for fast lookups.

#### Scenario: Generate manifest after commit
- **WHEN** a knowledge commit is created
- **THEN** system SHALL regenerate manifest
- **AND** manifest SHALL include all items with metadata
- **AND** manifest SHALL group items by layer
- **AND** manifest SHALL group items by type
- **AND** manifest SHALL group items by status
- **AND** manifest SHALL store current Git commit hash

#### Scenario: Load existing manifest
- **WHEN** system starts
- **THEN** system SHALL load manifest from Git repository
- **AND** system SHALL validate manifest integrity
- **AND** system SHALL use manifest for fast queries

### Requirement: Git Commit Model
The system SHALL use immutable Git commits to track all knowledge changes.

#### Scenario: Create knowledge commit
- **WHEN** a knowledge change occurs
- **THEN** system SHALL create Git commit
- **AND** commit SHALL include affected item IDs
- **AND** commit SHALL include change type (create, update, delete, supersede, status, federation)
- **AND** commit SHALL include manifest snapshot
- **AND** commit SHALL include author and timestamp

#### Scenario: Commit immutability
- **WHEN** a commit exists
- **THEN** system SHALL never modify the commit
- **AND** system SHALL only create new commits

#### Scenario: Get commit history
- **WHEN** requesting knowledge history
- **THEN** system SHALL return all commits for item
- **AND** system SHALL order commits by timestamp (newest first)

### Requirement: Constraint DSL Parsing
The system SHALL parse constraint definitions from knowledge item content.

#### Scenario: Parse must_use constraint
- **WHEN** parsing "must_use: React"
- **THEN** system SHALL create Constraint with operator='must_use'
- **AND** system SHALL set target='dependency'
- **AND** system SHALL set pattern='react'
- **AND** system SHALL set severity from item's severity

#### Scenario: Parse must_not_use constraint
- **WHEN** parsing "must_not_use: eval()"
- **THEN** system SHALL create Constraint with operator='must_not_use'
- **AND** system SHALL set target='code'
- **AND** system SHALL set pattern='eval\(\)'

#### Scenario: Parse must_match constraint with appliesTo
- **WHEN** parsing "must_match: '*.ts' appliesTo: ['src/**']"
- **THEN** system SHALL create Constraint with operator='must_match'
- **AND** system SHALL set target='file'
- **AND** system SHALL set pattern='*.ts'
- **AND** system SHALL set appliesTo=['src/**']

#### Scenario: Invalid constraint syntax
- **WHEN** parsing constraint with invalid syntax
- **THEN** system SHALL return CONSTRAINT_SYNTAX_ERROR
- **AND** error SHALL indicate which part is invalid

### Requirement: Constraint Evaluation
The system SHALL evaluate constraints against provided context.

#### Scenario: Evaluate must_use constraint
- **WHEN** checking must_use for 'react' in dependencies
- **THEN** system SHALL verify 'react' exists in dependencies
- **AND** system SHALL create violation if not found

#### Scenario: Evaluate must_not_use constraint
- **WHEN** checking must_not_use for 'eval()' in code
- **THEN** system SHALL search code for 'eval(' pattern
- **AND** system SHALL create violation if pattern found

#### Scenario: Evaluate must_match constraint
- **WHEN** checking must_match for '*.ts' in files
- **THEN** system SHALL verify all files match pattern
- **AND** system SHALL create violation if file doesn't match

#### Scenario: Evaluate must_exist constraint
- **WHEN** checking must_exist for 'README.md'
- **THEN** system SHALL verify file exists
- **AND** system SHALL create violation if not found

### Requirement: Multi-Tenant Federation
The system SHALL support syncing knowledge from upstream repositories while managing local overrides and conflicts with tenant isolation.

#### Scenario: Sync from upstream with tenant context
- **WHEN** synchronizing from upstream repository with TenantContext
- **THEN** system SHALL validate TenantContext authorization
- **AND** new and updated items from acceptable layers SHALL be merged into the local repository with tenant isolation
- **AND** conflicts SHALL be resolved according to tenant-specific conflict resolution policies

#### Scenario: Sync without tenant context
- **WHEN** synchronizing from upstream repository without TenantContext
- **THEN** system SHALL return MISSING_TENANT_CONTEXT error

### Requirement: Knowledge Error Handling
The system SHALL provide specific error codes for all failure scenarios.

#### Scenario: Item not found error
- **WHEN** getting non-existent knowledge item
- **THEN** system SHALL return ITEM_NOT_FOUND error
- **AND** error SHALL include the requested ID

#### Scenario: Invalid type error
- **WHEN** creating knowledge item with invalid type
- **THEN** system SHALL return INVALID_TYPE error
- **AND** error SHALL list valid types

#### Scenario: Invalid layer error
- **WHEN** creating knowledge item with invalid layer
- **THEN** system SHALL return INVALID_LAYER error
- **AND** error SHALL list valid layers

#### Scenario: Git operation error
- **WHEN** Git operation fails
- **THEN** system SHALL return GIT_ERROR
- **AND** error SHALL include Git error message
- **AND** error SHALL be marked as retryable

#### Scenario: Manifest corrupted error
- **WHEN** manifest fails validation
- **THEN** system SHALL return MANIFEST_CORRUPTED error
- **AND** system SHALL attempt to regenerate from Git history

### Requirement: Tenant Context Propagation
All knowledge operations SHALL require a TenantContext parameter for tenant isolation and authorization.

#### Scenario: Operation without tenant context
- **WHEN** any knowledge operation is attempted without TenantContext
- **THEN** system SHALL return MISSING_TENANT_CONTEXT error
- **AND** operation SHALL NOT proceed

#### Scenario: Tenant context validation
- **WHEN** TenantContext contains invalid or expired credentials
- **THEN** system SHALL return INVALID_TENANT_CONTEXT error
- **AND** operation SHALL NOT proceed

### Requirement: Governance Policy Validation
The system SHALL validate all knowledge operations against tenant governance policies before execution.

#### Scenario: Validate knowledge creation against policies
- **WHEN** creating a knowledge item that violates a tenant policy
- **THEN** system SHALL reject the operation with POLICY_VIOLATION error
- **AND** error SHALL include which policy was violated

#### Scenario: Validate knowledge update against policies
- **WHEN** updating a knowledge item with content that violates a tenant policy
- **THEN** system SHALL reject the operation with POLICY_VIOLATION error
- **AND** error SHALL include which policy was violated

### Requirement: Governance Event Emission
Knowledge operations SHALL emit governance events for audit and real-time monitoring. PR lifecycle transitions SHALL be the primary source of governance events when a remote repository is configured.

#### Scenario: Emit event on knowledge proposal
- **WHEN** a knowledge item is proposed
- **THEN** system SHALL emit a `KnowledgeProposed` event with tenant context
- **AND** event SHALL be published to Redis Streams for real-time consumption

#### Scenario: Emit event on knowledge approval
- **WHEN** a knowledge item is approved
- **THEN** system SHALL emit a `KnowledgeApproved` event with tenant context
- **AND** event SHALL include approver identity and timestamp

#### Scenario: Emit event on PR merge
- **WHEN** a governance PR is merged (detected via webhook or polling)
- **THEN** system SHALL emit a `GovernanceEvent::RequestApproved` event
- **AND** the event SHALL include the PR number, merge commit SHA, and entry metadata

#### Scenario: Emit event on PR rejection
- **WHEN** a governance PR is closed without merging
- **THEN** system SHALL emit a `GovernanceEvent::RequestRejected` event
- **AND** the event SHALL include the PR number and the entry that was rejected

### Requirement: Hindsight Knowledge Type
The repository SHALL support 'hindsight' as a new knowledge type for storing error-resolution patterns.

#### Scenario: Store hindsight item
- **WHEN** creating knowledge item with type='hindsight'
- **THEN** system SHALL validate error_signature and resolutions metadata
- **AND** system SHALL store as versioned Markdown in hindsight/ directory
- **AND** system SHALL generate unique ID with hindsight- prefix

#### Scenario: Query hindsight items
- **WHEN** querying knowledge with type='hindsight'
- **THEN** system SHALL return matching hindsight items
- **AND** system SHALL support filtering by error_type in metadata
- **AND** system SHALL support semantic search on error patterns

#### Scenario: Hindsight lifecycle
- **WHEN** hindsight item reaches success threshold
- **THEN** system MAY auto-transition from draft to proposed
- **AND** system SHALL require approval for accepted status

### Requirement: Summary Storage for Knowledge Items
The repository SHALL store pre-computed summaries for knowledge items to support efficient context assembly.

#### Scenario: Store item summary
- **WHEN** knowledge item is created or updated
- **AND** summary generation is enabled
- **THEN** system SHALL generate summaries at configured depths
- **AND** system SHALL store summaries in item metadata
- **AND** system SHALL include token counts and source hash

#### Scenario: Retrieve item summaries
- **WHEN** querying knowledge items for context assembly
- **THEN** system SHALL return summaries instead of full content
- **AND** system SHALL return appropriate depth based on token budget
- **AND** system SHALL return full content reference for deep inspection

### Requirement: Knowledge Type Configuration Extension
The system SHALL extend type configuration to include hindsight type with specific requirements.

#### Scenario: Hindsight type configuration
- **WHEN** validating hindsight knowledge item
- **THEN** system SHALL require error_signature field in metadata
- **AND** system SHALL require at least one resolution in metadata
- **AND** system SHALL validate error_signature schema (error_type, message_pattern)

#### Scenario: Hindsight file structure
- **WHEN** storing hindsight knowledge item
- **THEN** system SHALL store in {layer}/hindsight/{id}.md
- **AND** system SHALL include structured YAML frontmatter with error metadata

### Requirement: Constraint Support for Hindsight
The system SHALL support constraints derived from hindsight patterns.

#### Scenario: Generate constraint from hindsight
- **WHEN** hindsight item has high success_rate resolution
- **THEN** system MAY auto-generate must_not_match constraint for error pattern
- **AND** system SHALL set constraint severity based on hindsight confidence

#### Scenario: Link constraint to resolution
- **WHEN** constraint from hindsight is violated
- **THEN** system SHALL include resolution suggestion in violation message
- **AND** system SHALL include reference to source hindsight item

### Requirement: Remote Git Synchronization
The knowledge repository SHALL support synchronization with a remote Git repository so that all Aeterna replicas share a single source of truth.

#### Scenario: Clone on first start
- **WHEN** the repository initializes with a configured remote URL and the local path is empty
- **THEN** the system SHALL clone the remote repository to the local path
- **AND** the system SHALL authenticate using the configured SSH key

#### Scenario: Pull on subsequent start
- **WHEN** the repository initializes with a configured remote URL and the local path contains a valid clone
- **THEN** the system SHALL pull from the remote to synchronize with the latest state

#### Scenario: Pull before read
- **WHEN** any read operation (get, list, search, query) is executed with a remote URL configured
- **THEN** the system SHALL pull from the remote before reading the local filesystem
- **AND** the pull SHALL use fast-forward only merge strategy

#### Scenario: Local-only fallback
- **WHEN** no remote URL is configured (AETERNA_KNOWLEDGE_REPO_URL is empty or unset)
- **THEN** the system SHALL operate in local-only mode with no remote operations
- **AND** behavior SHALL be identical to the current implementation

#### Scenario: SSH key authentication
- **WHEN** authenticating with the remote repository
- **THEN** the system SHALL use the SSH private key content from the AETERNA_KNOWLEDGE_REPO_SSH_KEY environment variable
- **AND** the key SHALL be loaded from memory without writing to disk

### Requirement: Two-Track Write Model
The knowledge repository SHALL route write operations through one of two tracks based on the knowledge layer and status.

#### Scenario: Fast track for Project-layer drafts
- **WHEN** a write operation targets a knowledge entry with layer=Project and status=Draft
- **THEN** the system SHALL commit locally and push directly to the main branch
- **AND** the system SHALL retry up to 3 times on push conflict with pull-rebase-push

#### Scenario: Governance track for status transitions
- **WHEN** a status change operation transitions an entry to a non-Draft status (e.g., Draft to Accepted)
- **THEN** the system SHALL create a governance branch, commit the change, and open a pull request
- **AND** the pull request title SHALL describe the status transition
- **AND** the system SHALL NOT modify the main branch directly

#### Scenario: Governance track for higher-layer writes
- **WHEN** a write operation targets a knowledge entry with layer=Team, Org, or Tenant
- **THEN** the system SHALL create a governance branch, commit the change, and open a pull request
- **AND** the pull request body SHALL include the entry metadata and change description

#### Scenario: Governance track for promotions
- **WHEN** a knowledge entry is promoted from one layer to a higher layer
- **THEN** the system SHALL create a governance branch, commit the promoted entry at the target layer path, and open a pull request

#### Scenario: Governance track for deletions
- **WHEN** a delete operation is requested for any knowledge entry with a remote URL configured
- **THEN** the system SHALL create a governance branch, commit the file removal, and open a pull request

#### Scenario: Local-only mode bypasses governance
- **WHEN** no remote URL is configured
- **THEN** all write operations SHALL commit locally without governance track routing
- **AND** no branches or pull requests SHALL be created

### Requirement: PR Lifecycle Governance Events
The system SHALL emit GovernanceEvents corresponding to pull request lifecycle transitions.

#### Scenario: PR opened emits RequestCreated
- **WHEN** a pull request is opened for a governance-track write
- **THEN** the system SHALL emit a GovernanceEvent::RequestCreated event
- **AND** the event SHALL include the PR number, title, entry ID, and requesting user

#### Scenario: PR merged emits RequestApproved
- **WHEN** a pull request is merged (via webhook or polling)
- **THEN** the system SHALL emit a GovernanceEvent::RequestApproved event
- **AND** the system SHALL update the local knowledge entry status accordingly
- **AND** the event SHALL include the PR number, merge commit SHA, and approver

#### Scenario: PR closed without merge emits RequestRejected
- **WHEN** a pull request is closed without being merged
- **THEN** the system SHALL emit a GovernanceEvent::RequestRejected event
- **AND** the system SHALL clean up the local branch reference
- **AND** the event SHALL include the PR number and reason

### Requirement: GitHub Webhook Endpoint
The system SHALL expose an HTTP endpoint that receives GitHub webhook notifications for immediate knowledge synchronization.

#### Scenario: Webhook receives PR merge event
- **WHEN** the webhook endpoint receives a pull_request event with action=closed and merged=true
- **THEN** the system SHALL validate the X-Hub-Signature-256 HMAC signature
- **AND** the system SHALL trigger an immediate git pull on the knowledge repository
- **AND** the system SHALL emit a GovernanceEvent::RequestApproved
- **AND** the system SHALL notify the sync bridge of a CommitMismatch

#### Scenario: Webhook receives PR closed event (not merged)
- **WHEN** the webhook endpoint receives a pull_request event with action=closed and merged=false
- **THEN** the system SHALL validate the HMAC signature
- **AND** the system SHALL emit a GovernanceEvent::RequestRejected

#### Scenario: Webhook receives PR opened event
- **WHEN** the webhook endpoint receives a pull_request event with action=opened
- **THEN** the system SHALL validate the HMAC signature
- **AND** the system SHALL emit a GovernanceEvent::RequestCreated

#### Scenario: Webhook with invalid signature
- **WHEN** the webhook endpoint receives a request with an invalid or missing X-Hub-Signature-256
- **THEN** the system SHALL return HTTP 401 Unauthorized
- **AND** the system SHALL NOT process the event

#### Scenario: Webhook disabled
- **WHEN** AETERNA_WEBHOOK_SECRET is not configured
- **THEN** the webhook endpoint SHALL return HTTP 404 Not Found
- **AND** no webhook processing SHALL occur

### Requirement: Git Provider Abstraction
The system SHALL define an abstract GitProvider trait to decouple knowledge governance from a specific Git hosting platform.

#### Scenario: GitHub provider implementation
- **WHEN** the system is configured with a GitHub repository
- **THEN** the system SHALL use the GitHubProvider implementation of the GitProvider trait
- **AND** all branch, PR, and webhook operations SHALL use the octocrab crate

#### Scenario: Provider trait extensibility
- **WHEN** a new Git hosting platform (e.g., GitLab) needs to be supported
- **THEN** a new implementation of the GitProvider trait SHALL be sufficient
- **AND** no changes to the knowledge repository or governance engine SHALL be required

### Requirement: Governance Branch Naming
The system SHALL use a consistent branch naming convention for governance-track operations.

#### Scenario: Branch name for status change
- **WHEN** creating a governance branch for a status change operation
- **THEN** the branch name SHALL follow the pattern governance/{verb}-{slug}-{yyyymmdd}
- **AND** the verb SHALL match the operation (accept, deprecate, reject)

#### Scenario: Branch name for promotion
- **WHEN** creating a governance branch for a layer promotion
- **THEN** the branch name SHALL follow the pattern governance/promote-{slug}-{yyyymmdd}

#### Scenario: Branch name uniqueness
- **WHEN** a branch name conflicts with an existing branch
- **THEN** the system SHALL append a numeric suffix to ensure uniqueness

### Requirement: PR Idempotency
The system SHALL ensure that duplicate governance operations do not create duplicate pull requests.

#### Scenario: Duplicate PR prevention
- **WHEN** a governance-track write is initiated for an operation that already has an open PR
- **THEN** the system SHALL return the existing PR information without creating a new one

#### Scenario: Closed PR allows new creation
- **WHEN** a governance-track write is initiated for an operation whose previous PR was merged or closed
- **THEN** the system SHALL create a new PR with a new date suffix

### Requirement: PR-Backed Proposal Storage
The system SHALL replace the in-memory knowledge proposal storage with a PR-backed implementation for persistence across pod restarts.

#### Scenario: Propose creates governance branch
- **WHEN** a knowledge proposal is submitted via MCP tools
- **THEN** the system SHALL create a governance branch and commit the proposed entry
- **AND** the proposal SHALL persist across pod restarts via the remote repository

#### Scenario: Submit opens pull request
- **WHEN** a proposed knowledge entry is submitted for review
- **THEN** the system SHALL open a pull request from the governance branch to main
- **AND** the PR body SHALL include the proposal metadata

#### Scenario: List pending reads open PRs
- **WHEN** listing pending proposals
- **THEN** the system SHALL query open pull requests with the governance/ branch prefix
- **AND** the system SHALL return proposal metadata extracted from PR details

### Requirement: Canonical Tenant Repository Binding
The system SHALL maintain a canonical knowledge repository binding for each tenant and use that binding to resolve tenant knowledge operations.

#### Scenario: Tenant binding determines repository resolution
- **WHEN** a knowledge read or write is executed for a tenant
- **THEN** the system SHALL resolve the backing repository from that tenant's configured repository binding
- **AND** the system SHALL NOT fall back to a process-global repository default that is shared implicitly across tenants

#### Scenario: Missing tenant binding fails closed
- **WHEN** a tenant-scoped knowledge operation requires a repository binding and no valid binding exists for that tenant
- **THEN** the system SHALL reject the operation with a configuration error
- **AND** the system SHALL NOT route the operation to another tenant's repository or to an unspecified default repository

#### Scenario: Tenant binding isolation
- **WHEN** one tenant updates its repository binding
- **THEN** the change SHALL affect only that tenant's knowledge resolution
- **AND** knowledge operations for other tenants SHALL continue using their own bindings unchanged

#### Scenario: Sync-managed tenant preserves admin-managed binding
- **WHEN** an IdP or hierarchy sync updates a tenant or its synced hierarchy records
- **THEN** the sync SHALL NOT overwrite an admin-managed tenant repository binding unless an explicit binding-ownership policy allows it
- **AND** tenant knowledge operations SHALL continue using the last valid binding for that tenant

### Requirement: Tenant Repository Binding Validation
The system SHALL validate tenant repository bindings before accepting configuration changes.

#### Scenario: Valid tenant repository binding
- **WHEN** an administrator configures a tenant repository binding with valid repository settings and credential references
- **THEN** the system SHALL persist the binding
- **AND** the binding SHALL be available for subsequent knowledge operations and admin inspection

#### Scenario: Invalid tenant repository binding
- **WHEN** an administrator configures a tenant repository binding with invalid structure, unsupported repository mode, or missing credential references
- **THEN** the system SHALL reject the binding update
- **AND** the error SHALL identify which binding fields failed validation

#### Scenario: Binding stores secret references instead of raw secrets
- **WHEN** an administrator configures a tenant repository binding that requires credentials
- **THEN** the system SHALL persist secret references or handles rather than raw secret material in the binding record
- **AND** binding inspection responses SHALL redact or omit secret-reference resolution details that would expose sensitive values

### Requirement: Knowledge Promotion Request Lifecycle
The system SHALL treat promotion as a first-class lifecycle request distinct from generic knowledge CRUD.

#### Scenario: Create promotion request
- **WHEN** a user requests promotion of an accepted knowledge item to a higher layer
- **THEN** the system SHALL create a PromotionRequest
- **AND** the request SHALL capture target layer, promotion mode, shared content, residual content, residual role, and justification
- **AND** the request SHALL enter PendingReview status

### Requirement: Partial Promotion Preservation
The system SHALL preserve lower-layer specificity when only part of a knowledge item is promoted.

#### Scenario: Promote shared core and retain specialization
- **WHEN** a user promotes only the shared portion of a project-layer knowledge item
- **THEN** the system SHALL create a higher-layer canonical knowledge item
- **AND** the lower-layer item SHALL remain available
- **AND** the lower-layer item SHALL be marked with an appropriate semantic role such as Specialization, Applicability, Exception, or Clarification
- **AND** the items SHALL be linked by explicit semantic relations

### Requirement: Full Promotion Replacement
The system SHALL allow full promotion when higher-layer content fully replaces lower-layer content.

#### Scenario: Full-equivalence promotion
- **WHEN** reviewers approve a promotion as a full replacement
- **THEN** the promoted higher-layer item SHALL become canonical
- **AND** the lower-layer item SHALL transition to Superseded
- **AND** the system SHALL record explicit supersession relations

### Requirement: Knowledge Semantic Relations
The system SHALL persist explicit semantic relations between knowledge items.

#### Scenario: Create specialization relation
- **WHEN** a promoted item leaves project-specific detail behind
- **THEN** the system SHALL record a Specializes relation from the lower-layer item to the higher-layer canonical item

### Requirement: Deterministic Resolution Precedence
The system SHALL resolve canonical and residual knowledge deterministically.

#### Scenario: Resolve canonical and specialization together
- **WHEN** a query matches both a higher-layer canonical item and a local specialization
- **THEN** the system SHALL return the canonical item as shared truth
- **AND** the system SHALL return the local specialization as linked residual context
- **AND** the response SHALL preserve relation metadata

### Requirement: Confidential Promotion Validation
The system SHALL prevent confidential or scope-inappropriate content from being promoted to broader layers.

#### Scenario: Reject confidential promotion
- **WHEN** a promotion request contains content that violates confidentiality policy for the target layer
- **THEN** the system SHALL reject the promotion request
- **AND** the source lower-layer item SHALL remain unchanged

### Requirement: Tenant Boundary Enforcement
The system SHALL enforce tenant boundaries across promotion workflows.

#### Scenario: Cross-tenant promotion forbidden
- **WHEN** a promotion request attempts to promote knowledge across tenants without explicit federation support
- **THEN** the system SHALL reject the request
- **AND** no promoted item or relation SHALL be created

### Requirement: Promotion Lifecycle Auditability
The system SHALL audit all promotion lifecycle transitions.

#### Scenario: Audit promotion approval
- **WHEN** a promotion request is approved
- **THEN** the system SHALL record the approver identity, reviewer decision, resulting item IDs, relation changes, and timestamp

## Table of Contents

1. [Overview](#overview)
2. [Knowledge Types](#knowledge-types)
3. [Knowledge Item Schema](#knowledge-item-schema)
4. [Constraint DSL](#constraint-dsl)
5. [Repository Structure](#repository-structure)
6. [Versioning Model](#versioning-model)
7. [Multi-Tenant Federation](#multi-tenant-federation)
8. [Core Operations](#core-operations)
9. [Error Handling](#error-handling)

---

## Overview

The Knowledge Repository provides:

- **Structured storage**: Typed artifacts (ADRs, policies, patterns, specs)
- **Git-based versioning**: Full audit trail, immutable commits
- **Constraint enforcement**: Declarative rules guiding agent behavior
- **Multi-tenant federation**: Tenant → Org → Team → Project layers

```
┌─────────────────────────────────────────────────────────────────┐
│                   KNOWLEDGE REPOSITORY                           │
│                                                                  │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                 Knowledge Manager                        │    │
│  │  • Coordinates all knowledge operations                  │    │
│  │  • Enforces schema validation                            │    │
│  │  • Routes to appropriate layer                           │    │
│  └─────────────────────────────────────────────────────────┘    │
│                              │                                   │
│                              ▼                                   │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                 Constraint Engine                        │    │
│  │  • Parses constraint DSL                                 │    │
│  │  • Evaluates constraints against context                 │    │
│  │  • Reports violations by severity                        │    │
│  └─────────────────────────────────────────────────────────┘    │
│                              │                                   │
│                              ▼                                   │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                  Version Manager                         │    │
│  │  • Creates immutable commits                             │    │
│  │  • Manages manifest index                                │    │
│  │  • Handles federation sync                               │    │
│  └─────────────────────────────────────────────────────────┘    │
│                              │                                   │
│                              ▼                                   │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                   Git Backend                            │    │
│  │  • Persists to Git repository                            │    │
│  │  • Handles branching and merging                         │    │
│  │  • Supports local and remote repos                       │    │
│  └─────────────────────────────────────────────────────────┘    │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Knowledge Types

### The Four Knowledge Types

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  TYPE        PURPOSE                       EXAMPLES              │
│  ─────────────────────────────────────────────────────────────  │
│                                                                  │
│  adr         Architecture Decision         "Use PostgreSQL"     │
│              Records                        "Adopt microservices"│
│                                                                  │
│  policy      Organizational rules           "No console.log"     │
│              and constraints               "PR reviews required" │
│                                                                  │
│  pattern     Reusable solutions            "Error handling"      │
│              and best practices            "API response format" │
│                                                                  │
│  spec        Technical specifications       "API contract"       │
│              and contracts                  "Data schema"        │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Type Characteristics

| Type | Immutable | Has Constraints | Supersedes | Typical Size |
|------|-----------|-----------------|------------|--------------|
| `adr` | Yes (decisions are final) | Often | Previous ADRs | Medium |
| `policy` | No (can be updated) | Always | - | Small |
| `pattern` | No (evolves) | Sometimes | - | Large |
| `spec` | Yes (versioned) | Sometimes | Previous versions | Large |

### Type Definitions

```typescript
type KnowledgeType = 'adr' | 'policy' | 'pattern' | 'spec';

interface KnowledgeTypeConfig {
  /** Human-readable name */
  displayName: string;
  
  /** Description of this type */
  description: string;
  
  /** Whether items of this type can be updated (vs superseded) */
  allowUpdates: boolean;
  
  /** Required fields for this type */
  requiredFields: string[];
  
  /** File extension for this type */
  fileExtension: string;
}

const knowledgeTypeConfigs: Record<KnowledgeType, KnowledgeTypeConfig> = {
  adr: {
    displayName: 'Architecture Decision Record',
    description: 'Documents significant architectural decisions',
    allowUpdates: false, // Supersede instead
    requiredFields: ['context', 'decision', 'consequences'],
    fileExtension: '.md'
  },
  policy: {
    displayName: 'Policy',
    description: 'Organizational rules and guidelines',
    allowUpdates: true,
    requiredFields: ['scope', 'rules'],
    fileExtension: '.md'
  },
  pattern: {
    displayName: 'Pattern',
    description: 'Reusable solutions and best practices',
    allowUpdates: true,
    requiredFields: ['problem', 'solution'],
    fileExtension: '.md'
  },
  spec: {
    displayName: 'Specification',
    description: 'Technical specifications and contracts',
    allowUpdates: false, // Version instead
    requiredFields: ['version'],
    fileExtension: '.md'
  }
};
```

---

## Knowledge Item Schema

### Core Schema

```typescript
/**
 * A single knowledge item in the repository.
 */
interface KnowledgeItem {
  /** Unique identifier (e.g., "adr-042-database-selection") */
  id: string;
  
  /** Knowledge type */
  type: KnowledgeType;
  
  /** Layer this item belongs to */
  layer: KnowledgeLayer;
  
  /** Human-readable title */
  title: string;
  
  /** Brief summary (for memory pointer) */
  summary: string;
  
  /** Full content (Markdown) */
  content: string;
  
  /** SHA-256 hash of content */
  contentHash: string;
  
  /** Status in lifecycle */
  status: KnowledgeStatus;
  
  /** Severity for constraint enforcement */
  severity: ConstraintSeverity;
  
  /** Attached constraints */
  constraints: Constraint[];
  
  /** Tags for categorization */
  tags: string[];
  
  /** Metadata */
  metadata: KnowledgeMetadata;
  
  /** Creation timestamp */
  createdAt: string;
  
  /** Last update timestamp */
  updatedAt: string;
  
  /** Version number (for specs) */
  version?: string;
  
  /** ID of superseded item (for ADRs) */
  supersedes?: string;
  
  /** IDs of items that supersede this one */
  supersededBy?: string[];
}

type KnowledgeLayer = 'tenant' | 'org' | 'team' | 'project';

type KnowledgeStatus = 
  | 'draft'      // Work in progress
  | 'proposed'   // Ready for review
  | 'accepted'   // Approved and active
  | 'deprecated' // No longer recommended
  | 'superseded' // Replaced by another item
  | 'rejected';  // Not accepted

type ConstraintSeverity = 
  | 'info'   // Informational, no enforcement
  | 'warn'   // Warning, can be overridden
  | 'block'; // Blocking, must be followed

interface KnowledgeMetadata {
  /** Authors */
  authors?: string[];
  
  /** Reviewers who approved */
  reviewers?: string[];
  
  /** Related items */
  relatedItems?: string[];
  
  /** External references */
  references?: string[];
  
  /** Custom fields */
  [key: string]: unknown;
}
```

### Knowledge Layer Hierarchy

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  LAYER       SCOPE                  EXAMPLES                     │
│  ─────────────────────────────────────────────────────────────  │
│                                                                  │
│  tenant     Entire tenant         "All code must be typed"    │
│     │        (least specific)       "Security policy"           │
│     │                                                            │
│  org         Business unit          "Use React for frontend"    │
│     │                               "API versioning scheme"     │
│     │                                                            │
│  team        Team scope             "Code review checklist"     │
│     │                               "Sprint ceremonies"         │
│     │                                                            │
│  project     Single project         "Project-specific ADRs"     │
│              (most specific)        "Local conventions"         │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Example Knowledge Items

#### ADR Example

```yaml
---
id: adr-042-database-selection
type: adr
layer: org
title: Database Selection for New Services
summary: Use PostgreSQL for all new services requiring relational data storage
status: accepted
severity: block
tags: [database, infrastructure, architecture]
metadata:
  authors: [alice@example.com]
  reviewers: [bob@example.com, charlie@example.com]
  relatedItems: [adr-015-data-layer]
createdAt: "2025-01-07T10:00:00Z"
updatedAt: "2025-01-07T10:00:00Z"
---

## Context

We need to standardize on a relational database for new services. Currently, teams use MySQL, PostgreSQL, and SQLite inconsistently.

## Decision

We will use **PostgreSQL** for all new services requiring relational data storage.

## Rationale

1. Superior JSON support for semi-structured data
2. Better performance for complex queries
3. Strong community and ecosystem
4. Existing operational expertise

## Consequences

### Positive
- Consistent tooling and operations
- Easier cross-service data integration
- Reduced training burden

### Negative
- Teams with MySQL expertise need to learn PostgreSQL
- Some existing services will remain on MySQL

## Constraints

```yaml
constraints:
  - operator: must_use
    target: dependency
    pattern: "postgresql|pg|postgres"
    severity: block
    message: "Use PostgreSQL for relational data per ADR-042"
  - operator: must_not_use
    target: dependency
    pattern: "mysql|mysql2|mariadb"
    severity: block
    message: "MySQL not allowed for new services per ADR-042"
```
```

#### Policy Example

```yaml
---
id: policy-no-console-log
type: policy
layer: tenant
title: No Console.log in Production Code
summary: Console.log statements must be removed before merging to main
status: accepted
severity: warn
tags: [logging, code-quality]
metadata:
  authors: [security-team@example.com]
createdAt: "2025-01-01T00:00:00Z"
updatedAt: "2025-01-07T00:00:00Z"
---

## Scope

All production code in any language that has console/print debugging.

## Rules

1. No `console.log()` in JavaScript/TypeScript production code
2. No `print()` debugging in Python production code
3. Use proper logging frameworks with log levels

## Exceptions

- Test files (*.test.ts, *_test.py)
- Development-only scripts
- CLI tools intended for local use

## Constraints

```yaml
constraints:
  - operator: must_not_match
    target: code
    pattern: "console\\.log\\("
    appliesTo: ["*.ts", "*.js"]
    severity: warn
    message: "Remove console.log before merging (use logger instead)"
```
```

---

## Constraint DSL

### Constraint Schema

```typescript
/**
 * A constraint attached to a knowledge item.
 */
interface Constraint {
  /** Constraint operator */
  operator: ConstraintOperator;
  
  /** What the constraint targets */
  target: ConstraintTarget;
  
  /** Pattern to match (regex or glob depending on target) */
  pattern: string;
  
  /** File patterns this applies to (glob) */
  appliesTo?: string[];
  
  /** Severity of violation */
  severity: ConstraintSeverity;
  
  /** Human-readable message on violation */
  message?: string;
}

type ConstraintOperator =
  | 'must_use'       // Pattern MUST be present
  | 'must_not_use'   // Pattern MUST NOT be present
  | 'must_match'     // Content MUST match pattern
  | 'must_not_match' // Content MUST NOT match pattern
  | 'must_exist'     // File/path MUST exist
  | 'must_not_exist';// File/path MUST NOT exist

type ConstraintTarget =
  | 'file'       // File paths
  | 'code'       // Code content
  | 'dependency' // Package dependencies
  | 'import'     // Import statements
  | 'config';    // Configuration files
```

### Operator Semantics

| Operator | Target | Pattern | Passes When |
|----------|--------|---------|-------------|
| `must_use` | dependency | `postgresql` | `postgresql` is in dependencies |
| `must_not_use` | dependency | `mysql` | `mysql` is NOT in dependencies |
| `must_match` | code | `^import.*from ['"]@tenant/` | All imports from @tenant/* |
| `must_not_match` | code | `console\.log\(` | No console.log statements |
| `must_exist` | file | `README.md` | README.md exists |
| `must_not_exist` | file | `.env.local` | .env.local not committed |

### Constraint Evaluation

```typescript
interface ConstraintContext {
  /** Files being checked */
  files: FileInfo[];
  
  /** Dependencies from package.json/requirements.txt/etc */
  dependencies: DependencyInfo[];
  
  /** Current layer identifiers */
  layerIdentifiers: {
    companyId?: string;
    orgId?: string;
    teamId?: string;
    projectId?: string;
  };
}

interface FileInfo {
  path: string;
  content: string;
}

interface DependencyInfo {
  name: string;
  version: string;
  type: 'production' | 'development';
}

interface ConstraintViolation {
  /** The constraint that was violated */
  constraint: Constraint;
  
  /** Knowledge item the constraint came from */
  knowledgeItemId: string;
  
  /** Where the violation occurred */
  location?: {
    file: string;
    line?: number;
    column?: number;
  };
  
  /** Severity of this violation */
  severity: ConstraintSeverity;
  
  /** Human-readable message */
  message: string;
}

interface ConstraintCheckResult {
  /** Whether all constraints passed */
  passed: boolean;
  
  /** All violations found */
  violations: ConstraintViolation[];
  
  /** Violations by severity */
  summary: {
    info: number;
    warn: number;
    block: number;
  };
}
```

### Evaluation Algorithm

```typescript
function evaluateConstraints(
  constraints: Constraint[],
  context: ConstraintContext
): ConstraintCheckResult {
  const violations: ConstraintViolation[] = [];
  
  for (const constraint of constraints) {
    const applicableFiles = constraint.appliesTo
      ? context.files.filter(f => matchGlob(f.path, constraint.appliesTo!))
      : context.files;
    
    switch (constraint.target) {
      case 'dependency':
        evaluateDependencyConstraint(constraint, context.dependencies, violations);
        break;
      case 'code':
        evaluateCodeConstraint(constraint, applicableFiles, violations);
        break;
      case 'file':
        evaluateFileConstraint(constraint, context.files, violations);
        break;
      case 'import':
        evaluateImportConstraint(constraint, applicableFiles, violations);
        break;
      case 'config':
        evaluateConfigConstraint(constraint, context.files, violations);
        break;
    }
  }
  
  return {
    passed: violations.filter(v => v.severity === 'block').length === 0,
    violations,
    summary: {
      info: violations.filter(v => v.severity === 'info').length,
      warn: violations.filter(v => v.severity === 'warn').length,
      block: violations.filter(v => v.severity === 'block').length
    }
  };
}
```

### Severity Behavior

| Severity | On Violation | Agent Behavior |
|----------|--------------|----------------|
| `info` | Log message | Continue normally |
| `warn` | Show warning | Continue with caution, may prompt user |
| `block` | Raise error | Stop action, explain constraint, suggest fix |

---

## Repository Structure

### Directory Layout

```
knowledge-repo/
├── manifest.json           # Index of all items
├── tenant/                # Tenant-wide knowledge
│   ├── adrs/
│   │   └── adr-001-*.md
│   ├── policies/
│   │   └── policy-*.md
│   ├── patterns/
│   │   └── pattern-*.md
│   └── specs/
│       └── spec-*.md
├── orgs/                   # Organization-specific
│   └── {orgId}/
│       ├── adrs/
│       ├── policies/
│       ├── patterns/
│       └── specs/
├── teams/                  # Team-specific
│   └── {teamId}/
│       ├── adrs/
│       ├── policies/
│       ├── patterns/
│       └── specs/
└── projects/               # Project-specific
    └── {projectId}/
        ├── adrs/
        ├── policies/
        ├── patterns/
        └── specs/
```

### Manifest Schema

```typescript
interface KnowledgeManifest {
  /** Manifest version */
  version: '1.0';
  
  /** Generation timestamp */
  generatedAt: string;
  
  /** Git commit hash */
  commitHash: string;
  
  /** All items by ID */
  items: Record<string, ManifestEntry>;
  
  /** Items grouped by layer */
  byLayer: Record<KnowledgeLayer, string[]>;
  
  /** Items grouped by type */
  byType: Record<KnowledgeType, string[]>;
  
  /** Items grouped by status */
  byStatus: Record<KnowledgeStatus, string[]>;
}

interface ManifestEntry {
  /** Item ID */
  id: string;
  
  /** Item type */
  type: KnowledgeType;
  
  /** Item layer */
  layer: KnowledgeLayer;
  
  /** File path relative to repo root */
  path: string;
  
  /** Item title */
  title: string;
  
  /** Item summary */
  summary: string;
  
  /** Current status */
  status: KnowledgeStatus;
  
  /** Content hash for change detection */
  contentHash: string;
  
  /** Whether item has constraints */
  hasConstraints: boolean;
  
  /** Constraint count by severity */
  constraintSeverity?: {
    info: number;
    warn: number;
    block: number;
  };
  
  /** Tags */
  tags: string[];
  
  /** Last modified timestamp */
  updatedAt: string;
}
```

### Example Manifest

```json
{
  "version": "1.0",
  "generatedAt": "2025-01-07T12:00:00Z",
  "commitHash": "abc123def456",
  "items": {
    "adr-042-database-selection": {
      "id": "adr-042-database-selection",
      "type": "adr",
      "layer": "org",
      "path": "orgs/engineering/adrs/adr-042-database-selection.md",
      "title": "Database Selection for New Services",
      "summary": "Use PostgreSQL for all new services",
      "status": "accepted",
      "contentHash": "sha256:abc123...",
      "hasConstraints": true,
      "constraintSeverity": {
        "info": 0,
        "warn": 0,
        "block": 2
      },
      "tags": ["database", "infrastructure"],
      "updatedAt": "2025-01-07T10:00:00Z"
    }
  },
  "byLayer": {
    "tenant": ["policy-security-baseline"],
    "org": ["adr-042-database-selection"],
    "team": [],
    "project": []
  },
  "byType": {
    "adr": ["adr-042-database-selection"],
    "policy": ["policy-security-baseline"],
    "pattern": [],
    "spec": []
  },
  "byStatus": {
    "accepted": ["adr-042-database-selection", "policy-security-baseline"],
    "draft": [],
    "proposed": [],
    "deprecated": [],
    "superseded": [],
    "rejected": []
  }
}
```

---

## Versioning Model

### Commit Schema

```typescript
/**
 * An immutable commit in the knowledge repository.
 */
interface KnowledgeCommit {
  /** Commit hash */
  hash: string;
  
  /** Parent commit hash (null for initial) */
  parent: string | null;
  
  /** Commit timestamp */
  timestamp: string;
  
  /** Author identifier */
  author: string;
  
  /** Commit message */
  message: string;
  
  /** Type of change */
  changeType: CommitChangeType;
  
  /** Items affected */
  affectedItems: string[];
  
  /** Manifest snapshot at this commit */
  manifest: KnowledgeManifest;
}

type CommitChangeType =
  | 'create'     // New item added
  | 'update'     // Existing item modified
  | 'delete'     // Item removed
  | 'supersede'  // Item superseded by another
  | 'status'     // Status change only
  | 'federation';// Sync from upstream
```

### Version History

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  Commit: abc123                                                  │
│  Parent: def456                                                  │
│  Author: alice@example.com                                       │
│  Date:   2025-01-07T10:00:00Z                                   │
│  Type:   create                                                  │
│                                                                  │
│  Message: Add ADR-042: Database Selection                        │
│                                                                  │
│  Affected Items:                                                 │
│    + adr-042-database-selection                                 │
│                                                                  │
│  ─────────────────────────────────────────────────────────────  │
│                                                                  │
│  Commit: def456                                                  │
│  Parent: ghi789                                                  │
│  Author: bob@example.com                                         │
│  Date:   2025-01-06T15:30:00Z                                   │
│  Type:   update                                                  │
│                                                                  │
│  Message: Update security policy with new requirements           │
│                                                                  │
│  Affected Items:                                                 │
│    ~ policy-security-baseline                                   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Immutability Rules

1. **Commits are immutable**: Once created, never modified
2. **Items can be superseded**: New item references old one
3. **History is append-only**: No rebasing or force-push
4. **Manifest reflects current state**: Regenerated on each commit

---

## Multi-Tenant Federation

### Federation Model

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│                     CENTRAL HUB                                  │
│                  (tenant-wide repo)                             │
│                                                                  │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │  tenant/                                                │    │
│  │    └── policies/                                         │    │
│  │    └── patterns/                                         │    │
│  └─────────────────────────────────────────────────────────┘    │
│                              │                                   │
│              ┌───────────────┼───────────────┐                   │
│              │               │               │                   │
│              ▼               ▼               ▼                   │
│  ┌───────────────┐ ┌───────────────┐ ┌───────────────┐          │
│  │   ORG REPO    │ │   ORG REPO    │ │   ORG REPO    │          │
│  │  (Engineering)│ │   (Product)   │ │   (Platform)  │          │
│  │               │ │               │ │               │          │
│  │ orgs/eng/     │ │ orgs/prod/    │ │ orgs/plat/    │          │
│  │   └── adrs/   │ │   └── adrs/   │ │   └── adrs/   │          │
│  └───────┬───────┘ └───────────────┘ └───────────────┘          │
│          │                                                       │
│          ▼                                                       │
│  ┌───────────────┐                                               │
│  │  PROJECT REPO │                                               │
│  │  (Backend API)│                                               │
│  │               │                                               │
│  │ projects/api/ │                                               │
│  │   └── adrs/   │                                               │
│  └───────────────┘                                               │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Federation Config

```typescript
interface FederationConfig {
  /** Central hub repository */
  centralHub?: {
    url: string;
    branch: string;
    syncInterval: string; // e.g., "1h", "6h", "1d"
  };
  
  /** Upstream repositories to sync from */
  upstreams: UpstreamConfig[];
  
  /** Layers to accept from each upstream */
  layerMapping: Record<string, KnowledgeLayer[]>;
}

interface UpstreamConfig {
  /** Unique identifier */
  id: string;
  
  /** Repository URL */
  url: string;
  
  /** Branch to sync */
  branch: string;
  
  /** Layers to pull */
  layers: KnowledgeLayer[];
  
  /** Auto-sync enabled */
  autoSync: boolean;
}
```

### Sync Algorithm

```typescript
async function syncFromUpstream(
  upstream: UpstreamConfig,
  localRepo: KnowledgeRepository
): Promise<SyncResult> {
  // 1. Fetch upstream manifest
  const upstreamManifest = await fetchManifest(upstream);
  
  // 2. Compare with local
  const localManifest = await localRepo.getManifest();
  
  // 3. Compute delta
  const delta = computeDelta(localManifest, upstreamManifest, upstream.layers);
  
  // 4. Apply changes
  const applied: string[] = [];
  const conflicts: ConflictInfo[] = [];
  
  for (const item of delta.added) {
    await localRepo.createItem(item);
    applied.push(item.id);
  }
  
  for (const item of delta.updated) {
    const local = await localRepo.getItem(item.id);
    if (local && local.contentHash !== item.contentHash) {
      // Conflict: both modified
      conflicts.push({ itemId: item.id, local, upstream: item });
    } else {
      await localRepo.updateItem(item);
      applied.push(item.id);
    }
  }
  
  for (const itemId of delta.deleted) {
    await localRepo.markSuperseded(itemId, 'upstream-deleted');
    applied.push(itemId);
  }
  
  // 5. Create sync commit
  await localRepo.commit({
    message: `Sync from upstream: ${upstream.id}`,
    changeType: 'federation',
    affectedItems: applied
  });
  
  return { applied, conflicts };
}
```

### Layer Precedence in Federation

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  When same item exists at multiple layers:                      │
│                                                                  │
│    project  ◄── Highest precedence (most specific)              │
│       │                                                          │
│    team                                                          │
│       │                                                          │
│    org                                                           │
│       │                                                          │
│    tenant ◄── Lowest precedence (least specific)               │
│                                                                  │
│  Project-level items override tenant-level items               │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Core Operations

### Operation: Query Knowledge

Search for knowledge items matching criteria.

```typescript
interface QueryKnowledgeInput {
  /** Text search query */
  query?: string;
  
  /** Filter by type */
  type?: KnowledgeType;
  
  /** Filter by layer */
  layer?: KnowledgeLayer;
  
  /** Filter by status */
  status?: KnowledgeStatus | KnowledgeStatus[];
  
  /** Filter by tags */
  tags?: string[];
  
  /** Filter by severity */
  severity?: ConstraintSeverity;
  
  /** Maximum results */
  limit?: number;
  
  /** Layer identifiers for scoping */
  identifiers?: {
    companyId?: string;
    orgId?: string;
    teamId?: string;
    projectId?: string;
  };
}

interface QueryKnowledgeOutput {
  /** Matching items (summary only, not full content) */
  items: KnowledgeItemSummary[];
  
  /** Total count */
  totalCount: number;
}

interface KnowledgeItemSummary {
  id: string;
  type: KnowledgeType;
  layer: KnowledgeLayer;
  title: string;
  summary: string;
  status: KnowledgeStatus;
  tags: string[];
  hasConstraints: boolean;
  updatedAt: string;
}
```

### Operation: Get Knowledge Item

Retrieve full content of a knowledge item.

```typescript
interface GetKnowledgeInput {
  /** Item ID */
  id: string;
  
  /** Include constraint details */
  includeConstraints?: boolean;
  
  /** Include version history */
  includeHistory?: boolean;
}

interface GetKnowledgeOutput {
  /** Full knowledge item */
  item: KnowledgeItem | null;
  
  /** Version history (if requested) */
  history?: KnowledgeCommit[];
}
```

### Operation: Check Constraints

Evaluate constraints against a context.

```typescript
interface CheckConstraintsInput {
  /** Files to check */
  files?: FileInfo[];
  
  /** Dependencies to check */
  dependencies?: DependencyInfo[];
  
  /** Specific knowledge items to check (or all if empty) */
  knowledgeItemIds?: string[];
  
  /** Only check certain severity levels */
  minSeverity?: ConstraintSeverity;
  
  /** Layer identifiers */
  identifiers: {
    companyId?: string;
    orgId?: string;
    teamId?: string;
    projectId?: string;
  };
}

interface CheckConstraintsOutput {
  /** Check result */
  result: ConstraintCheckResult;
  
  /** Knowledge items that were checked */
  checkedItems: string[];
}
```

### Operation: Propose Knowledge

Create a new knowledge item proposal.

```typescript
interface ProposeKnowledgeInput {
  /** Item type */
  type: KnowledgeType;
  
  /** Title */
  title: string;
  
  /** Summary */
  summary: string;
  
  /** Full content (Markdown) */
  content: string;
  
  /** Target layer */
  layer: KnowledgeLayer;
  
  /** Severity */
  severity?: ConstraintSeverity;
  
  /** Constraints */
  constraints?: Constraint[];
  
  /** Tags */
  tags?: string[];
  
  /** If superseding existing item */
  supersedes?: string;
}

interface ProposeKnowledgeOutput {
  /** Created item (in draft status) */
  item: KnowledgeItem;
  
  /** Generated ID */
  id: string;
  
  /** Path in repository */
  path: string;
}
```

### Operation: Update Knowledge Status

Change the status of a knowledge item.

```typescript
interface UpdateStatusInput {
  /** Item ID */
  id: string;
  
  /** New status */
  status: KnowledgeStatus;
  
  /** Reason for status change */
  reason?: string;
}

interface UpdateStatusOutput {
  /** Updated item */
  item: KnowledgeItem;
  
  /** Commit hash */
  commitHash: string;
}
```

---

## Error Handling

### Error Response Format

```typescript
interface KnowledgeError {
  /** Error code */
  code: KnowledgeErrorCode;
  
  /** Human-readable message */
  message: string;
  
  /** Operation that failed */
  operation: string;
  
  /** Additional context */
  details?: Record<string, unknown>;
}

type KnowledgeErrorCode =
  | 'ITEM_NOT_FOUND'
  | 'INVALID_TYPE'
  | 'INVALID_LAYER'
  | 'INVALID_STATUS_TRANSITION'
  | 'INVALID_CONSTRAINT'
  | 'DUPLICATE_ID'
  | 'MANIFEST_CORRUPTED'
  | 'GIT_ERROR'
  | 'FEDERATION_ERROR'
  | 'VALIDATION_ERROR';
```

### Valid Status Transitions

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  draft ──────────────► proposed                                 │
│    │                      │                                      │
│    │                      ├──────► accepted                     │
│    │                      │           │                          │
│    │                      │           ├──────► deprecated       │
│    │                      │           │                          │
│    │                      │           └──────► superseded       │
│    │                      │                                      │
│    │                      └──────► rejected                     │
│    │                                                             │
│    └─────────────────────► rejected                             │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

**Next**: [04-memory-knowledge-sync.md](./04-memory-knowledge-sync.md) - Memory-Knowledge Sync Specification
