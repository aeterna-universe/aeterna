---
title: Admin Dashboard Specification
status: draft
version: 0.1.0
created: 2026-04-11
authors:
  - AI Systems Architecture Team
related:
  - server-runtime
  - opencode-plugin-auth
  - granular-authorization
  - tenant-admin-control-plane
  - runtime-operations
  - memory-system
  - knowledge-repository
  - governance
---

## Purpose

The Admin Dashboard capability provides a browser-based administration interface for Aeterna platform operators and tenant administrators. It enables visual management of tenants, organizational hierarchies, users, roles, knowledge, memories, governance workflows, policies, drift detection, and system health through a single-page application served directly from the Aeterna server.

## Requirements

### Requirement: Admin UI Authentication
The admin dashboard SHALL authenticate users through the existing plugin_auth flow, adapting the GitHub OAuth exchange for browser-based sessions.

#### Scenario: User logs in via GitHub OAuth
- **WHEN** an unauthenticated user navigates to `/admin`
- **THEN** the dashboard SHALL display a login page with a GitHub authentication option
- **AND** the login flow SHALL exchange a GitHub access token for Aeterna-issued JWT credentials via `POST /api/v1/auth/plugin/bootstrap`
- **AND** the dashboard SHALL store the access and refresh tokens for subsequent API requests

#### Scenario: Access token automatic refresh
- **WHEN** the stored access token is expired or within 60 seconds of expiry
- **THEN** the dashboard SHALL automatically refresh the token via `POST /api/v1/auth/plugin/refresh`
- **AND** subsequent API requests SHALL use the refreshed token without user interaction

#### Scenario: Refresh token expired or revoked
- **WHEN** the refresh token is expired, revoked, or invalid
- **THEN** the dashboard SHALL redirect the user to the login page
- **AND** the dashboard SHALL clear all stored tokens

#### Scenario: PlatformAdmin role detection
- **WHEN** a user authenticates and their resolved roles include a PlatformAdmin grant stored under the `__root__` sentinel tenant
- **THEN** the dashboard SHALL display PlatformAdmin-only navigation items (tenant management, platform settings)
- **AND** the dashboard SHALL enable the cross-tenant selector in the header

### Requirement: Tenant Context Management
The admin dashboard SHALL support multi-tenant context switching for PlatformAdmin users and display the active tenant context for all users.

#### Scenario: Regular user sees own tenant context
- **WHEN** a non-PlatformAdmin user is authenticated
- **THEN** the dashboard SHALL display the user's resolved tenant in the header
- **AND** all API calls SHALL include the user's tenant context from the JWT
- **AND** the tenant selector SHALL show only the user's own tenant memberships

#### Scenario: PlatformAdmin selects target tenant
- **WHEN** a PlatformAdmin user selects a different tenant from the searchable tenant selector
- **THEN** all subsequent API calls for tenant-scoped operations SHALL include the `X-Target-Tenant-ID` header with the selected tenant's identifier
- **AND** the dashboard header SHALL display the currently targeted tenant prominently

#### Scenario: PlatformAdmin clears tenant target
- **WHEN** a PlatformAdmin user clears the tenant target selection
- **THEN** the dashboard SHALL revert to the user's default tenant context
- **AND** API calls SHALL no longer include the `X-Target-Tenant-ID` header

### Requirement: System Health Dashboard
The admin dashboard SHALL display real-time system health and operational statistics on the home page.

#### Scenario: Health status display
- **WHEN** a user navigates to the dashboard home page
- **THEN** the dashboard SHALL fetch `GET /health` and `GET /ready` endpoints
- **AND** the dashboard SHALL display the overall system status and per-backend health (PostgreSQL, vector store, Redis)

#### Scenario: Health auto-refresh
- **WHEN** the dashboard home page is displayed
- **THEN** the health status SHALL automatically refresh at a configurable interval (default 30 seconds)
- **AND** status transitions (healthy → degraded, degraded → healthy) SHALL be visually highlighted

#### Scenario: Quick statistics display
- **WHEN** a user views the dashboard home page
- **THEN** the dashboard SHALL display aggregate counts for tenants, users, memories, and knowledge items
- **AND** the dashboard SHALL show a count of pending governance requests with a direct link to the governance page

### Requirement: Tenant Lifecycle Management
The admin dashboard SHALL provide PlatformAdmin users with visual interfaces for tenant lifecycle operations.

