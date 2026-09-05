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
  await expect
    .poll(() => page.evaluate(() => window.__fixture_arika_ready?.() ?? false), { timeout: 15000 })
    .toBe(true);
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
  await open(page, `sats=nan:${SAT_A}&centre=0&frame=localOrbital&epoch=${EPOCH}`);
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
