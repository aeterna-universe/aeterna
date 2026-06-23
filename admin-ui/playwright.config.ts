import { defineConfig, devices } from "@playwright/test"

/**
 * Playwright config for admin-ui e2e tests.
 *
 * Strategy: hermetic tests.
 *   - `webServer` below boots Vite dev server on port 5173.
 *   - Tests do NOT hit a real backend. Every network call is intercepted
 *     with `page.route()` and answered by the test itself. This keeps the
 *     suite fast, deterministic, and runnable in CI without any infra.
 */
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 2 : undefined,
  reporter: process.env.CI ? [["github"], ["html", { open: "never" }]] : "list",
  timeout: 30_000,

  use: {
    baseURL: "http://localhost:5173",
    trace: "on-first-retry",
    screenshot: "only-on-failure",
    video: "retain-on-failure",
  },

  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],

  webServer: {
    command: "npm run dev -- --port 5173 --strictPort",
    url: "http://localhost:5173",
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
    stdout: "pipe",
    stderr: "pipe",
  },
})
