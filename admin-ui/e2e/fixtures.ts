/**
 * Playwright fixtures for admin-ui e2e tests.
 *
 * Provides a `login()` helper that pre-seeds localStorage with a fake
 * JWT-ish token (so AuthContext hydrates as logged-in) and installs
 * baseline API mocks via page.route(). Individual tests layer extra
 * mocks on top.
 *
 * All tests are hermetic: no real backend is ever contacted.
 */
import { test as base, expect, type Page, type Route } from "@playwright/test"

export interface ApiMock {
  method?: "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
  urlPattern: string | RegExp
  status?: number
  body?: unknown
  /** Optional: fully custom handler if a static status/body isn't enough. */
  handler?: (route: Route) => void | Promise<void>
}

export interface MockedSession {
  userId?: string
  userEmail?: string
  tenants?: Array<{ id: string; slug: string; name: string; status?: string }>
  isPlatformAdmin?: boolean
}

export function defaultSessionBody(session: MockedSession = {}) {
  const tenants = session.tenants ?? [
    { id: "tenant-acme-id", slug: "acme", name: "Acme Tenant", status: "Active" },
  ]
  return {
    user: {
      id: session.userId ?? "user-test",
      email: session.userEmail ?? "test@example.com",
      displayName: "Test User",
    },
    tenants: tenants.map((t) => ({
      id: t.id,
      slug: t.slug,
      name: t.name,
      status: t.status ?? "Active",
      sourceOwner: "Admin",
      createdAt: 1_700_000_000,
      updatedAt: 1_700_000_000,
      deactivatedAt: null,
    })),
    roles: [],
    is_platform_admin: session.isPlatformAdmin ?? true,
  }
}

async function seedAuth(page: Page) {
  await page.addInitScript(() => {
    const now = Math.floor(Date.now() / 1000)
    const tokens = {
      access_token: "e2e-fake-access-token",
      refresh_token: "e2e-fake-refresh-token",
      expires_in: 3600,
      token_type: "Bearer",
      stored_at: now,
    }
    localStorage.setItem("aeterna_tokens", JSON.stringify(tokens))
  })
}

async function installMocks(page: Page, mocks: ApiMock[]) {
  for (const mock of mocks) {
    await page.route(mock.urlPattern, async (route) => {
      if (mock.method && route.request().method() !== mock.method) {
        await route.fallback()
        return
      }
      if (mock.handler) {
        await mock.handler(route)
        return
      }
      await route.fulfill({
        status: mock.status ?? 200,
        contentType: "application/json",
        body: JSON.stringify(mock.body ?? {}),
      })
    })
  }
}

export const test = base.extend<{
  login: (opts?: { session?: MockedSession; mocks?: ApiMock[] }) => Promise<void>
}>({
  login: async ({ page }, use) => {
    await use(async ({ session, mocks } = {}) => {
      await seedAuth(page)
      await installMocks(page, [
        { urlPattern: /\/api\/v1\/auth\/session$/, body: defaultSessionBody(session) },
        ...(mocks ?? []),
      ])
    })
  },
})

export { expect }
