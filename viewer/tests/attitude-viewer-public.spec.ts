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
import { drawnExtentForSpan, NOMINAL_SPACECRAFT_SPAN } from "../src/spacecraftScale.js";

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

interface CameraView {
  position: [number, number, number];
  forward: [number, number, number];
  near: number;
  far: number;
}

/** The rendered camera, after R3F's `lookAt` and the initial fit have moved it. */
async function cameraView(page: Page): Promise<CameraView> {
  const view = await page.evaluate(() => {
    const w = window as unknown as { __debug_get_camera_view?: () => CameraView | null };
    return w.__debug_get_camera_view?.() ?? null;
  });
  if (view == null) throw new Error("no camera hook: the probe did not register");
  return view;
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
 * Wait until the arika WASM has loaded.
 *
 * Before it does, a body-fixed frame falls back to inertial and no body has a
 * Sun direction — the same picture the fallbacks produce on purpose. A test about
 * either has to start from readiness, or it passes for the wrong reason.
 */
async function waitForArika(page: Page) {
  await expect
    .poll(() => page.evaluate(() => window.__fixture_arika_ready?.() ?? false), { timeout: 15000 })
    .toBe(true);
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
    { waitUntil: "load" },
  );
  await expect(page.locator("canvas")).toBeVisible();
  // The epoch means this scene draws the Sun. Waiting for that arrow establishes
  // that the subtree mounted, so the absence below is the attitude being refused
  // and not a scene that never arrived.
  await expectArrows(page, ["sun"]);
  const quat = await page.evaluate((id) => window.__debug_get_sat_world_quat?.(id) ?? null, SAT);
  expect(quat, "no body axes are registered without an attitude").toBeNull();
});

test("body-fixed rotates the spacecraft about the scene's polar axis", async ({ page }) => {
  // The advertised body-fixed frame co-rotates with Earth: R_z(−ERA). Reading the
  // rendered body +X in both frames pins that without the test having to know the
  // angle — a rotation about Z leaves the z component and the length of the xy
  // part alone, and turns the xy part by something other than nothing. Skipping
  // the rotation, or applying it about the wrong axis, breaks one of the three.
  await open(page, `epoch=${EPOCH}`);
  await waitForArika(page);
  await expectArrows(page, ["sun"]);
  const inertial = await bodyAxisInScene(page, 0);

  await open(page, `orientation=bodyFixed&epoch=${EPOCH}`);
  await waitForArika(page);
  await expectArrows(page, ["sun"]);
  const bodyFixed = await bodyAxisInScene(page, 0);

  expect(bodyFixed[2], "a rotation about Z leaves the polar component").toBeCloseTo(inertial[2], 6);
  expect(
    Math.hypot(bodyFixed[0], bodyFixed[1]),
    "and the length of the equatorial part",
  ).toBeCloseTo(Math.hypot(inertial[0], inertial[1]), 6);
  const turned = Math.hypot(bodyFixed[0] - inertial[0], bodyFixed[1] - inertial[1]);
  expect(turned, "but turns it: an unrotated body-fixed frame is just inertial").toBeGreaterThan(
    0.01,
  );
});

test("a body-fixed request for a body the viewer cannot rotate falls back", async ({ page }) => {
  // `earth_rotation_angle` is Earth's angle; using it for Mars would be a wrong
  // picture rather than a missing one. The fallback is inertial, where +90° about
  // Z still puts body +X on scene +Y.
  await open(page, `orientation=bodyFixed&body=mars&epoch=${EPOCH}`);
  // Mars has an ephemeris, so its Sun arrow appearing says the module is loaded:
  // without that wait this would read the pre-WASM inertial fallback and pass
  // even if a loaded Mars were rotated by Earth's angle.
  await waitForArika(page);
  await expectArrows(page, ["sun"]);
  const bodyX = await bodyAxisInScene(page, 0);
  expect(bodyX[1]).toBeCloseTo(1, 2);
});