#### Scenario: Tenant list display
- **WHEN** a PlatformAdmin user navigates to the tenant management page
- **THEN** the dashboard SHALL display a paginated list of all tenants with name, slug, status, creation date, and domain mappings

#### Scenario: Tenant creation
- **WHEN** a PlatformAdmin user submits the tenant creation form with a valid slug and name
- **THEN** the dashboard SHALL create the tenant via `POST /api/v1/admin/tenants`
- **AND** the new tenant SHALL appear in the tenant list upon successful creation

#### Scenario: Tenant detail management
- **WHEN** a PlatformAdmin user selects a tenant from the list
- **THEN** the dashboard SHALL display a tabbed detail view with tabs for Overview, Domain Mappings, Repository Bindings, Configuration, Secrets, and Git Provider Connections

#### Scenario: Tenant configuration editing
- **WHEN** a user edits tenant configuration fields
- **THEN** the dashboard SHALL display each field with its ownership (Platform vs Tenant) and allow editing of fields the user is authorized to modify
- **AND** secret values SHALL never be displayed — only logical names and references SHALL be shown

### Requirement: Organizational Hierarchy Visualization
The admin dashboard SHALL provide a visual tree representation of the organizational hierarchy within a tenant.

#### Scenario: Hierarchy tree display
- **WHEN** a user navigates to the organization management page
- **THEN** the dashboard SHALL display a collapsible tree view showing Tenant → Organization → Team → Project hierarchy

#### Scenario: Hierarchy unit creation
- **WHEN** a user creates a new organizational unit with a valid name, type, and parent
- **THEN** the dashboard SHALL create the unit via the appropriate API endpoint
- **AND** the tree view SHALL update to reflect the new unit in its correct position

#### Scenario: Member management within unit
- **WHEN** a user selects an organizational unit from the tree
- **THEN** the dashboard SHALL display the unit's members with their roles
- **AND** the dashboard SHALL provide controls to add, remove, and change member roles within that unit's scope

### Requirement: Knowledge Management Interface
The admin dashboard SHALL provide semantic search, browsing, editing, and promotion workflow interfaces for knowledge items.

#### Scenario: Knowledge semantic search
- **WHEN** a user enters a search query on the knowledge management page
- **THEN** the dashboard SHALL execute a semantic search via the knowledge API
- **AND** the dashboard SHALL display results with relevance scores, type badges (ADR, Policy, Pattern, Spec, Hindsight), and layer indicators

#### Scenario: Knowledge item detail display
- **WHEN** a user selects a knowledge item from search results or a list
- **THEN** the dashboard SHALL display the item's rendered markdown content, metadata, type, layer, status, related items, and promotion history

#### Scenario: Knowledge item editing
- **WHEN** a user opens the editor for a knowledge item
- **THEN** the dashboard SHALL provide a markdown editor with live preview, type selector, layer selector, tag management, and metadata editing

#### Scenario: Knowledge promotion workflow
- **WHEN** a user views a knowledge item and initiates or reviews a promotion request
- **THEN** the dashboard SHALL display the promotion request status, mode (Full/Partial), approval chain, and provide approve/reject actions with comment fields

#### Scenario: Knowledge relations visualization
- **WHEN** a user views a knowledge item's relations
- **THEN** the dashboard SHALL display a graph visualization of related items (Specializes, Clarifies, Supersedes, etc.) using a force-directed layout

### Requirement: Memory Management Interface
The admin dashboard SHALL provide search, browse, and feedback interfaces for the memory system.

#### Scenario: Memory semantic search
- **WHEN** a user enters a search query on the memory management page
- **THEN** the dashboard SHALL execute a semantic search via the memory API
- **AND** the dashboard SHALL display results with relevance scores, layer indicators, and importance scores

#### Scenario: Memory layer browsing
- **WHEN** a user selects a memory layer (Agent, User, Session, Project, Team, Org, Tenant)
- **THEN** the dashboard SHALL display a paginated list of memories in that layer with content preview and metadata

#### Scenario: Memory feedback submission
- **WHEN** a user submits feedback for a memory entry (helpful, irrelevant, outdated, inaccurate, duplicate) with a score and optional reasoning
- **THEN** the dashboard SHALL submit the feedback via the memory feedback API
- **AND** the feedback type and score SHALL be reflected in the memory entry's detail view

### Requirement: Governance Workflow Interface
The admin dashboard SHALL provide a visual workflow for reviewing, approving, and rejecting governance requests with full audit trail visibility.

