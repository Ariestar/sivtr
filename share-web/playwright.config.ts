import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "e2e",
  use: { baseURL: "http://127.0.0.1:8787", trace: "retain-on-failure" },
  webServer: { command: "wrangler dev --local --port 8787", port: 8787, reuseExistingServer: true },
});
