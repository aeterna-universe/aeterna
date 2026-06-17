## 1. Project scaffold and build tooling

- [ ] 1.1 Initialize `admin-ui/` directory with Vite + React 18 + TypeScript template (`npm create vite@latest admin-ui -- --template react-ts`).
- [ ] 1.2 Configure `tsconfig.json` with strict mode, path aliases (`@/` mapping to `src/`), and JSX preserve.
- [ ] 1.3 Install and configure Tailwind CSS v3 with PostCSS in `tailwind.config.ts`.
- [ ] 1.4 Install and initialize shadcn/ui with the default theme and required base components (Button, Card, Input, Select, Dialog, Table, Tabs, Badge, Avatar, DropdownMenu, Sheet, Tooltip, Separator).
- [ ] 1.5 Configure Vite proxy to forward `/api/*`, `/health`, `/ready` to `http://localhost:8080` during development.
- [ ] 1.6 Add `.env.development` and `.env.production` files with `VITE_API_BASE_URL` configuration.
- [ ] 1.7 Add ESLint + Prettier configuration aligned with TypeScript strict rules.
- [ ] 1.8 Add `admin-ui/.gitignore` for `node_modules/`, `dist/`, and `.env.local`.

## 2. API client and authentication

- [ ] 2.1 Create `src/api/client.ts` — typed fetch wrapper that injects `Authorization: Bearer <token>` and `X-Target-Tenant-ID` headers, handles 401 with automatic refresh, and retries once after token refresh.
- [ ] 2.2 Create `src/api/types.ts` — TypeScript type definitions matching key Rust API types (TenantRecord, TenantContext, MemoryEntry, KnowledgeEntry, OrganizationalUnit, Role, RoleIdentifier, GovernanceRequest, Policy, GovernanceEvent, HealthResponse, ReadinessResponse, UserRecord, PolicyRule, DriftResult).
- [ ] 2.3 Create `src/auth/AuthContext.tsx` — React context providing `user`, `tokens`, `login()`, `logout()`, `refresh()`, `isAuthenticated`, `isPlatformAdmin`, `isTenantAdmin`, and `activeTenant`.
- [ ] 2.4 Create `src/auth/LoginPage.tsx` — GitHub OAuth login page that initiates OAuth flow, exchanges GitHub token via `POST /api/v1/auth/plugin/bootstrap`, and stores resulting JWT credentials.
- [ ] 2.5 Create `src/auth/token-manager.ts` — localStorage-based token storage with automatic refresh scheduling (refresh 60s before access token expiry), single-use rotation on refresh, and cleanup on logout.
- [ ] 2.6 Create `src/auth/ProtectedRoute.tsx` — Route wrapper that redirects unauthenticated users to the login page.
- [ ] 2.7 Create `src/auth/RequireRole.tsx` — Component wrapper that conditionally renders children based on role checks (e.g., PlatformAdmin-only sections hidden for other roles).

## 3. Shell layout and navigation

- [ ] 3.1 Create `src/layouts/AdminLayout.tsx` — Main layout with collapsible sidebar, top header bar with user avatar/menu and tenant selector, and main content area with breadcrumbs.
- [ ] 3.2 Create `src/components/Sidebar.tsx` — Navigation sidebar with icons and labels for all 10 page areas, with active-route highlighting and role-based visibility (tenant management hidden for non-PlatformAdmin).
- [ ] 3.3 Create `src/components/TenantSelector.tsx` — Dropdown showing user's tenant memberships for regular users; searchable tenant list for PlatformAdmin with ability to target any tenant via `X-Target-Tenant-ID`.
- [ ] 3.4 Create `src/components/UserMenu.tsx` — Avatar dropdown with user profile info, current role display, logout action, and dark mode toggle.
- [ ] 3.5 Create `src/hooks/useTenant.ts` — Custom hook providing `activeTenantId`, `setActiveTenant()`, and `targetTenantHeader` for API calls.
- [ ] 3.6 Set up React Router v6 with route definitions for all page areas, lazy-loaded route components, and `AdminLayout` as the parent layout route.
- [ ] 3.7 Configure TanStack Query provider with default staleTime, refetchOnWindowFocus, and error/loading defaults.

## 4. Dashboard (home page)