#### Scenario: Pending requests dashboard
- **WHEN** a user navigates to the governance page
- **THEN** the dashboard SHALL display all pending governance requests filterable by type, layer, requestor, and mine-only toggle

#### Scenario: Approval with comment and concurrency control
- **WHEN** an authorized user approves a governance request with an optional comment
- **THEN** the dashboard SHALL submit the approval decision via the governance API with the optimistic concurrency token
- **AND** the request SHALL transition to approved status in the UI

#### Scenario: Governance configuration
- **WHEN** a TenantAdmin edits governance configuration
- **THEN** the dashboard SHALL provide controls for approval mode (Automatic, SingleApprover, MultiApprover), minimum approvers, timeout hours, and escalation contact

#### Scenario: Audit log browsing
- **WHEN** a user navigates to the audit log viewer
- **THEN** the dashboard SHALL display governance events filterable by action, actor, resource type, and timestamp range
- **AND** events SHALL be sorted by timestamp descending by default

### Requirement: Policy Management Interface
The admin dashboard SHALL provide policy creation, visualization, simulation, and drift detection interfaces.

#### Scenario: Policy list with inheritance
- **WHEN** a user navigates to the policy management page
- **THEN** the dashboard SHALL display policies with layer/mode filters and indicate which policies are inherited from parent layers

#### Scenario: Policy creation wizard
- **WHEN** a user creates a new policy
- **THEN** the dashboard SHALL provide a multi-step wizard: select template or describe in natural language, configure rules (target, operator, value, severity), set scope/layer, review and submit

#### Scenario: Policy simulation
- **WHEN** a user simulates a policy against sample data
- **THEN** the dashboard SHALL display per-rule evaluation results (pass/fail/info) with violation details

#### Scenario: Drift detection and remediation
- **WHEN** a user views drift detection results
- **THEN** the dashboard SHALL display drift score, confidence, violation details, and fix suggestions
- **AND** the dashboard SHALL provide a one-click auto-fix action for fixable drift items

### Requirement: Admin Operations Interface
The admin dashboard SHALL provide interfaces for system administration tasks including health monitoring, export/import, and sync operations.

#### Scenario: System health details
- **WHEN** a user navigates to the admin operations page
- **THEN** the dashboard SHALL display per-component health (memory subsystem, knowledge subsystem, policy engine, each storage backend) with connection status and latency

#### Scenario: Export/import management
- **WHEN** a user initiates an export or import operation
- **THEN** the dashboard SHALL provide target selection, format/mode options, job progress tracking, and download/upload functionality

#### Scenario: Sync trigger
- **WHEN** a user triggers an organization sync
- **THEN** the dashboard SHALL initiate the sync via the admin API, display progress, and show last-sync timestamp

### Requirement: Role-Based Navigation
The admin dashboard SHALL conditionally display navigation items and page sections based on the authenticated user's resolved roles.

#### Scenario: PlatformAdmin sees all navigation items
- **WHEN** a PlatformAdmin user is authenticated
- **THEN** the sidebar SHALL display all navigation items including tenant management and platform-level admin operations

#### Scenario: TenantAdmin sees tenant-scoped navigation
- **WHEN** a TenantAdmin user is authenticated
- **THEN** the sidebar SHALL display tenant-scoped navigation items (org hierarchy, users, knowledge, memory, governance, policies)
- **AND** the sidebar SHALL NOT display cross-tenant or platform-level administration items

#### Scenario: Lower-privilege users see limited navigation
- **WHEN** a user with Developer or Viewer role is authenticated
- **THEN** the sidebar SHALL display only the navigation items appropriate to their role
- **AND** create/edit/delete actions SHALL be hidden or disabled for read-only roles

### Requirement: Admin UI Static Asset Serving
The Aeterna server SHALL serve the admin dashboard as static assets at the `/admin` path prefix.

#### Scenario: Static asset request
- **WHEN** a request arrives at `/admin/` or `/admin/assets/*`
- **THEN** the server SHALL serve the corresponding file from the configured admin UI dist directory

#### Scenario: SPA client-side routing fallback
- **WHEN** a request arrives at `/admin/tenants/acme-corp` or any `/admin/*` path that does not match a static file
- **THEN** the server SHALL serve `index.html` from the admin UI dist directory
- **AND** the React Router in the SPA SHALL handle the route client-side

#### Scenario: Admin UI not deployed
- **WHEN** the configured admin UI dist directory does not exist
- **THEN** the server SHALL start successfully without the admin UI route
- **AND** requests to `/admin/*` SHALL return HTTP 404
