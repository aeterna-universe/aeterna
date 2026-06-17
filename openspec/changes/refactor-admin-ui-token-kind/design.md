## Context

The server currently treats browser admin sessions and OpenCode plugin sessions as the same token family. That shortcut keeps the auth implementation compact, but it couples browser and plugin revocation, makes the browser appear as a plugin audience in audits, and prevents the admin UI from using browser-appropriate TTLs.

The current runtime does not persist refresh tokens in a PostgreSQL `oauth_refresh_tokens` table; instead it stores refresh tokens in the existing in-memory/Redis refresh-token backend. The implementation therefore needs to split token kinds within the existing runtime store first, while preserving backward compatibility for general authenticated API routes that already accept bearer tokens minted by the plugin flow.

## Goals / Non-Goals

**Goals:**
- Introduce a distinct `admin-ui` token kind and audience for browser sessions.
- Keep plugin-access issuance unchanged for OpenCode plugin flows.
- Make admin UI refresh and revoke operate only on admin-ui refresh tokens.
- Allow protected general API routes to accept either plugin-access or admin-ui access tokens.
- Keep the browser OAuth redirect flow intact while switching the minted browser token family to `admin-ui`.

**Non-Goals:**
- Rebuild auth around a new database-backed refresh-token persistence model in this change.
- Add a brand-new browser login UX; existing `/auth/web/authorize` and callback flow stays.
- Introduce a route-wide audience table for every protected endpoint; the important split here is token minting and refresh/revoke scoping.

## Decisions

### 1. Keep one JWT claims type, add a second accepted kind
The existing token claims model already carries both `aud` and `kind`. Instead of creating a second claims struct, we reuse the same claims shape and add `admin-ui` / `aeterna-admin-ui` constants. Validation for general bearer-authenticated API routes accepts either audience and either supported kind.

**Alternatives considered**
- Separate claims structs for plugin and admin UI: rejected as duplication with no practical gain.
- Leave validation plugin-only and special-case admin UI endpoints only: rejected because the admin UI session endpoint and general API routes need to accept browser tokens.

### 2. Split refresh-token behavior by kind inside the existing refresh store
Refresh tokens are already single-use and shared through the `RefreshTokenStoreBackend`. We extend stored refresh entries with `token_kind` and add kind-aware take/revoke operations. This gives us the audience isolation we need without inventing a second persistence layer.

**Alternatives considered**
- Add the proposed `oauth_refresh_tokens.token_kind` schema immediately: rejected for this step because the running code does not use that table yet.
- Revoke by raw token only and trust callers: rejected because plugin and browser revocation would remain coupled.

### 3. Browser OAuth flow mints admin-ui tokens
The admin UI login page already relies on `/auth/web/authorize` and `/auth/web/callback`. Rather than adding a second browser bootstrap handshake, we keep that redirect flow but change the callback to mint `admin-ui` access and refresh tokens.

**Alternatives considered**
- Add a separate browser bootstrap POST and migrate the UI to exchange a code manually: rejected as extra moving parts for no user-visible benefit.

### 4. Admin UI refresh and revoke get dedicated routes
The browser now calls `/api/v1/auth/admin-ui/refresh` and `/api/v1/auth/admin-ui/revoke`. Plugin flows remain on `/auth/plugin/*`. The server returns `401 wrong_audience` when a refresh token of the wrong kind is presented to one of these routes.

**Alternatives considered**
- Keep shared refresh endpoint and infer kind from the token: rejected because route separation is the explicit contract this change is meant to restore.

## Risks / Trade-offs

- **Legacy browser sessions minted before the split will refresh against the old plugin route** → Mitigation: this change updates the current admin UI client to use the new refresh route; users may need to re-login once after deployment if they still hold an older plugin-kind browser refresh token.
- **General API routes now accept two audiences** → Mitigation: refresh/revoke and minting are still split by kind, which is the actual security boundary needed immediately.
- **Refresh-token persistence is still not database-backed** → Mitigation: retain the current Redis-backed HA path and leave the database migration as a future follow-up if needed.

## Migration Plan

1. Deploy server support for `admin-ui` token kind and dedicated admin-ui refresh/revoke endpoints.
2. Deploy the admin UI so browser OAuth callback sessions and refreshes use the `admin-ui` family.
3. Existing plugin consumers remain on `plugin-access` unchanged.
4. If any browser users hold pre-change refresh tokens, ask them to sign in again once so the new token family is minted.

## Open Questions

- Should the legacy-acceptance flag be implemented at the bearer middleware level for route-specific enforcement, or is kind-aware refresh/revoke enough for the current production posture?
- When the refresh-token store moves to a database table, should `token_kind` become a first-class persisted column there as proposed in the original task list?