- [ ] 4.1 Create `src/pages/dashboard/DashboardPage.tsx` — Main dashboard page with card grid layout.
- [ ] 4.2 Create `src/pages/dashboard/HealthStatusCard.tsx` — Card fetching `GET /health` and `GET /ready` to display system health with per-backend status indicators (postgres, vector_store, redis).
- [ ] 4.3 Create `src/pages/dashboard/PendingGovernanceCard.tsx` — Card showing count of pending governance requests with link to governance page.
- [ ] 4.4 Create `src/pages/dashboard/RecentAuditCard.tsx` — Card showing the 5 most recent audit events with actor, action, and timestamp.
- [ ] 4.5 Create `src/pages/dashboard/QuickStatsCard.tsx` — Card showing aggregate counts (tenants, users, memories, knowledge items) fetched from list endpoints.
- [ ] 4.6 Add auto-refresh (30s interval) for health and pending governance counts via TanStack Query refetchInterval.

## 5. Tenant management (PlatformAdmin)

- [ ] 5.1 Create `src/pages/tenants/TenantListPage.tsx` — Paginated table of tenants with columns: name, slug, status, created date, domain mappings. Search/filter controls. Create tenant button.
- [ ] 5.2 Create `src/pages/tenants/TenantCreateDialog.tsx` — Modal form for creating a new tenant (slug, name) via `POST /api/v1/admin/tenants`.
- [ ] 5.3 Create `src/pages/tenants/TenantDetailPage.tsx` — Tabbed detail view for a single tenant with sub-tabs: Overview, Domains, Repositories, Configuration, Secrets, Git Providers.
- [ ] 5.4 Create `src/pages/tenants/TenantDomainsTab.tsx` — List and manage domain mappings for the tenant.
- [ ] 5.5 Create `src/pages/tenants/TenantRepositoriesTab.tsx` — Repository binding management (kind, URL, branch, credentials) with validation status display and validate button.
- [ ] 5.6 Create `src/pages/tenants/TenantConfigTab.tsx` — JSON key-value configuration editor showing fields with ownership metadata (Platform vs Tenant), add/update/delete config fields.
- [ ] 5.7 Create `src/pages/tenants/TenantSecretsTab.tsx` — Masked display of tenant secrets (values never shown, only logical names), with add/delete operations.
- [ ] 5.8 Create `src/pages/tenants/TenantGitProvidersTab.tsx` — List platform-level Git provider connections and grant/revoke access for this tenant.

## 6. Organizational hierarchy

- [ ] 6.1 Create `src/pages/organizations/OrgTreePage.tsx` — Visual tree view showing Tenant → Organization → Team → Project hierarchy using a collapsible tree component.
- [ ] 6.2 Create `src/pages/organizations/OrgUnitCreateDialog.tsx` — Dialog for creating org units (type, name, description, parent) with parent selection from the tree.
- [ ] 6.3 Create `src/pages/organizations/OrgUnitDetailPanel.tsx` — Side panel showing org unit details, metadata, and member list with roles.
- [ ] 6.4 Create `src/pages/organizations/MemberManagement.tsx` — Component for adding/removing members and setting roles within an org unit scope.
- [ ] 6.5 Create `src/pages/organizations/OrgUnitEditDialog.tsx` — Dialog for updating org unit name, description, and metadata.

## 7. User and role management

- [ ] 7.1 Create `src/pages/users/UserListPage.tsx` — Paginated user list with search, role filter, org/team filter, and status columns.
- [ ] 7.2 Create `src/pages/users/UserDetailPage.tsx` — User profile card showing identity (GitHub login, email), tenant memberships, and role assignments across all scopes.
- [ ] 7.3 Create `src/pages/users/RoleAssignmentDialog.tsx` — Dialog for assigning a role to a user at a specific scope (Instance, Tenant, Org, Team, Project) with scope selector and role dropdown.
- [ ] 7.4 Create `src/pages/users/EffectivePermissionsView.tsx` — Read-only view showing the computed effective permissions for a user at a given scope.
- [ ] 7.5 Create `src/pages/users/UserInviteDialog.tsx` — Dialog for inviting a user (GitHub login or email) to a tenant/org/team with an initial role.

## 8. Knowledge management

