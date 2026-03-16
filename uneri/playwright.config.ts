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
      command: "npx vite --config vite.mixed-density.config.ts --port 5175",
      port: 5175,
      reuseExistingServer: true,
    },
    {
      command: "npx tsx examples/multi-series/server.ts",
      port: 9004,
      reuseExistingServer: true,
    },
    {
      command: "npx vite --config vite.multi-series.config.ts --port 5176",
      port: 5176,
      reuseExistingServer: true,
    },
    {
      command: "npx tsx examples/multi-table/server.ts",
      port: 9005,
      reuseExistingServer: true,
    },
    {
      command: "npx vite --config vite.multi-table.config.ts --port 5177",
      port: 5177,
      reuseExistingServer: true,
    },
  ],
});
