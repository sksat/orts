import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  timeout: 30000,
  use: { headless: true },
  webServer: [
    {
      command: "npx tsx examples/sine-wave/server.ts",
      port: 9002,
      reuseExistingServer: true,
    },
    {
      command: "npx vite --config vite.example.config.ts --port 5174",
      port: 5174,
      reuseExistingServer: true,
    },
  ],
});
