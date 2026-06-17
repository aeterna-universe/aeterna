## 1. Bootstrap from server session

- [x] 1.1 In `AuthContext.tsx`, extend the session fetch to read `defaultTenantId` and `defaultTenantSlug` from the `/auth/session` response.
- [x] 1.2 On login/reload, if `sessionStorage.activeTenant` is unset and `defaultTenantSlug` is set, seed `activeTenant` with the default.
- [x] 1.3 If neither is set and the user has exactly one membership, auto-select it (mirror server behavior).
- [x] 1.4 Add `defaultTenantSlug` to the `AuthContextValue` interface.

## 2. TenantSelector server persistence

- [x] 2.1 Add `rememberAcrossDevices: boolean` state to `TenantSelector`, default `true`, persisted per-user in `localStorage` under `ui.tenantSelector.remember`.
- [x] 2.2 On tenant change, if `rememberAcrossDevices`, call `PUT /api/v1/user/me/default-tenant { slug }` in addition to updating local state.
- [x] 2.3 Show a small inline checkbox in the dropdown footer: `[✓] Remember across devices`.
- [x] 2.4 On API failure for the PUT (e.g., network), roll back the UI state and show a toast error.
- [x] 2.5 Add PlatformAdmin-only "Clear active tenant" menu item that removes `sessionStorage.activeTenant` and, if `rememberAcrossDevices`, calls `DELETE /api/v1/user/me/default-tenant`.

## 3. select_tenant banner

- [x] 3.1 Create `SelectTenantBanner.tsx` component: renders a sticky top-of-app banner with message, dropdown populated from `availableTenants`, `hint` tooltip, and a `[Select]` button.
- [x] 3.2 In `api/client.ts`, detect `HTTP 400` with `error === "select_tenant"`, store the payload in a global `pendingSelectTenant` signal, and defer the original request promise.
- [x] 3.3 When the banner submits a selection, resolve the deferred request with the chosen tenant applied as `X-Target-Tenant-ID`.
- [x] 3.4 If multiple requests are pending with `select_tenant`, coalesce: one banner, one selection resolves all pending.
- [x] 3.5 The banner respects `rememberAcrossDevices`: when the user selects a tenant there, it also writes `PUT /user/me/default-tenant` if the toggle is on.

## 4. Source label in the top-bar

- [x] 4.1 Compute `tenantSource` in `useTenant()`: `local-session`, `server-default`, `single-membership`, `admin-impersonation`, `explicit-selection`.
- [x] 4.2 Render a muted `<span>` next to the selector showing a short form (`from server default`, `impersonating`, etc).
- [x] 4.3 Tooltip on the label explains the source and how to change it.

## 5. Tests

- [ ] 5.1 Component test: `TenantSelector` with `rememberAcrossDevices=true` calls the PUT endpoint on change.
- [ ] 5.2 Component test: `TenantSelector` with `rememberAcrossDevices=false` does not call PUT.
- [ ] 5.3 Integration (MSW) test: a `GET /user` returning `400 select_tenant` triggers the banner; selecting a tenant retries and succeeds.
- [ ] 5.4 Integration test: PlatformAdmin's "Clear active tenant" removes the header and triggers a platform-scoped page refresh.
- [ ] 5.5 Visual regression (Playwright): screenshot of the banner, of the selector with checkbox, of the source label states.

## 6. Documentation

- [ ] 6.1 Update `admin-ui/README.md` with the TenantSelector persistence behavior.
- [ ] 6.2 Add a note to the user-facing docs describing how "Remember across devices" affects the server-side preference.