test("with controls disabled the camera still faces the spacecraft", async ({ page }) => {
  // Reported twice as a defect: with no `OrbitControls` mounted, the offset
  // default camera was said to keep its −Z view direction and leave the origin
  // outside the frustum. Measured here instead of argued: R3F applies
  // `lookAt(0, 0, 0)` to a camera it builds from props, and the initial fit only
  // scales the position along that same ray.
  await open(page, `epoch=${EPOCH}&controls=0`);
  const view = await cameraView(page);
  const distance = Math.hypot(...view.position);
  expect(distance, "the camera sits away from the spacecraft").toBeGreaterThan(1);
  // The camera looks along the ray from its position to the origin.
  const toOrigin = view.position.map((c) => -c / distance);
  const dot =
    toOrigin[0] * view.forward[0] + toOrigin[1] * view.forward[1] + toOrigin[2] * view.forward[2];
  expect(dot).toBeCloseTo(1, 3);
  // And the frustum reaches the spacecraft: a far plane short of the camera's own
  // distance would clip the origin however well the camera is aimed.
  expect(view.far).toBeGreaterThan(distance);
  expect(view.near).toBeLessThan(distance);
});

test("a near plane past the scene pulls the camera back instead of blanking it", async ({
  page,
}) => {
  // `near` reaches the camera from a public prop, chosen without knowing this
  // view's scale. Ten spans is a perfectly good frustum that the default framing
  // — some seven spans out — leaves entirely behind the near plane, so the fit
  // has to clear it as well as fit the viewport.
  const near = 10;
  await open(page, `epoch=${EPOCH}&near=${near}&controls=0`);
  const view = await cameraView(page);
  expect(view.near, "the prop reached the camera").toBeCloseTo(near, 6);

  const distance = Math.hypot(...view.position);
  const extent = drawnExtentForSpan(NOMINAL_SPACECRAFT_SPAN);
  // The whole drawn sphere sits between the planes, so something is on screen.
  expect(distance - extent).toBeGreaterThanOrEqual(near);
  expect(view.far).toBeGreaterThanOrEqual(distance + extent);
});

test("a near plane the scene cannot be resolved against falls back to the default", async ({
  page,
}) => {
  // Past 1e17 spans the drawn extent rounds away against the near plane — adding
  // it changes nothing — so no camera distance can put the spacecraft between
  // the planes, and a position fitted for that plane squares to infinity when
  // its length is taken. The framing reverts rather than compute with it.
  await open(page, `epoch=${EPOCH}&near=1e17&controls=0`);
  const view = await cameraView(page);
  expect(view.near, "the unusable near plane was replaced").toBeLessThan(1);
  const distance = Math.hypot(...view.position);
  expect(Number.isFinite(distance)).toBe(true);
  expect(Number.isFinite(view.far)).toBe(true);
  expect(view.far).toBeGreaterThan(distance);
  expect(view.near).toBeLessThan(distance - drawnExtentForSpan(NOMINAL_SPACECRAFT_SPAN));
});

test("an extreme zoom leaves the camera somewhere Three.js can measure", async ({ page }) => {
  // `zoom: 1e200` builds a finite projection matrix, so the prop guard passes it
  // — and it narrows the effective field of view to 5.3e-199°, which the fit
  // would answer with a distance of 6.5e200 spans. Three.js measures a position
  // by summing its squared components, so that distance overflows the length and
  // the far plane derived from it becomes infinite. The view stays at the framing
  // it was built with instead.
  await open(page, `epoch=${EPOCH}&zoom=1e200&controls=0`);
  const view = await cameraView(page);
  const distance = Math.hypot(...view.position);
  expect(Number.isFinite(distance), `camera at ${view.position}`).toBe(true);
  expect(Number.isFinite(view.far), `far plane ${view.far}`).toBe(true);
  expect(view.far).toBeGreaterThan(distance);
  expect(distance).toBeGreaterThan(0);
});

test("a body with no Sun ephemeris draws no Sun arrow", async ({ page }) => {
  // Uranus has no elements in arika, and `sun_direction_from_body` answers +X
  // there — a guess the view must not draw.
  await open(page, `body=uranus&epoch=${EPOCH}&position=${POSITION}`);
  // In *this* document: the module is loaded, and the Sun arrow is still absent.
  // Checking Earth in a later document would prove nothing about this one, since
  // every body lacks the arrow until arika arrives.
  await waitForArika(page);
  await expectArrows(page, ["nadir"]);

  // Earth, for contrast: the same page does draw the Sun once loaded.
  await open(page, `body=earth&epoch=${EPOCH}&position=${POSITION}`);
  await expectArrows(page, ["nadir", "sun"]);
});
