import { defineConfig, devices } from "@playwright/test"

/**
 * Playwright config for admin-ui e2e tests.
 *
 * Two modes:
 *
 * 1. Hermetic (default): `webServer` boots Vite dev server on port 5173.
 *    Tests intercept every network call with `page.route()`. Fast, deterministic,
 *    no backend needed. This is what runs in admin-ui-e2e.yml on every PR.
 *
 * 2. Real backend: set E2E_BASE_URL to a live Aeterna server (e.g.
 *    http://localhost:8080). Tests hit the real API. Used by the system
 *    integration workflow (e2e-system.yml) to verify the UI + API together.
 *
 *    E2E_BASE_URL=http://localhost:8080 npx playwright test --project=real-backend
 */
const realBackendUrl = process.env.E2E_BASE_URL
const useRealBackend = !!realBackendUrl

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: !useRealBackend,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? (useRealBackend ? 1 : 2) : undefined,
  reporter: process.env.CI ? [["github"], ["html", { open: "never" }]] : "list",
  timeout: 30_000,

  use: {
    baseURL: useRealBackend ? realBackendUrl : "http://localhost:5173",
    trace: "on-first-retry",
    screenshot: "only-on-failure",
    video: "retain-on-failure",
  },

  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
    // Real-backend project — skips the Vite webServer, hits E2E_BASE_URL directly.
    // Enable by setting E2E_BASE_URL and running: --project=real-backend
    ...(useRealBackend
      ? [{ name: "real-backend", use: { ...devices["Desktop Chrome"] } }]
      : []),
  ],

  // Only start the Vite dev server in hermetic mode.
  ...(useRealBackend
    ? {}
    : {
        webServer: {
          command: "npm run dev -- --port 5173 --strictPort",
          url: "http://localhost:5173",
          reuseExistingServer: !process.env.CI,
          timeout: 120_000,
          stdout: "pipe",
          stderr: "pipe",
        },
      }),
})
