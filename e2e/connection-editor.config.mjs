import { defineConfig } from "@playwright/test";
import { fileURLToPath } from "node:url";

export default defineConfig({
  testDir: ".",
  testMatch: "connection-editor.spec.mjs",
  outputDir: "../temp/connection-editor-results",
  workers: 1,
  timeout: 120_000,
  use: {
    baseURL: "http://127.0.0.1:1423",
    browserName: "chromium",
    viewport: { width: 900, height: 680 },
  },
  webServer: {
    cwd: fileURLToPath(new URL("..", import.meta.url)),
    command: "pnpm exec vite --host 127.0.0.1 --port 1423 --strictPort",
    url: "http://127.0.0.1:1423/e2e/fixtures/connection-editor.html",
    reuseExistingServer: !process.env.CI,
  },
});
