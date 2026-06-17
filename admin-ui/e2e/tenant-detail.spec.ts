/**
 * Regression test for #85 — TenantDetailPage envelope unwrap.
 *
 * The bug: `show_tenant` returns { success, tenant: {...} } but the page
 * used `useQuery<TenantRecord>` directly, so `tenant.slug` was undefined
 * and sub-tabs fetched `/api/v1/admin/tenants/undefined/*`. This test
 * pins the correct behavior by asserting that:
 *
 *   1. No request is ever made to a URL containing `/tenants/undefined/`.
 *   2. When the Config tab is opened, the request URL contains the real
 *      slug from the envelope (`acme`).
 */
import { test, expect } from "./fixtures"

const TENANT_ENVELOPE = {
  success: true,
  tenant: {
    id: "tenant-acme-id",
    slug: "acme",
    name: "Acme Tenant",
    status: "Active",
    sourceOwner: "Admin",
    createdAt: 1_700_000_000,
    updatedAt: 1_700_000_000,
    deactivatedAt: null,
  },
}

test("TenantDetailPage unwraps the envelope and never requests /tenants/undefined/*", async ({
  page,
  login,
}) => {
  // Track every request so we can assert URL hygiene at the end.
  const requests: string[] = []
  page.on("request", (req) => requests.push(req.url()))

  await login({
    mocks: [
      // GET /api/v1/admin/tenants/:id → envelope
      {
        urlPattern: /\/api\/v1\/admin\/tenants\/[^/]+$/,
        method: "GET",
        body: TENANT_ENVELOPE,
      },
      // Config tab
      {
        urlPattern: /\/api\/v1\/admin\/tenants\/[^/]+\/config$/,
        method: "GET",
        body: { "memory.vectorStore.type": "qdrant" },
      },
      // Providers tab (may be prefetched by tab bar)
      {
        urlPattern: /\/api\/v1\/admin\/tenants\/[^/]+\/providers$/,
        method: "GET",
        body: [],
      },
      // Repository-binding tab
      {
        urlPattern: /\/api\/v1\/admin\/tenants\/[^/]+\/repository-binding$/,
        method: "GET",
        body: { binding: null },
      },
    ],
  })

  await page.goto("/admin/tenants/acme")

  // Overview content proves the envelope was unwrapped correctly: if the
  // unwrap had failed, tenant.name/tenant.slug would render as empty and
  // these text nodes would not be on the page at all.
  await expect(page.getByRole("heading", { name: "Acme Tenant" })).toBeVisible({
    timeout: 10_000,
  })
  // The slug appears in the overview definition list.
  await expect(
    page.getByRole("definition").filter({ hasText: /^acme$/ }),
  ).toBeVisible()

  // Open the Config tab, which is the one that triggered the undefined bug.
  await page.getByRole("button", { name: /^config$/i }).click()

  // Wait for the config fixture content to render (proves the tab
  // successfully fetched AND parsed the response).
  await expect(page.getByText("memory.vectorStore.type")).toBeVisible({
    timeout: 10_000,
  })

  // HARD assertion: no request for the literal "undefined" path.
  const undefinedCalls = requests.filter((u) => u.includes("/tenants/undefined/"))
  expect(undefinedCalls, `Unexpected requests: ${undefinedCalls.join(", ")}`).toEqual([])

  // Soft assertion: at least one correctly-slugged sub-tab call exists.
  const slugCalls = requests.filter((u) => /\/tenants\/acme\/config(\?|$)/.test(u))
  expect(slugCalls.length).toBeGreaterThan(0)
})
