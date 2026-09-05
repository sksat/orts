/**
 * The published `OrbitViewer`, in states a live run cannot produce.
 *
 * `orbit-direction-vectors.spec.ts` drives the app against a real `orts serve`,
 * where every satellite shares one clock and every sample is a number. The public
 * props allow neither: `SatelliteState.time` is per satellite, and it can arrive
 * as `NaN` from a source whose parse failed. Both decide the epoch the Sun and
 * the body rotations are evaluated at, so both are driven from a fixture here.
 *
 * Assertions read the rendered scene through the dev hooks rather than pixels.
 * See .claude/skills/playwright-viewer-testing.
 */
import { expect, type Page, test } from "@playwright/test";

/** Epoch used wherever the Sun has to be computable (UTC JD). */
const EPOCH = 2460000.5;

/** Equatorial circular orbit at +X, and a second one a quarter turn along. */
const SAT_A = "7000,0,0:0,7.546,0";
const SAT_B = "0,7000,0:-7.546,0,0";

/**
 * Ninety days in seconds. The Sun moves about a degree a day, so two scenes
 * evaluated this far apart cannot be confused for each other — a difference of
 * minutes would be within the noise of an assertion.
 */
const LATER = 90 * 86400;

type Drawn = { kind: string; direction: [number, number, number]; distance: number };

async function open(page: Page, query: string) {
  await page.goto(`/fixtures/orbit-viewer.html?${query}`, { waitUntil: "load" });
  await expect(page.locator("canvas")).toBeVisible();
  // Only where there is an epoch: the scene loads the arika WASM to evaluate one,
  // and a scene without an epoch never asks for it — so waiting for readiness
  // there waits for something that is not coming.
  if (query.includes("epoch=")) {
    await expect
      .poll(() => page.evaluate(() => window.__fixture_arika_ready?.() ?? false), {
        timeout: 15000,
      })
      .toBe(true);
  }
}

/** The arrows drawn at satellite `index`, once the named kinds are all there. */
async function arrowsAt(page: Page, index: number, kinds: string[]): Promise<Drawn[]> {
  const id = `fixture-sat-${index}`;
  await expect
    .poll(
      async () => {
        const drawn = await page.evaluate(
          (satId) =>
            (
              window as unknown as {
                __debug_get_direction_vectors?: (id: string) => Drawn[] | null;
              }
            ).__debug_get_direction_vectors?.(satId) ?? null,
          id,
        );
        return drawn == null ? null : drawn.map((d) => d.kind).sort();
      },
      { timeout: 15000 },
    )
    .toEqual([...kinds].sort());
  return await page.evaluate(
    (satId) =>
      (
        window as unknown as {
          __debug_get_direction_vectors?: (id: string) => Drawn[] | null;
        }
      ).__debug_get_direction_vectors?.(satId) ?? [],
    id,
  );
}

function sunDirection(arrows: Drawn[]): [number, number, number] {
  const sun = arrows.find((a) => a.kind === "sun");
  if (sun == null) throw new Error("no Sun arrow was drawn");
  return sun.direction;
}

test("the Sun is drawn at the centred satellite's own time", async ({ page }) => {
  // Two satellites at times ninety days apart, centred on the second. The Sun
  // has to be the Sun at *that* satellite's time: the scene used to take
  // whichever time the position Map yielded first, which is the other one.
  await open(page, `sats=0:${SAT_A};${LATER}:${SAT_B}&centre=1&epoch=${EPOCH}&arrows=sun`);
  const centred = sunDirection(await arrowsAt(page, 1, ["sun"]));

  // The same satellite alone, at the same time: the direction to compare with.
  await open(page, `sats=${LATER}:${SAT_B}&centre=0&epoch=${EPOCH}&arrows=sun`);
  const alone = sunDirection(await arrowsAt(page, 0, ["sun"]));

  for (const axis of [0, 1, 2]) {
    expect(
      centred[axis],
      `Sun component ${axis} should match the centred satellite's time`,
    ).toBeCloseTo(alone[axis], 6);
  }

  // And the other satellite's time really does give a different Sun, so the
  // comparison above could have failed.
  await open(page, `sats=0:${SAT_B}&centre=0&epoch=${EPOCH}&arrows=sun`);
  const atZero = sunDirection(await arrowsAt(page, 0, ["sun"]));
  const cosine = atZero[0] * alone[0] + atZero[1] * alone[1] + atZero[2] * alone[2];
  expect(cosine, "ninety days apart the Sun is nowhere near the same place").toBeLessThan(0.5);
});

