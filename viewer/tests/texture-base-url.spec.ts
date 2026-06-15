import { expect, test } from "@playwright/test";

// Verify the textureBaseUrl resolution for the static-deployment path (#105).
//
// VITE_TEXTURE_BASE_URL is read at Vite startup and can be set in any mode
// (dev, build, or custom). The expected value is derived from
// process.env.VITE_TEXTURE_BASE_URL so the test remains correct whether or
// not the env var is present in the test environment.
//
// The "env var set" path is covered by unit tests in src/textureBaseUrl.test.ts.

test("textureBaseUrl matches VITE_TEXTURE_BASE_URL when disconnected", async ({ page }) => {
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

  // Inline the same normalization the app uses — avoids importing src code
  // into a Node/Playwright context where .js→.ts rewriting may not apply.
  const rawEnv = process.env.VITE_TEXTURE_BASE_URL?.trim();
  const expected = rawEnv ? (rawEnv.endsWith("/") ? rawEnv : `${rawEnv}/`) : undefined;
  expect(url).toBe(expected);
});
