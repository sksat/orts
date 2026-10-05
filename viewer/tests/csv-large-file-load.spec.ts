/**
 * E2E test: a file with more points than a stream's buffers keep is shown whole.
 *
 * A stream's trail keeps 50,000 points (75,000 before it trims) and the live
 * chart buffer 50,000. A file keeps every point it loads (DESIGN.md, the file
 * source policy), so after loading a longer file:
 * - the point count is the file's,
 * - playback rests at the end, replaying rather than following live,
 * - the chart (drawn from DuckDB) starts at the file's first sample,
 * - seeking to the start puts the satellite on the file's first sample.
 */

import { writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";

/** More than the 75,000 points at which a stream's trail trims. */
const NUM_POINTS = 80_001;
const DT = 10;
const LAST_T = (NUM_POINTS - 1) * DT;
const R = 6778; // km

/** A circular orbit, one row per `dt`, starting on the +x axis at t = 0. */
function generateTestCSV(numPoints: number, dt: number): string {
  const lines: string[] = [
    "# orts 2-body orbit propagation",
    "# mu = 398600.4418 km^3/s^2",
    "# epoch_jd = 2451545.0",
    "# central_body = earth",
    "# central_body_radius = 6378.137 km",
  ];
  const v = 7.669; // km/s
  const omega = v / R;
  for (let i = 0; i < numPoints; i++) {
    const t = i * dt;
    const angle = omega * t;
    // t,x,y,z,vx,vy,vz,a,e,inc,raan,omega,nu
    lines.push(
      `${t},${R * Math.cos(angle)},${R * Math.sin(angle)},0,${-v * Math.sin(angle)},${v * Math.cos(angle)},0,${R},0,0.9,0,0,${angle}`,
    );
  }
  return lines.join("\n");
}

test("a CSV longer than a stream's trail is loaded, charted and replayed whole", async ({
  page,
}) => {
  // Writing, parsing and inserting 80,000 rows takes longer than the default.
  test.setTimeout(180_000);

  const csvPath = join(tmpdir(), `orts-large-${Date.now()}.csv`);
  writeFileSync(csvPath, generateTestCSV(NUM_POINTS, DT));

  await page.goto("/?noAutoConnect=1");
  await page.locator('input[type="file"]').setInputFiles(csvPath);

  await expect(page.locator('[data-testid="orbit-info-file"]')).toContainText(
    `${NUM_POINTS} points`,
    { timeout: 120_000 },
  );

  // The count shown with the scene is the trail's, which now holds the file.
  await expect(page.locator('[data-testid="orbit-info-points"]')).toContainText(
    `${NUM_POINTS} points`,
  );

  // Loaded: paused at the end, replaying, with no Live to return to.
  await expect(page.locator('[data-testid="playback-mode"]')).toHaveText("Replay");
  await expect(page.locator('[data-testid="play-pause-btn"]')).toHaveText("Play");
  await expect(page.getByRole("button", { name: "Live" })).toHaveCount(0);

  // The chart comes from DuckDB and spans the file: the query's downsampling
  // keeps each bucket's first row, so its first t is the file's.
  await expect
    .poll(
      () =>
        page.evaluate(() => {
          const data = (window as unknown as Record<string, unknown>).__debug_chart_data as {
            t?: ArrayLike<number>;
          } | null;
          if (!data?.t || data.t.length === 0) return null;
          return [data.t[0], data.t[data.t.length - 1]];
        }),
      { timeout: 60_000 },
    )
    .toEqual([0, LAST_T]);

  // Seek to the start: the satellite is on the file's first sample.
  await page.locator('[data-testid="time-slider"]').fill("0");
  await expect
    .poll(() =>
      page.evaluate(() => {
        const viewer = (window as unknown as Record<string, unknown>).__debug_orbit_viewer as {
          satellite: (id: string) => { t: number; x: number; y: number } | null;
        };
        return viewer.satellite("default");
      }),
    )
    .toMatchObject({ t: 0, x: R, y: 0 });
});