- [ ] 8.1 Create `src/pages/knowledge/KnowledgeSearchPage.tsx` — Search interface with query input, type filter (ADR, Policy, Pattern, Spec, Hindsight), layer filter (Tenant → Project), and paginated results with relevance scores.
- [ ] 8.2 Create `src/pages/knowledge/KnowledgeDetailPage.tsx` — Detail view showing knowledge item content (rendered markdown), metadata, type, layer, status, relations, and promotion history.
- [ ] 8.3 Create `src/pages/knowledge/KnowledgeEditorDialog.tsx` — Markdown editor dialog for creating/editing knowledge items with live preview, type selector, layer selector, and tag management.
- [ ] 8.4 Create `src/pages/knowledge/PromotionWorkflowPanel.tsx` — Panel showing promotion requests for a knowledge item: request → preview → approve/reject with comments. Includes promotion mode selection (Full/Partial).
- [ ] 8.5 Create `src/pages/knowledge/KnowledgeRelationsGraph.tsx` — Graph visualization of knowledge item relations (Specializes, Clarifies, Supersedes, etc.) using a force-directed layout.
- [ ] 8.6 Create `src/pages/knowledge/KnowledgeBatchActions.tsx` — Batch operations toolbar (bulk delete, bulk re-layer) for selected knowledge items.

## 9. Memory management

- [ ] 9.1 Create `src/pages/memory/MemorySearchPage.tsx` — Semantic search interface with query input, layer filter (Agent → Tenant), threshold slider, and results list with relevance scores.
- [ ] 9.2 Create `src/pages/memory/MemoryListPage.tsx` — Memory list by selected layer with pagination and sorting by importance/recency.
- [ ] 9.3 Create `src/pages/memory/MemoryDetailView.tsx` — Detail view showing memory content, metadata, layer, importance score, tags, and feedback history.
- [ ] 9.4 Create `src/pages/memory/MemoryAddDialog.tsx` — Dialog for adding a memory (content, layer, tags, metadata).
- [ ] 9.5 Create `src/pages/memory/MemoryFeedbackPanel.tsx` — Feedback submission interface (helpful/irrelevant/outdated/inaccurate/duplicate) with score slider (-1 to 1) and optional reasoning text.
- [ ] 9.6 Create `src/pages/memory/LayerHierarchyViz.tsx` — Visual representation of the 8-layer memory hierarchy with counts per layer, showing the layer selected for the current view.

## 10. Governance and approvals

- [ ] 10.1 Create `src/pages/governance/PendingRequestsPage.tsx` — Dashboard of pending governance requests with filters (type, layer, requestor, mine-only), sortable table, and approve/reject action buttons.
- [ ] 10.2 Create `src/pages/governance/RequestDetailPage.tsx` — Detail view for a single governance request showing request data, approval chain status, and approve/reject form with comment field and optimistic concurrency token.
- [ ] 10.3 Create `src/pages/governance/GovernanceConfigPage.tsx` — Configuration editor for governance settings (approval mode, min approvers, timeout hours, escalation contact) per scope.
- [ ] 10.4 Create `src/pages/governance/GovernanceRolesPage.tsx` — Manage governance roles (approver, reviewer) assignment per scope/user.
- [ ] 10.5 Create `src/pages/governance/AuditLogPage.tsx` — Filterable audit log viewer with columns (timestamp, actor, action, resource_type, resource_id, details), date range picker, action type filter, and actor search.

## 11. Policy management

- [ ] 11.1 Create `src/pages/policies/PolicyListPage.tsx` — Policy list with layer/mode filters (mandatory/optional), search, and inherited policy indicators.
- [ ] 11.2 Create `src/pages/policies/PolicyCreateWizard.tsx` — Multi-step wizard for creating policies: select template or describe in natural language, configure rules (target, operator, value, severity), set scope/layer, review and submit.
- [ ] 11.3 Create `src/pages/policies/PolicyDetailPage.tsx` — Policy detail view with structured rule editor, metadata, merge strategy display, and constraint violations list.
- [ ] 11.4 Create `src/pages/policies/PolicySimulationPage.tsx` — Interface to simulate a policy against sample data and view per-rule evaluation results (pass/fail/info).
- [ ] 11.5 Create `src/pages/policies/DriftDetectionPage.tsx` — View drift detection results with score, confidence, violation details, fix suggestions, and one-click auto-fix action.

## 12. Admin operations

