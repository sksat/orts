/**
 * E2E test: a file with no points replaces the chart of the file before it.
 *
 * The single-satellite chart worker, and the DuckDB table in it, outlive a
 * source switch. A loaded file is charted from that table, so a file that
 * brings no points (an empty `.rrd` decodes to none) has to empty it, or the
 * previous file's rows stay on the chart.
 */

import { writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, type Page, test } from "@playwright/test";

function chartSpan(page: Page): Promise<number[] | null> {
  return page.evaluate(() => {
    const data = (window as unknown as Record<string, unknown>).__debug_chart_data as {
      t?: ArrayLike<number>;
    } | null;
    if (!data?.t || data.t.length === 0) return null;
    return [data.t[0], data.t[data.t.length - 1]];
  });
}

test("a file with no points replaces the chart of the file before it", async ({ page }) => {
  const lines = ["# mu = 398600.4418 km^3/s^2", "# epoch_jd = 2451545.0"];
  for (let i = 0; i < 50; i++) lines.push(`${i * 10},6778,0,0,0,7.669,0,6778,0,0.9,0,0,0`);
  const csvPath = join(tmpdir(), `orts-before-empty-${Date.now()}.csv`);
  writeFileSync(csvPath, lines.join("\n"));
  const emptyRrd = join(tmpdir(), `orts-empty-${Date.now()}.rrd`);
  writeFileSync(emptyRrd, "");

  await page.goto("/?noAutoConnect=1");
  const fileInput = page.locator('input[type="file"]');
  const info = page.locator('[data-testid="orbit-info-file"]');

  await fileInput.setInputFiles(csvPath);
  await expect(info).toContainText("50 points", { timeout: 10_000 });
  await expect.poll(() => chartSpan(page), { timeout: 30_000 }).toEqual([0, 490]);

  await fileInput.setInputFiles(emptyRrd);
  await expect(info).toContainText("0 points", { timeout: 10_000 });
  await expect.poll(() => chartSpan(page), { timeout: 30_000 }).toBeNull();
});
