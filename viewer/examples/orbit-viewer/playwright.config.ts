import { defineConfig } from "@playwright/test";

const PORT = Number(process.env.EXAMPLE_PORT ?? 15273);

export default defineConfig({
  testDir: "./tests",
  timeout: 60000,
  // Live-server E2E is inherently subject to transient timing; retry on CI so a
  // single flaky miss doesn't fail the whole job.
  retries: process.env.CI ? 2 : 0,
  use: {
    baseURL: `http://localhost:${PORT}`,
    headless: true,
    launchOptions: {
      args: ["--use-gl=angle", "--use-angle=swiftshader"],
    },
  },
  webServer: {
    command: `vite --port ${PORT} --strictPort`,
    port: PORT,
    reuseExistingServer: !!process.env.CI,
  },
});
