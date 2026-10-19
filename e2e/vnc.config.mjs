import { defineConfig } from "@playwright/test";
import { fileURLToPath } from "node:url";

export default defineConfig({
  testDir: ".",
  testMatch: "vnc.spec.mjs",
  outputDir: "../temp/vnc-results",
  workers: 1,
  timeout: 60_000,
  use: { baseURL: "http://127.0.0.1:1424", browserName: "chromium" },
  webServer: {
    cwd: fileURLToPath(new URL("..", import.meta.url)),
    command: "pnpm exec vite --host 127.0.0.1 --port 1424 --strictPort",
    url: "http://127.0.0.1:1424/e2e/fixtures/vnc.html",
    reuseExistingServer: !process.env.CI,
  },
});
