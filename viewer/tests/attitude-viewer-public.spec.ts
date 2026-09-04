/**
 * The published `AttitudeViewer`, mounted on its own.
 *
 * `attitude-view.spec.ts` drives the app; this drives the seam an embedder uses.
 * Between them sits everything the app happens to configure correctly: prop
 * wiring, WASM readiness, the frame fallbacks, and whether the arrows and the
 * spacecraft reach the scene graph at all.
 *
 * Assertions read the *rendered* scene graph through the dev hooks rather than
 * pixels — a frame invariant ("body +X points along scene +Y") is immune to the
 * ±q sign ambiguity, and a few triangles on a dark background make a poor pixel
 * test. See .claude/skills/playwright-viewer-testing.
 */
import { expect, type Page, test } from "@playwright/test";

/** The fixture's own id, which is what the debug registries are keyed by. */
const SAT = "fixture-sat";

/** Epoch used wherever the Sun has to be computable (UTC JD). */
const EPOCH = 2460000.5;

/** Equatorial circular orbit at +X: radial +X, in-track +Y, cross-track +Z. */
const POSITION = "7000,0,0";
const VELOCITY = "0,7.546,0";

type Drawn = { kind: string; direction: [number, number, number]; distance: number };

async function open(page: Page, query: string) {
  await page.goto(`/fixtures/attitude-viewer.html?${query}`, { waitUntil: "load" });
  await expect(page.locator("canvas")).toBeVisible();
  // The spacecraft appears once React has mounted the scene and the group's ref
  // is attached; the hook's presence is what says so.
  await expect
    .poll(
      () =>
        page.evaluate(
          (id) =>
            typeof window.__debug_get_sat_world_quat === "function" &&
            window.__debug_get_sat_world_quat(id) != null,
          SAT,
        ),
      { timeout: 15000 },
    )
    .toBe(true);
}

/** Rendered body axis `axis` in scene coordinates, from the world quaternion. */
async function bodyAxisInScene(page: Page, axis: 0 | 1 | 2) {
  const q = await page.evaluate((id) => window.__debug_get_sat_world_quat?.(id), SAT);
  if (q == null) throw new Error("no rendered attitude");
  const [x, y, z, w] = q;
  const cols: [number, number, number][] = [
    [1 - 2 * (y * y + z * z), 2 * (x * y + z * w), 2 * (x * z - y * w)],
    [2 * (x * y - z * w), 1 - 2 * (x * x + z * z), 2 * (y * z + x * w)],
    [2 * (x * z + y * w), 2 * (y * z - x * w), 1 - 2 * (x * x + y * y)],
  ];
  return cols[axis];
}

async function drawnArrows(page: Page): Promise<Drawn[]> {
  return await page.evaluate(
    (id) => (window.__debug_get_direction_vectors?.(id) ?? []) as Drawn[],
    SAT,
  );
}

/**
 * Wait until exactly these arrow kinds are drawn.
 *
 * The Sun's arrow appears only once the arika WASM has loaded — the scene asks
 * it whether this body has an ephemeris, and the answer before it is ready is
 * "no". So a set of arrows is something to wait for, and asserting an arrow's
 * *absence* means first waiting for the ones that should be there.
 */
async function expectArrows(page: Page, kinds: string[]) {
  await expect
    .poll(async () => (await drawnArrows(page)).map((a) => a.kind).sort(), { timeout: 15000 })
    .toEqual([...kinds].sort());
}

test("inertial: the delivered attitude reaches the scene unchanged", async ({ page }) => {
  // +90° about Z maps body +X onto inertial +Y, and the inertial view's scene
  // axes *are* the inertial axes.
  await open(page, `epoch=${EPOCH}`);
  const bodyX = await bodyAxisInScene(page, 0);
  expect(bodyX[0]).toBeCloseTo(0, 2);
  expect(bodyX[1]).toBeCloseTo(1, 2);
  expect(bodyX[2]).toBeCloseTo(0, 2);
});

test("local orbital: nadir is drawn on scene -Z, whatever the orbit phase", async ({ page }) => {
  // The viewer's basis is [in-track, cross-track, radial], so radial is scene +Z
  // and nadir — its negative — is exactly scene -Z. Nothing about the phase
  // enters, which is the point of asserting it here.
  await open(
    page,
    `orientation=localOrbital&epoch=${EPOCH}&position=${POSITION}&velocity=${VELOCITY}`,
  );
  await expectArrows(page, ["nadir", "sun"]);
  const arrows = await drawnArrows(page);
  const nadir = arrows.find((a) => a.kind === "nadir");
  expect(nadir, "the local-orbital view draws nadir").toBeDefined();
  if (nadir == null) return;
  expect(nadir.direction[0]).toBeCloseTo(0, 2);
  expect(nadir.direction[1]).toBeCloseTo(0, 2);
  expect(nadir.direction[2]).toBeCloseTo(-1, 2);
});

test("no epoch: the Sun arrow is left out rather than guessed", async ({ page }) => {
  // Establish that arrows do reach this scene, so the absence below means the
  // Sun was dropped and not that the whole subtree failed to mount.
  await open(page, `epoch=${EPOCH}&position=${POSITION}`);
  await expectArrows(page, ["nadir", "sun"]);

  await open(page, `position=${POSITION}`);
  await expectArrows(page, ["nadir"]);
});

test("no position: nadir is left out, and the spacecraft still renders", async ({ page }) => {
  await open(page, `epoch=${EPOCH}`);
  await expectArrows(page, ["sun"]);
});

test("an unusable attitude draws no orientation at all", async ({ page }) => {
  // A zero quaternion names no rotation. The body axes are the thing that would
  // claim one, so they must be absent — and the request for an
  // orientation-revealing cube must not be honoured either.
  await page.goto(
    `/fixtures/attitude-viewer.html?epoch=${EPOCH}&attitude=0,0,0,0&shape=axes-cube`,
    {
      waitUntil: "load",
    },
  );
  await expect(page.locator("canvas")).toBeVisible();
  await page.waitForTimeout(2000);
  const quat = await page.evaluate((id) => window.__debug_get_sat_world_quat?.(id) ?? null, SAT);
  expect(quat, "no body axes are registered without an attitude").toBeNull();
});

test("a body-fixed request for a body the viewer cannot rotate falls back", async ({ page }) => {
  // `earth_rotation_angle` is Earth's angle; using it for Mars would be a wrong
  // picture rather than a missing one. The fallback is inertial, where +90° about
  // Z still puts body +X on scene +Y.
  await open(page, `orientation=bodyFixed&body=mars&epoch=${EPOCH}`);
  const bodyX = await bodyAxisInScene(page, 0);
  expect(bodyX[1]).toBeCloseTo(1, 2);
});

test("a body with no Sun ephemeris draws no Sun arrow", async ({ page }) => {
  // Uranus has no elements in arika, and `sun_direction_from_body` answers +X
  // there — a guess the view must not draw.
  await open(page, `body=uranus&epoch=${EPOCH}&position=${POSITION}`);
  await expectArrows(page, ["nadir"]);
  // With Earth as the central body the same page draws the Sun, so the absence
  // above is the ephemeris check and not a scene that failed to mount.
  await open(page, `body=earth&epoch=${EPOCH}&position=${POSITION}`);
  await expectArrows(page, ["nadir", "sun"]);
});
