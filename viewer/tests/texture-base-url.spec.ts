import { expect, test } from "@playwright/test";

// Verify the textureBaseUrl resolution for the static-deployment path (#105).
//
// The VITE_TEXTURE_BASE_URL env var is a build-time constant (undefined in the
// Vite dev server), so we test two things:
//
// (a) No env var + no WS → textureBaseUrl is undefined (2K bundled textures).
//     Covered here via window.__debug_texture_base_url (DEV-mode only hook).
//
// (b) Env var set + no WS → textureBaseUrl derived from env var.
//     Covered by unit tests in src/textureBaseUrl.test.ts (vi.stubEnv friendly).

test("textureBaseUrl is undefined when disconnected and VITE_TEXTURE_BASE_URL is not set", async ({
  page,
}) => {
  await page.goto("/?noAutoConnect=1");

  // Wait for the React app to mount (canvas appears when Three.js scene initialises).
  await page.waitForSelector("canvas", { timeout: 15000 });

  const url = await page.evaluate(
    () => (window as unknown as Record<string, unknown>).__debug_texture_base_url,
  );
  expect(url).toBeUndefined();
});
