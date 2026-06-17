/**
 * Wizard e2e — §12 harden-tenant-provisioning.
 *
 * Pins:
 *   1. Wizard advances through all 5 steps and exposes a YAML preview.
 *   2. Identity-step validation blocks `Next` until slug+name are provided.
 *   3. Submit POSTs to /api/v1/admin/tenants/provision with
 *      `X-Aeterna-Client-Kind: ui` and a structurally valid manifest.
 *   4. The provision response renders per-step results.
 *   5. TenantDetailPage Manifest tab calls GET /tenants/{slug}/manifest
 *      and downloads the YAML.
 */
import { test, expect } from "./fixtures"

test("wizard composes manifest, ships ui header, surfaces step results", async ({ page, login }) => {
  let captured: { url: string; headers: Record<string, string>; body: unknown } | null = null

  await login({
    mocks: [
      { urlPattern: /\/api\/v1\/admin\/tenants$/, method: "GET", body: { tenants: [] } },
      {
        urlPattern: /\/api\/v1\/admin\/tenants\/provision/,
        method: "POST",
        handler: async (route) => {
          captured = {
            url: route.request().url(),
            headers: route.request().headers(),
            body: route.request().postDataJSON(),
          }
          await route.fulfill({
            status: 200,
            contentType: "application/json",
            body: JSON.stringify({
              success: true,
              status: "create",
              steps: [
                { step: "validate", status: "ok" },
                { step: "persist_tenant", status: "ok" },
                { step: "apply_config", status: "ok" },
                { step: "apply_roles", status: "skipped", message: "no roles" },
              ],
            }),
          })
        },
      },
    ],
  })

  await page.goto("/admin/tenants")
  await page.getByRole("button", { name: /create tenant/i }).click()

  // Step 1 — validation gate.
  const next = page.getByTestId("wizard-next")
  await next.click()
  await expect(page.getByTestId("wizard-step-error")).toContainText(/slug is required/i)

  await page.getByLabel("Slug *").fill("acme")
  await page.getByLabel("Display name *").fill("Acme Tenant")
  await next.click()

  // Step 2 — add an env reference.
  await expect(page.getByTestId("wizard-step-secrets")).toBeVisible()
  await page.getByLabel("New reference name").fill("db.password")
  await page.getByLabel("New reference kind").selectOption("env")
  await page.locator('[data-testid="wizard-step-secrets"] button:has-text("Add")').click()
  await page.getByLabel("db.password env variable").fill("DB_PASSWORD")
  await next.click()

  // Step 3 — hierarchy.
  await expect(page.getByTestId("wizard-step-hierarchy")).toBeVisible()
  await page.getByLabel("Unit name").fill("Acme")
  await page.locator('[data-testid="wizard-step-hierarchy"] button:has-text("Add")').click()
  await next.click()

  // Step 4 — roles.
  await expect(page.getByTestId("wizard-step-roles")).toBeVisible()
  await page.getByLabel("User email").fill("admin@acme.test")
  await page.locator('[data-testid="wizard-step-roles"] button:has-text("Add")').click()
  await next.click()

  // Step 5 — providers; just continue to preview.
  await expect(page.getByTestId("wizard-step-providers")).toBeVisible()
  await next.click()

  // Preview — YAML must contain the slug + the secret reference name.
  await expect(page.getByTestId("wizard-preview")).toBeVisible()
  const yaml = await page.getByTestId("wizard-yaml").textContent()
  expect(yaml).toContain("slug: acme")
  expect(yaml).toContain("db.password")
  expect(yaml).toContain("DB_PASSWORD")

  // Submit.
  await page.getByTestId("wizard-submit").click()

  // Result panel + each step rendered.
  await expect(page.getByTestId("wizard-result")).toBeVisible()
  await expect(page.getByTestId("wizard-result-steps")).toContainText("persist_tenant")

  // Wire-contract assertions — §12.8.
  expect(captured).not.toBeNull()
  const cap = captured as unknown as {
    url: string
    headers: Record<string, string>
    body: { tenant: { slug: string }; config?: { secretReferences?: Record<string, unknown> } }
  }
  expect(cap.headers["x-aeterna-client-kind"]).toBe("ui")
  expect(cap.url).not.toContain("allowInline") // no inline ref → no flag
  expect(cap.body.tenant.slug).toBe("acme")
  expect(cap.body.config?.secretReferences?.["db.password"]).toMatchObject({
    logicalName: "db.password",
    kind: "env",
    var: "DB_PASSWORD",
  })
})

test("wizard adds ?allowInline=true when an inline secret is opted in", async ({ page, login }) => {
  let capturedUrl: string | null = null
  await login({
    mocks: [
      { urlPattern: /\/api\/v1\/admin\/tenants$/, method: "GET", body: { tenants: [] } },
      {
        urlPattern: /\/api\/v1\/admin\/tenants\/provision/,
        method: "POST",
        handler: async (route) => {
          capturedUrl = route.request().url()
          await route.fulfill({
            status: 200,
            contentType: "application/json",
            body: JSON.stringify({ success: true, status: "create", steps: [] }),
          })
        },
      },
    ],
  })

  await page.goto("/admin/tenants")
  await page.getByRole("button", { name: /create tenant/i }).click()
  await page.getByLabel("Slug *").fill("betacorp")
  await page.getByLabel("Display name *").fill("Beta")
  await page.getByTestId("wizard-next").click()

  // Add an inline ref.
  await page.getByLabel("New reference name").fill("api.token")
  await page.getByLabel("New reference kind").selectOption("inline")
  await page.locator('[data-testid="wizard-step-secrets"] button:has-text("Add")').click()
  await page.getByLabel("api.token inline value").fill("hunter2")

  // Skip remaining steps.
  for (let i = 0; i < 4; i++) await page.getByTestId("wizard-next").click()

  await page.getByTestId("wizard-allow-inline").check()
  await page.getByTestId("wizard-submit").click()
  await expect(page.getByTestId("wizard-result")).toBeVisible()
  expect(capturedUrl).toContain("allowInline=true")
})

test("manifest tab renders YAML and downloads the file", async ({ page, login }) => {
  await login({
    mocks: [
      {
        urlPattern: /\/api\/v1\/admin\/tenants\/[^/]+$/,
        method: "GET",
        body: {
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
        },
      },
      {
        urlPattern: /\/api\/v1\/admin\/tenants\/acme\/manifest/,
        method: "GET",
        body: {
          apiVersion: "aeterna.io/v1",
          kind: "TenantManifest",
          metadata: { generation: 4 },
          tenant: { slug: "acme", name: "Acme Tenant" },
          config: { fields: { "memory.vectorStore.type": "qdrant" } },
        },
      },
    ],
  })

  await page.goto("/admin/tenants/acme")
  await page.getByRole("button", { name: "Manifest" }).click()
  await page.getByTestId("manifest-fetch-redacted").click()
  const yaml = await page.getByTestId("manifest-yaml").textContent()
  expect(yaml).toContain("slug: acme")
  expect(yaml).toContain("generation: 4")

  const downloadPromise = page.waitForEvent("download")
  await page.getByTestId("manifest-download").click()
  const download = await downloadPromise
  expect(download.suggestedFilename()).toBe("acme.manifest.redacted.yaml")
})
