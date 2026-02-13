import { test, expect } from "@playwright/test";

test("multi-series chart renders with canvas", async ({ page }) => {
  await page.goto("http://localhost:5176");
  await page.waitForTimeout(3000);

  const chart = page.locator("[data-testid='time-series-chart']");
  await expect(chart).toHaveCount(1);
  await expect(chart).toBeVisible();

  const canvas = chart.locator("canvas");
  await expect(canvas).toBeVisible();
});

test("uPlot legend shows both series labels", async ({ page }) => {
  await page.goto("http://localhost:5176");
  await page.waitForTimeout(3000);

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
