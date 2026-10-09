/**
 * E2E test: the large-file warning stays with the file it is about.
 *
 * The real threshold is a million points; a dev build lets the test lower it
 * (`__debug_large_file_points`) so a small CSV stands in for a large one.
 * - a file at the threshold is warned about,
 * - picking a CSV with no data leaves the loaded file in place, and its warning,
 * - a file below the threshold replaces both.
 */

import { writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, type Page, test } from "@playwright/test";

const THRESHOLD = 40;

function writeCSV(name: string, numPoints: number): string {
  const lines = ["# mu = 398600.4418 km^3/s^2", "# epoch_jd = 2451545.0"];
  for (let i = 0; i < numPoints; i++) {
    // t,x,y,z,vx,vy,vz,a,e,inc,raan,omega,nu
    lines.push(`${i * 10},6778,0,0,0,7.669,0,6778,0,0.9,0,0,0`);
  }
  const path = join(tmpdir(), `orts-${name}-${Date.now()}.csv`);
  writeFileSync(path, lines.join("\n"));
  return path;
}

async function load(page: Page, path: string): Promise<void> {
  await page.locator('input[type="file"]').setInputFiles(path);
}

test("the large-file warning stays with the file it is about", async ({ page }) => {
  await page.goto("/?noAutoConnect=1");
  await page.evaluate((threshold) => {
    (window as unknown as Record<string, unknown>).__debug_large_file_points = threshold;
  }, THRESHOLD);

  const info = page.locator('[data-testid="orbit-info-file"]');
  const warning = page.locator('[data-testid="file-size-warning"]');

  await load(page, writeCSV("large", 50));
  await expect(info).toContainText("50 points", { timeout: 10_000 });
  await expect(warning).toContainText("50 points");

  // No data rows: the load is refused and the 50-point file stays on screen.
  await load(page, writeCSV("empty", 0));
  await expect(info).toContainText("No valid orbit data", { timeout: 10_000 });
  await expect(page.locator('[data-testid="orbit-info-points"]')).toContainText("50 points");
  await expect(warning).toContainText("50 points");

  await load(page, writeCSV("small", 30));
  await expect(info).toContainText("30 points", { timeout: 10_000 });
  await expect(warning).toHaveCount(0);
});
