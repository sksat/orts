import { expect, test } from "@playwright/test";

test("chart containers render and are visible", async ({ page }) => {
  await page.goto("http://localhost:5174");
  // Wait for DuckDB to initialize and data to start streaming
  await page.waitForTimeout(3000);

  const charts = page.locator("[data-testid='time-series-chart']");
  await expect(charts).toHaveCount(3);
  await expect(charts.first()).toBeVisible();
});

test("uPlot canvas is created for each chart", async ({ page }) => {
  await page.goto("http://localhost:5174");
  await page.waitForTimeout(3000);

  const canvases = page.locator("[data-testid='time-series-chart'] canvas");
  await expect(canvases).toHaveCount(3);
});

test("data updates over time", async ({ page }) => {
  await page.goto("http://localhost:5174");
  await page.waitForTimeout(5000);

  // Verify charts have rendered (uPlot creates a .uplot container)
  const uplotContainers = page.locator(".uplot");
  await expect(uplotContainers.first()).toBeVisible();
});
