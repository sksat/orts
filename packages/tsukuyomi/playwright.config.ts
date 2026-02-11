import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  timeout: 60000,
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
    {
      command: "npx tsx examples/mixed-density/server.ts",
      port: 9003,
      reuseExistingServer: true,
    },
    {
      command:
        "npx vite --config vite.mixed-density.config.ts --port 5175",
      port: 5175,
      reuseExistingServer: true,
    },
  ],
});
