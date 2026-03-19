import { expect, test } from "@playwright/test";

/** Wait for the chart to be visible with at least one canvas (uPlot fully initialized). */
async function waitForChart(page: import("@playwright/test").Page) {
  const chart = page.locator("[data-testid='time-series-chart']");
  await expect(chart).toBeVisible({ timeout: 15000 });
  // uPlot renders multiple canvases (plot + overlay); wait for at least one
  await expect(chart.locator("canvas").first()).toBeVisible({ timeout: 15000 });
}

test("multi-series chart renders with canvas", async ({ page }) => {
  await page.goto("http://localhost:5176");
  await waitForChart(page);

  const chart = page.locator("[data-testid='time-series-chart']");
  await expect(chart).toHaveCount(1);
  await expect(chart).toBeVisible();

  const canvas = chart.locator("canvas");
  await expect(canvas).toBeVisible();
});

test("uPlot legend shows both series labels", async ({ page }) => {
  await page.goto("http://localhost:5176");
  await waitForChart(page);

  // uPlot legend renders series labels as .u-series elements inside .u-legend
  const legendEntries = page.locator(".u-legend .u-series");
  // Expect at least 2 series entries (slow + fast) plus the x-axis entry
  const count = await legendEntries.count();
  expect(count).toBeGreaterThanOrEqual(3);

  // Check that both "slow" and "fast" labels appear in the legend
  const legendText = await page.locator(".u-legend").textContent();
  expect(legendText).toContain("slow");
  expect(legendText).toContain("fast");
});

test("legend click isolates a series (Grafana-style)", async ({ page }) => {
  await page.goto("http://localhost:5176");
  await waitForChart(page);

  const legendEntries = page.locator(".u-legend .u-series");
  await expect(legendEntries).toHaveCount(3, { timeout: 10000 });

  // entries: [0]=x-axis, [1]=slow, [2]=fast
  const slowEntry = legendEntries.nth(1);
  const fastEntry = legendEntries.nth(2);

  // Initially both y-series should be visible (no u-off class)
  await expect(slowEntry).not.toHaveClass(/u-off/);
  await expect(fastEntry).not.toHaveClass(/u-off/);

  // Click "slow" → isolate it (fast should get u-off)
  await slowEntry.click();
  await expect(fastEntry).toHaveClass(/u-off/, { timeout: 5000 });
  await expect(slowEntry).not.toHaveClass(/u-off/);

  // Click "slow" again → un-isolate (show all)
  await slowEntry.click();
  await expect(slowEntry).not.toHaveClass(/u-off/, { timeout: 5000 });
  await expect(fastEntry).not.toHaveClass(/u-off/, { timeout: 5000 });
});

test("legend click on hidden series isolates it", async ({ page }) => {
  await page.goto("http://localhost:5176");
  await waitForChart(page);

  const legendEntries = page.locator(".u-legend .u-series");
  await expect(legendEntries).toHaveCount(3, { timeout: 10000 });

  const slowEntry = legendEntries.nth(1);
  const fastEntry = legendEntries.nth(2);

  // Click "slow" → isolate it
  await slowEntry.click();
  await expect(fastEntry).toHaveClass(/u-off/, { timeout: 5000 });

  // Click "fast" (currently hidden) → isolate fast instead
  await fastEntry.click();
  await expect(fastEntry).not.toHaveClass(/u-off/, { timeout: 5000 });
  await expect(slowEntry).toHaveClass(/u-off/, { timeout: 5000 });
});
