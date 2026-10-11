/**
 * E2E test: a paused single-satellite chart is drawn from DuckDB.
 *
 * While live, a single satellite's chart reads the live buffer; once paused it
 * reads the uneri chart worker's DuckDB query. That worker is the one uneri's
 * build emits as a separate file, so a chart here proves the file the build
 * references is the one the page can load.
 */

import { writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";

const NUM_POINTS = 50;
const DT = 10;

function generateTestCSV(numPoints: number, dt: number): string {
  const lines: string[] = [
    "# orts 2-body orbit propagation",
    "# mu = 398600.4418 km^3/s^2",
    "# epoch_jd = 2451545.0",
    "# central_body = earth",
    "# central_body_radius = 6378.137 km",
  ];
  const r = 6778; // km
  const v = 7.669; // km/s
  const omega = v / r;
  for (let i = 0; i < numPoints; i++) {
    const t = i * dt;
    const angle = omega * t;
    // t,x,y,z,vx,vy,vz,a,e,inc,raan,omega,nu
    lines.push(
      `${t},${r * Math.cos(angle)},${r * Math.sin(angle)},0,${-v * Math.sin(angle)},${v * Math.cos(angle)},0,${r},0,0.9,0,0,${angle}`,
    );
  }
  return lines.join("\n");
}

test("a paused single-satellite chart is drawn from DuckDB", async ({ page }) => {
  const workerErrors: string[] = [];
  page.on("console", (msg) => {
    if (msg.text().includes("Worker error")) workerErrors.push(msg.text());
  });

  const csvPath = join(tmpdir(), `orts-paused-chart-${Date.now()}.csv`);
  writeFileSync(csvPath, generateTestCSV(NUM_POINTS, DT));

  await page.goto("/?noAutoConnect=1");
  await page.locator('input[type="file"]').setInputFiles(csvPath);
  await expect(page.locator('[data-testid="orbit-info-file"]')).toContainText(
    `${NUM_POINTS} points`,
    { timeout: 10_000 },
  );

  // Pause: the chart leaves the live buffer for the DuckDB query.
  await page.locator('[data-testid="play-pause-btn"]').click();

  await expect
    .poll(
      () =>
        page.evaluate(() => {
          const data = (window as unknown as Record<string, unknown>).__debug_chart_data as {
            t?: ArrayLike<number>;
          } | null;
          return data?.t ? [data.t[0], data.t[data.t.length - 1]] : null;
        }),
      { timeout: 30_000 },
    )
    .toEqual([0, (NUM_POINTS - 1) * DT]);
  expect(workerErrors).toEqual([]);
});
