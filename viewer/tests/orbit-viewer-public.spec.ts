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
