/**
 * E2E for the embeddable <OrbitViewer> (viewer/src/lib), driven by the standalone
 * example page (examples/orbit-viewer) — no backend.
 *
 * The headline guarantee is performance: a satellite's trail is backed by a
 * *persistent* GPU buffer, so neither advancing `time` nor appending points
 * rebuilds it. Measuring raw FPS is flaky (and skips under swiftshader on CI),
 * so instead we assert the deterministic signal behind FPS: the trail buffer's
 * `generation` only changes on a real reset. A stable generation across a time
 * advance / append means the existing incremental-upload path in OrbitTrail is
 * intact rather than being defeated by full rebuilds.
 */

import { expect, test } from "@playwright/test";

type TrailInfo = { length: number; generation: number } | null;

function readTrail(page: import("@playwright/test").Page): Promise<TrailInfo> {
  return page.evaluate(() => {
    const dbg = (window as unknown as { __debug_orbit_viewer?: { trail(id: string): TrailInfo } })
      .__debug_orbit_viewer;
    return dbg ? dbg.trail("demo") : null;
  });
}

async function waitForTrail(page: import("@playwright/test").Page): Promise<void> {
  await page.waitForFunction(
    () => {
      const dbg = (window as unknown as { __debug_orbit_viewer?: { trail(id: string): TrailInfo } })
        .__debug_orbit_viewer;
      const t = dbg ? dbg.trail("demo") : null;
      return !!t && t.length > 0;
    },
    undefined,
    { timeout: 15000 },
  );
}

test("renders a central body + satellite with a trail", async ({ page }) => {
  await page.goto("/examples/orbit-viewer/");
  await expect(page.locator("canvas").first()).toBeVisible();
  await waitForTrail(page);

  const trail = await readTrail(page);
  expect(trail).not.toBeNull();
  expect(trail?.length).toBeGreaterThan(0);
});

test("advancing time does not rebuild the trail buffer (stable generation)", async ({ page }) => {
  await page.goto("/examples/orbit-viewer/");
  await waitForTrail(page);

  const before = await readTrail(page);
  expect(before).not.toBeNull();

  // Advance the clock several times: only Sun/rotation depend on time.
  for (let i = 0; i < 5; i++) {
    await page.evaluate(() =>
      (window as unknown as { __example: { advanceTime(dt: number): void } }).__example.advanceTime(
        60,
      ),
    );
  }
  await page.waitForTimeout(300);

  const after = await readTrail(page);
  expect(after?.generation).toBe(before?.generation); // no full rebuild
  expect(after?.length).toBe(before?.length); // trail untouched by time
});

test("appending trail points uploads incrementally (grows length, same generation)", async ({
  page,
}) => {
  await page.goto("/examples/orbit-viewer/");
  await waitForTrail(page);

  const before = await readTrail(page);
  expect(before).not.toBeNull();

  await page.evaluate(() =>
    (window as unknown as { __example: { appendTrail(n: number): void } }).__example.appendTrail(
      120,
    ),
  );
  await page.waitForTimeout(300);

  const after = await readTrail(page);
  expect(after?.length ?? 0).toBeGreaterThan(before?.length ?? 0); // points added
  expect(after?.generation).toBe(before?.generation); // append, not rebuild
});

test("local-orbital (LVLH) frame renders without error", async ({ page }) => {
  await page.goto("/examples/orbit-viewer/?frame=lvlh");
  await expect(page.locator("canvas").first()).toBeVisible();
  await waitForTrail(page);
  expect((await readTrail(page))?.length).toBeGreaterThan(0);
});
