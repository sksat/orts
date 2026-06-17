/**
 * E2E for the embeddable <OrbitViewer> as a *registry consumer*: this example is
 * a standalone workspace package that imports the shadcn-distributed source
 * (`shadcn add`-copied into `components/orbit-viewer/`), not viewer/src/lib, and
 * runs on its own Vite — so this also proves the registry output is consumable.
 * No backend.
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
  await page.goto("/");
  await expect(page.locator("canvas").first()).toBeVisible();
  await waitForTrail(page);

  const trail = await readTrail(page);
  expect(trail).not.toBeNull();
  expect(trail?.length).toBeGreaterThan(0);
});

test("advancing time does not rebuild the trail buffer (stable generation)", async ({ page }) => {
  await page.goto("/?animate=0"); // freeze so only the hook mutates state
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
  await page.goto("/?animate=0"); // freeze so only the hook mutates state
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
  await page.goto("/?frame=lvlh");
  await expect(page.locator("canvas").first()).toBeVisible();
  await waitForTrail(page);
  expect((await readTrail(page))?.length).toBeGreaterThan(0);
});

test("satellite-centred inertial and LVLH are distinct frames (#90)", async ({ page }) => {
  type SceneFrame = {
    lvlhActive: boolean;
    cameraTracking: boolean;
    originPosition: [number, number, number] | null;
  };
  // The scene mounts inside the r3f reconciler, whose effects can flush after
  // the page-level ones — wait for the global rather than reading immediately.
  const readFrame = async () => {
    await page.waitForFunction(
      () => (window as unknown as { __debug_scene_frame?: unknown }).__debug_scene_frame != null,
      undefined,
      { timeout: 10000 },
    );
    return page.evaluate(
      () => (window as unknown as { __debug_scene_frame?: SceneFrame }).__debug_scene_frame ?? null,
    );
  };

  // LVLH: data is transformed into the orbit frame; the camera stays put.
  await page.goto("/?frame=lvlh&animate=0");
  await waitForTrail(page);
  const lvlh = await readFrame();
  expect(lvlh?.lvlhActive).toBe(true);
  expect(lvlh?.cameraTracking).toBe(false);

  // Inertial: satellite is still centred, but axes stay star-fixed —
  // no LVLH data transform and no camera co-rotation.
  await page.goto("/?frame=sat&animate=0");
  await waitForTrail(page);
  const inertial = await readFrame();
  expect(inertial?.originPosition).not.toBeNull();
  expect(inertial?.lvlhActive).toBe(false);
  expect(inertial?.cameraTracking).toBe(false);
});

test("renders a custom central body (bodies prop, radiusKm from the definition)", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  // ?body=custom centres on a user-defined body with no built-in entry and an
  // omitted centralBody.radiusKm (resolved from the definition).
  await page.goto("/?body=custom&animate=0");
  await expect(page.locator("canvas").first()).toBeVisible();
  await waitForTrail(page); // scene composes around the custom body without error
  expect((await readTrail(page))?.length ?? 0).toBeGreaterThan(0);
  expect(errors).toEqual([]);
});

test("a marker-only satellite (no trail prop) gets no trail buffer", async ({ page }) => {
  await page.goto("/?animate=0");
  await waitForTrail(page); // scene is up: the demo satellite's trail is filled

  const markerTrail = await page.evaluate(() => {
    const dbg = (window as unknown as { __debug_orbit_viewer?: { trail(id: string): TrailInfo } })
      .__debug_orbit_viewer;
    return dbg ? dbg.trail("marker") : null;
  });
  expect(markerTrail).toBeNull(); // no buffer allocated, no OrbitTrail mounted
});