- [ ] 12.1 Create `src/pages/admin/SystemHealthPage.tsx` — Per-component health dashboard (memory subsystem, knowledge subsystem, policy engine, each storage backend) with connection status, latency indicators, and pool utilization.
- [ ] 12.2 Create `src/pages/admin/ConfigValidationPage.tsx` — Configuration validation interface showing current config values and any validation errors/warnings per section.
- [ ] 12.3 Create `src/pages/admin/MigrationStatusPage.tsx` — Database migration status showing applied migrations, pending migrations, and current schema version.
- [ ] 12.4 Create `src/pages/admin/ExportImportPage.tsx` — UI for triggering export/import jobs (ties to backup-restore change), target selection, format, mode, job progress tracking, and download links.
- [ ] 12.5 Create `src/pages/admin/SyncTriggerPage.tsx` — Interface to trigger GitHub org sync and IDP sync with status display and last-sync timestamp.
- [ ] 12.6 Create `src/pages/admin/TenantProvisioningWizard.tsx` — Wizard for provisioning a tenant from a manifest file (upload YAML/JSON, validate, preview, confirm).

## 13. System settings

- [ ] 13.1 Create `src/pages/settings/ServerConfigPage.tsx` — Read-only view of current server configuration (deployment mode, feature flags, key environment variables — no secrets shown).
- [ ] 13.2 Create `src/pages/settings/LLMProviderPage.tsx` — LLM provider configuration display showing active provider, model selection, and connection status.
- [ ] 13.3 Create `src/pages/settings/EmbeddingProviderPage.tsx` — Embedding provider configuration display showing active provider, model, dimension, and connection status.
- [ ] 13.4 Create `src/pages/settings/StorageStatusPage.tsx` — Storage backend connection health display (PostgreSQL, Qdrant, Redis, DuckDB) with connection pool stats and latency metrics.
- [ ] 13.5 Create `src/pages/settings/ObservabilityPage.tsx` — Links to Prometheus/Grafana dashboards (if configured) and basic metrics display from the metrics endpoint.

## 14. Axum server integration

- [ ] 14.1 Add `fs` feature to `tower-http` dependency in workspace `Cargo.toml`.
- [ ] 14.2 Add `/admin` static asset serving route to `cli/src/server/router.rs` using `tower_http::services::ServeDir` pointing to the admin UI dist directory, with `ServeFile` fallback to `index.html` for SPA routing.
- [ ] 14.3 Place the `/admin` route outside the API auth layer in `build_router()` so static assets are served without JWT validation.
- [ ] 14.4 Add configurable admin UI dist path via `AETERNA_ADMIN_UI_PATH` environment variable (default: `./admin-ui/dist`), with graceful skip if the path does not exist.
- [ ] 14.5 Add `POST /api/v1/auth/admin/session` endpoint that returns user profile, roles across all tenants (including `__root__` PlatformAdmin grants), and tenant memberships in a single response.
- [ ] 14.6 Ensure CORS layer allows `http://localhost:5173` origin during development mode.

## 15. Build integration and CI

- [ ] 15.1 Add `Makefile` target `make admin-ui` that runs `cd admin-ui && npm ci && npm run build` to produce `admin-ui/dist/`.
- [ ] 15.2 Add Dockerfile stage for admin UI build (Node.js image → npm ci → npm run build → copy dist/ to final image).
- [ ] 15.3 Add CI workflow job for admin UI lint, typecheck, and build.
- [ ] 15.4 Update Helm chart to optionally mount admin UI dist as a volume or include it in the container image.

## 16. Testing and polish

- [ ] 16.1 Add Vitest configuration for unit testing React components and hooks.
- [ ] 16.2 Write unit tests for `token-manager.ts` (token storage, refresh scheduling, rotation, cleanup).
- [ ] 16.3 Write unit tests for `AuthContext` (login flow, logout, refresh, PlatformAdmin detection via `__root__` roles).
- [ ] 16.4 Write unit tests for `api/client.ts` (header injection, 401 retry, tenant header injection).
- [ ] 16.5 Write component tests for critical UI components (TenantSelector, Sidebar role-based visibility, ProtectedRoute redirect).
- [ ] 16.6 Add Playwright or Cypress e2e test for login → dashboard → navigate to tenants flow.
- [ ] 16.7 Add loading states, error boundaries, and empty states for all page components.
- [ ] 16.8 Add keyboard navigation and ARIA labels for accessibility on all interactive components.
- [ ] 16.9 Add dark mode support via Tailwind CSS dark variants and theme toggle in UserMenu.
- [ ] 16.10 Performance audit: verify initial bundle size under 200KB gzipped, route-based code splitting working, and no unnecessary re-renders.
