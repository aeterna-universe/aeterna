/**
 * §13.4 — UI runner for the tenant-provisioning consistency suite.
 *
 * What this is:
 *   The browser-shaped sibling of `runner_api` (§13.2) and `runner_cli`
 *   (§13.3). It loops over every fixture in
 *   `tests/tenant_provisioning/scenarios/` and asserts that, when the
 *   server returns that manifest from `GET /admin/tenants/{slug}/manifest`,
 *   the Admin UI's Manifest tab:
 *
 *     1. renders YAML that round-trips every structural marker of the
 *        fixture (slug, name, generation, scenario label, every
 *        `config.fields` key, every `config.secretReferences` key, and
 *        every top-level `hierarchy[*].name`),
 *     2. downloads as `<slug>.manifest.redacted.yaml`.
 *
 * What this is NOT:
 *   It does not exercise the full create-tenant wizard for every
 *   fixture. The wizard's wire shape is already pinned by
 *   `tenant-wizard.spec.ts` (§12). The matrix-runner role here is
 *   render-side: the same invariant the API and CLI runners enforce
 *   server-side (`render(apply(M)) ≈ M` modulo allowlist) is enforced
 *   here for the UI's YAML serializer (`manifestToYaml`), which the
 *   §13.2/§13.3 runners cannot reach because they consume JSON.
 *
 * Hermetic: no network, no backend, no Docker. Vite dev server only.
 */
import { readFileSync, readdirSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import { test, expect } from "./fixtures"

// Playwright runs specs as ESM, so `__dirname` is unbound. Resolve it
// from `import.meta.url` once at module load.
const HERE = dirname(fileURLToPath(import.meta.url))
const SCENARIO_DIR = join(HERE, "..", "..", "tests", "tenant_provisioning", "scenarios")

interface ManifestFixture {
  apiVersion: string
  kind: string
  metadata: { generation?: number; labels?: Record<string, string> }
  tenant: { slug: string; name: string }
  config?: {
    fields?: Record<string, unknown>
    secretReferences?: Record<string, unknown>
  }
  hierarchy?: Array<{ name: string }>
}

interface LoadedFixture {
  fileName: string
  manifest: ManifestFixture
}

function loadFixtures(): LoadedFixture[] {
  return readdirSync(SCENARIO_DIR)
    .filter((f) => f.endsWith(".json"))
    .sort()
    .map((fileName) => ({
      fileName,
      manifest: JSON.parse(readFileSync(join(SCENARIO_DIR, fileName), "utf8")) as ManifestFixture,
    }))
}

/**
 * Mirror of the quoting predicate in
 * `admin-ui/src/api/tenant-manifest.ts::manifestToYaml`. Kept inline
 * (not imported) so a regression in the renderer cannot accidentally
 * silence the assertion by changing both sides at once.
 */
function yamlScalar(v: string): string {
  if (v === "" || /[:#\n\-\{\}\[\]&*!|>%@`,]/.test(v) || /^\s|\s$/.test(v)) {
    return JSON.stringify(v)
  }
  return v
}

const FIXTURES = loadFixtures()

test.describe("§13.4 ui runner — consistency matrix", () => {
  // Sanity: the harness must actually find fixtures; a silent zero-loop
  // would make the whole runner trivially pass.
  test("fixture set is non-empty and matches §13.1", () => {
    expect(FIXTURES.length).toBeGreaterThanOrEqual(5)
    const names = FIXTURES.map((f) => f.fileName)
    expect(names).toEqual(
      expect.arrayContaining([
        "01-bootstrap.json",
        "02-add-tenant-root.json",
        "03-rotate-reference.json",
        "04-noop-reapply.json",
        "05-prune.json",
      ]),
    )
  })

  for (const { fileName, manifest } of FIXTURES) {
    test(`renders + round-trips ${fileName}`, async ({ page, login }) => {
      const slug = manifest.tenant.slug
      const displayName = manifest.tenant.name

      await login({
        mocks: [
          // Tenant summary envelope — TenantDetailPage requires this
          // before any tab is rendered.
          {
            urlPattern: new RegExp(`/api/v1/admin/tenants/${slug}$`),
            method: "GET",
            body: {
              success: true,
              tenant: {
                id: `tenant-${slug}-id`,
                slug,
                name: displayName,
                status: "Active",
                sourceOwner: "Admin",
                createdAt: 1_700_000_000,
                updatedAt: 1_700_000_000,
                deactivatedAt: null,
              },
            },
          },
          // Manifest endpoint returns the fixture verbatim. This is
          // the "render" half of the round-trip invariant: the server
          // has already produced this manifest; the UI must not mangle
          // it.
          {
            urlPattern: new RegExp(`/api/v1/admin/tenants/${slug}/manifest`),
            method: "GET",
            body: manifest,
          },
        ],
      })

      await page.goto(`/admin/tenants/${slug}`)
      await page.getByRole("button", { name: "Manifest" }).click()
      await page.getByTestId("manifest-fetch-redacted").click()

      const yaml = await page.getByTestId("manifest-yaml").textContent()
      expect(yaml).not.toBeNull()
      const text = yaml as string

      // Identity markers — every fixture carries these.
      expect(text).toContain(`slug: ${yamlScalar(slug)}`)
      expect(text).toContain(`name: ${yamlScalar(displayName)}`)
      if (manifest.metadata.generation !== undefined) {
        expect(text).toContain(`generation: ${manifest.metadata.generation}`)
      }
      const scenarioLabel = manifest.metadata.labels?.scenario
      if (scenarioLabel) {
        expect(text).toContain(`scenario: ${yamlScalar(scenarioLabel)}`)
      }

      // Scenario-specific markers — every config key, every secret
      // reference logical name, and every top-level hierarchy unit
      // name must survive YAML serialization.
      for (const fieldKey of Object.keys(manifest.config?.fields ?? {})) {
        expect(text, `config.fields.${fieldKey} missing from rendered YAML`).toContain(fieldKey)
      }
      for (const refKey of Object.keys(manifest.config?.secretReferences ?? {})) {
        expect(text, `config.secretReferences.${refKey} missing from rendered YAML`).toContain(
          refKey,
        )
      }
      for (const unit of manifest.hierarchy ?? []) {
        expect(text, `hierarchy unit "${unit.name}" missing from rendered YAML`).toContain(
          unit.name,
        )
      }

      // Download filename pins the redacted-suffix contract documented
      // in §14.3 (security appendix).
      const downloadPromise = page.waitForEvent("download")
      await page.getByTestId("manifest-download").click()
      const dl = await downloadPromise
      expect(dl.suggestedFilename()).toBe(`${slug}.manifest.redacted.yaml`)
    })
  }
})
