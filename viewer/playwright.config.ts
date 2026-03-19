import { defineConfig } from "@playwright/test";

const VIEWER_PORT = Number(process.env.VIEWER_PORT ?? 15173);

export default defineConfig({
  testDir: "./tests",
  timeout: 60000,
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
