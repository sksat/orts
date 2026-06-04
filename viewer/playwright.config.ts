import { defineConfig } from "@playwright/test";

const VIEWER_PORT = Number(process.env.VIEWER_PORT ?? 15173);

export default defineConfig({
  testDir: "./tests",
  timeout: 60000,
  // Live-server E2E is inherently subject to transient timing; retry on CI so a
  // single flaky miss doesn't fail the whole job. See issue #65.
  retries: process.env.CI ? 2 : 0,
  use: {
    baseURL: `http://localhost:${VIEWER_PORT}`,
    headless: true,
    launchOptions: {
      args: ["--use-gl=angle", "--use-angle=swiftshader"],
    },
  },
  webServer: {
    command: `npx vite --port ${VIEWER_PORT} --strictPort`,
    port: VIEWER_PORT,
    reuseExistingServer: !!process.env.CI,
  },
});
