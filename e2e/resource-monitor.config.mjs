import { defineConfig } from "@playwright/test";
import { fileURLToPath } from "node:url";

// Layout checks need real CSS geometry, but do not need SSH or the Web backend.
export default defineConfig({
  testDir: ".",
  testMatch: "resource-monitor.spec.mjs",
  outputDir: "../temp/resource-monitor-results",
  workers: 1,
  timeout: 90_000,
  use: {
    baseURL: "http://127.0.0.1:1422",
    browserName: "chromium",
    viewport: { width: 800, height: 600 },
  },
  webServer: {
    cwd: fileURLToPath(new URL("..", import.meta.url)),
    command: "pnpm exec vite --host 127.0.0.1 --port 1422 --strictPort",
    url: "http://127.0.0.1:1422/e2e/fixtures/resource-monitor.html",
    reuseExistingServer: !process.env.CI,
  },
});
