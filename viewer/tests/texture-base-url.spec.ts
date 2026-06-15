import { expect, test } from "@playwright/test";

// Verify the textureBaseUrl resolution for the static-deployment path (#105).
//
// VITE_TEXTURE_BASE_URL is read at Vite startup: it is defined only when the
// env var is set before `vite` (or `vite build`) runs. In the default test
// environment the env var is absent, so the var resolves to undefined.
//
// (a) No env var + no WS → textureBaseUrl is undefined (2K bundled textures).
//     Covered here via window.__debug_texture_base_url (DEV-mode only hook).
//
// (b) Env var set + no WS → textureBaseUrl derived from env var.
//     Covered by unit tests in src/textureBaseUrl.test.ts.

test("textureBaseUrl is undefined when disconnected and VITE_TEXTURE_BASE_URL is not set", async ({
  page,
}) => {
  await page.goto("/?noAutoConnect=1");

  // Wait for the React app to mount (canvas appears when Three.js scene initialises).
  await page.waitForSelector("canvas", { timeout: 15000 });

  // The debug hook is installed by a useEffect; wait for it to appear so we
  // don't assert before the effect has run (a missing key and undefined both
  // evaluate to undefined, which would cause a false positive).
  await page.waitForFunction(
    () => "__debug_texture_base_url" in (window as unknown as Record<string, unknown>),
    { timeout: 5000 },
  );

  const url = await page.evaluate(
    () => (window as unknown as Record<string, unknown>).__debug_texture_base_url,
  );
  expect(url).toBeUndefined();
});