test("a satellite whose time is not a number still leaves a drawable scene", async ({ page }) => {
  // `SatelliteState.time` reaching the scene as `NaN` is what a failed parse
  // upstream looks like. It decides the epoch of the Earth rotation angle and of
  // the body orientations, and a NaN there renders nothing at all — a group
  // carrying a NaN quaternion disappears.
  await open(page, `sats=nan:${SAT_A}&centre=0&frame=localOrbital&epoch=${EPOCH}&arrows=sun,nadir`);
  const arrows = await arrowsAt(page, 0, ["nadir", "sun"]);
  for (const arrow of arrows) {
    expect(
      arrow.direction.every(Number.isFinite),
      `${arrow.kind} arrow's direction should be finite`,
    ).toBe(true);
    expect(arrow.distance, `${arrow.kind} arrow should have a length`).toBeGreaterThan(0);
  }
});

test("a satellite with no usable time stays in the scene's frame", async ({ page }) => {
  // A body-fixed view rotates everything by the Earth rotation angle at the
  // scene's time. A satellite whose own `time` is `NaN` has to be rotated the
  // same way — dropping the rotation for it would leave that one marker in the
  // inertial frame while the body, the trails and every other satellite are
  // body-fixed, which is a picture of two conventions at once.
  const attitude = "0.7071067811865476,0,0,0.7071067811865476";
  const worldQuat = async () => {
    const q = await page.evaluate(
      (id) => window.__debug_get_sat_world_quat?.(id) ?? null,
      "fixture-sat-0",
    );
    if (q == null) throw new Error("no rendered attitude");
    return q;
  };

  await open(page, `sats=nan:${SAT_A}&frame=bodyFixed&epoch=${EPOCH}&att=${attitude}&arrows=none`);
  await expect.poll(async () => (await worldQuat()) != null, { timeout: 15000 }).toBe(true);
  const withoutTime = await worldQuat();

  // The scene's own time is what it falls back to, and with this one satellite
  // carrying no usable time that is 0 — so a satellite explicitly at 0 has to
  // come out identical.
  await open(page, `sats=0:${SAT_A}&frame=bodyFixed&epoch=${EPOCH}&att=${attitude}&arrows=none`);
  const atZero = await worldQuat();
  for (const i of [0, 1, 2, 3]) {
    expect(withoutTime[i], `quaternion component ${i}`).toBeCloseTo(atZero[i], 6);
  }

  // And the inertial frame gives a different quaternion, so the comparison above
  // is not satisfied by any rotation at all.
  await open(page, `sats=0:${SAT_A}&frame=inertial&epoch=${EPOCH}&att=${attitude}&arrows=none`);
  const inertial = await worldQuat();
  const same = [0, 1, 2, 3].every((i) => Math.abs(inertial[i] - atZero[i]) < 1e-6);
  expect(same, "the body-fixed frame should not equal the inertial one").toBe(false);
});

