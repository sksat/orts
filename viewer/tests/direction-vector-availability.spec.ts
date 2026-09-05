/**
 * What the app offers, over the scene it is actually drawing.
 *
 * A CSV source is the deterministic way to reach the states that decide this, and
 * `orts serve` will not produce them: a spacecraft at the coordinate origin, a
 * run with no epoch, and a spacecraft carrying no attitude at all (the CSV format
 * has no attitude columns). No server is involved — `?noAutoConnect=1` plus the
 * file input, as in `csv-file-load.spec.ts`.
 *
 * The rule under test is that a control's availability matches what the scene
 * draws. Two ways to break it, and both have: offering an arrow the scene drops,
 * and disabling one the scene draws.
 */

import { writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, type Page, test } from "@playwright/test";

/** The single satellite a CSV source yields. */
const SAT = "default";

type CsvPosition = "orbit" | "origin";

/**
 * A CSV run of 20 points.
 *
 * `orbit` is an ordinary circular one; `origin` puts every sample at [0, 0, 0], a
 * finite position the frame accepts and one no bearing can be taken from. `epoch`
 * omits the `# epoch_jd` header, which is what leaves the Sun uncomputable.
 *
 * A non-finite position has no variant here: `parseCSVLine` drops any row with a
 * `NaN` field, so no CSV can put one into app state. That case is a unit test on
 * `drawableAtCentre`, and a library one in `orbit-viewer-public.spec.ts`.
 */
function csvRun(opts: { epoch: boolean; position: CsvPosition }): string {
  const lines: string[] = [
    "# orts 2-body orbit propagation",
    "# mu = 398600.4418 km^3/s^2",
    ...(opts.epoch ? ["# epoch_jd = 2451545.0"] : []),
    "# central_body = earth",
    "# central_body_radius = 6378.137 km",
  ];
  const r = 6778;
  const speed = 7.669;
  for (let i = 0; i < 20; i++) {
    const t = i * 10;
    const angle = (speed / r) * t;
    const vx = -speed * Math.sin(angle);
    const vy = speed * Math.cos(angle);
    const [x, y, z] =
      opts.position === "origin" ? [0, 0, 0] : [r * Math.cos(angle), r * Math.sin(angle), 0];
    lines.push(`${t},${x},${y},${z},${vx},${vy},0,${r},0,0.9,0,0,${angle}`);
  }
  return lines.join("\n");
}

async function loadRun(page: Page, opts: { epoch: boolean; position: CsvPosition }) {
  const path = join(tmpdir(), `orts-availability-${Date.now()}-${Math.random()}.csv`);
  writeFileSync(path, csvRun(opts));
  await page.goto("/?noAutoConnect=1");
  await page.locator('input[type="file"]').setInputFiles(path);
  await expect(page.locator('[data-testid="orbit-info-file"]')).toContainText("points", {
    timeout: 15000,
  });
}

/** Centre the orbit view on the CSV's satellite, which is what draws the arrows. */
async function centreOnSatellite(page: Page) {
  await page.locator('[data-testid="frame-selector-select"]').selectOption(`satellite:${SAT}`);
}

interface Drawn {
  kind: string;
}

/** The arrows the scene has actually put in the graph. */
async function drawnKinds(page: Page): Promise<string[]> {
  return await page.evaluate((id) => {
    const w = window as unknown as {
      __debug_get_direction_vectors?: (satId: string) => Drawn[] | null;
    };
    return (w.__debug_get_direction_vectors?.(id) ?? []).map((v) => v.kind);
  }, SAT);
}

test("a spacecraft at the coordinate origin keeps the Sun, and loses only nadir", async ({
  page,
}) => {
  // The frame refuses a non-finite position and accepts every other, so a centre
  // at the origin is drawn like any other — lit from the Sun's direction, with the
  // Sun arrow along it. Nadir is the one that cannot be taken: it is the bearing
  // from the spacecraft to the body, and there is none from the body's centre.
  await loadRun(page, { epoch: true, position: "origin" });
  await centreOnSatellite(page);

  await expect.poll(() => drawnKinds(page), { timeout: 20000 }).toEqual(["sun"]);

  const sun = page.locator('[data-testid="direction-vector-sun"]');
  const nadir = page.locator('[data-testid="direction-vector-nadir"]');
  await expect(sun, "the Sun is drawn here, so its toggle stays usable").not.toHaveAttribute(
    "aria-disabled",
    "true",
  );
  await expect(nadir).toHaveAttribute("aria-disabled", "true");
  // The reason reaches a reader who cannot hover, and names nadir's own condition
  // rather than the frame's — the position here is finite, and accepted.
  await expect(nadir).toHaveAttribute("aria-label", "Nadir: Requires a non-zero position");
});

test("a run with no epoch offers no Sun, and says why in the control's name", async ({ page }) => {
  // Without an epoch the Sun's direction cannot be computed, and the scene draws
  // no arrow it would have to guess. This position is an ordinary one, so nadir is
  // drawn and stays on offer — which is what makes the Sun's absence the subject.
  await loadRun(page, { epoch: false, position: "orbit" });
  await centreOnSatellite(page);

  await expect.poll(() => drawnKinds(page), { timeout: 20000 }).toEqual(["nadir"]);

  const sun = page.locator('[data-testid="direction-vector-sun"]');
  await expect(sun).toHaveAttribute("aria-disabled", "true");
  await expect(sun).toHaveAttribute("aria-label", "Sun: Requires epoch");
  await expect(page.locator('[data-testid="direction-vector-nadir"]')).not.toHaveAttribute(
    "aria-disabled",
    "true",
  );
});

test("a spacecraft with no attitude is named as that, not as one still arriving", async ({
  page,
}) => {
  // The CSV format carries no attitude columns, so this spacecraft has arrived and
  // has none — the second of the two states that leave the attitude view empty.
  await loadRun(page, { epoch: true, position: "orbit" });
  await page.locator('[data-testid="view-attitude"]').click();

  await expect(page.locator('[data-testid="attitude-no-data"]')).toBeVisible();
  await expect(page.locator('[data-testid="attitude-no-spacecraft"]')).toHaveCount(0);

  // The arrows are drawn at the spacecraft, so an absent one takes both with it —
  // including the Sun, whose own inputs are all present here (this run has an
  // epoch). The controls say the same thing the placeholder does.
  for (const [kind, label] of [
    ["sun", "Sun"],
    ["nadir", "Nadir"],
  ]) {
    const toggle = page.locator(`[data-testid="direction-vector-${kind}"]`);
    await expect(toggle, `${kind} is not drawable with no spacecraft drawn`).toHaveAttribute(
      "aria-disabled",
      "true",
    );
    await expect(toggle).toHaveAttribute("aria-label", `${label}: This spacecraft has no attitude`);
  }
});