test("a central-body view is drawn at the scene's time, not a satellite's", async ({ page }) => {
  // With no satellite centred there is nothing for the Sun to be drawn *at*, so
  // the epoch belongs to the scene: `OrbitSceneDataProps.time` drives the
  // lighting and the central body's rotation. The scene used to take whichever
  // time the position Map yielded first instead.
  //
  // The central body's rendered rotation is what reports the epoch the scene
  // chose — a satellite's own rotation would not, since each marker turns by its
  // own sample time.
  const earthQuat = async () => {
    await expect
      .poll(() => page.evaluate(() => window.__debug_get_earth_world_quat?.() != null), {
        timeout: 15000,
      })
      .toBe(true);
    const q = await page.evaluate(() => window.__debug_get_earth_world_quat?.() ?? null);
    if (q == null) throw new Error("no rendered central body");
    return q;
  };

  await open(page, `sats=0:${SAT_A}&epoch=${EPOCH}&t=0&arrows=none`);
  const sceneAtZero = await earthQuat();

  // The satellites are ninety days along while the scene is still at zero. The
  // central body has to be where the scene's time puts it.
  await open(page, `sats=${LATER}:${SAT_A};${LATER}:${SAT_B}&epoch=${EPOCH}&t=0&arrows=none`);
  const satellitesLater = await earthQuat();
  for (const i of [0, 1, 2, 3]) {
    expect(satellitesLater[i], `quaternion component ${i}`).toBeCloseTo(sceneAtZero[i], 6);
  }

  // And the scene's own time does move it, so the comparison could have failed.
  await open(page, `sats=0:${SAT_A}&epoch=${EPOCH}&t=${LATER}&arrows=none`);
  const sceneLater = await earthQuat();
  const same = [0, 1, 2, 3].every((i) => Math.abs(sceneLater[i] - sceneAtZero[i]) < 1e-6);
  expect(same, "ninety days of rotation should not leave the body where it was").toBe(false);
});

test("an embedder who says nothing about the arrows gets none", async ({ page }) => {
  // `directionVectors` is opt-in: the prop is omitted here, which is what every
  // embedder written before it existed passes. The centred satellite is the one
  // case that *would* draw them, so this is where the default has to hold.
  // The attitude is here to make the spacecraft register its own hook, which is
  // the only way to know the subtree mounted before reading an absence.
  await open(page, `sats=0:${SAT_A}&centre=0&frame=localOrbital&epoch=${EPOCH}&att=1,0,0,0`);

  // The scene has to be up before absence means anything: the spacecraft's own
  // hook says so, and it is registered by the same subtree that would carry the
  // arrows.
  await expect
    .poll(
      () => page.evaluate((id) => window.__debug_get_sat_world_quat?.(id) != null, "fixture-sat-0"),
      { timeout: 15000 },
    )
    .toBe(true);

  const drawn = await page.evaluate(
    (satId) =>
      (
        window as unknown as {
          __debug_get_direction_vectors?: (id: string) => unknown[] | null;
        }
      ).__debug_get_direction_vectors?.(satId) ?? null,
    "fixture-sat-0",
  );
  // Null rather than an empty list: with nothing to draw the component is not
  // mounted, so it registers no hook at all.
  expect(drawn, "no arrows are registered when the prop is left out").toBeNull();
});

test("the Sun arrow is left out where the direction would be a guess", async ({ page }) => {
  // `sun_direction_from_body` answers +X — the vernal equinox — for a body it
  // cannot place, and with no epoch there is nothing to evaluate at all. The
  // lighting keeps that fallback so a 3D model is not left black; the *arrow*
  // has to be dropped, or the picture shows a guess as a measurement.
  //
  // Nadir is the control: it needs only a position, so its presence says the
  // arrows reached this scene and the Sun's absence is a decision.
  await open(page, `sats=0:${SAT_A}&centre=0&frame=localOrbital&arrows=sun,nadir`);
  await arrowsAt(page, 0, ["nadir"]);

  // Uranus has no elements in arika, so the Sun cannot be placed relative to it
  // even with an epoch. Its radius is given because the scene has none to look up.
  await open(
    page,
    `sats=0:${SAT_A}&centre=0&frame=localOrbital&epoch=${EPOCH}&body=uranus&radius=25559&arrows=sun,nadir`,
  );
  await arrowsAt(page, 0, ["nadir"]);

  // And Earth with the same epoch does draw it, so the two above are not passing
  // for want of a Sun arrow anywhere.
  await open(page, `sats=0:${SAT_A}&centre=0&frame=localOrbital&epoch=${EPOCH}&arrows=sun,nadir`);
  await arrowsAt(page, 0, ["nadir", "sun"]);
});
