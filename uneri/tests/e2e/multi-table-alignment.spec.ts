import { expect, test } from "@playwright/test";

/**
 * E2E test: multi-table DuckDB downsampling produces aligned timestamps.
 *
 * Two DuckDB tables (tbl_alpha, tbl_beta) receive data at the same rate.
 * After enough data accumulates, both tables are queried with the same
 * unified tMax via buildDerivedQuery's time-bucket downsampling.
 *
 * Key assertion: timestamps from both queries should be identical (or
 * near-identical), so alignTimeSeries() doesn't introduce NaN gaps.
 * Without the unified tMax fix, independent bucket boundaries cause
 * different timestamps → NaN in 70-80% of aligned samples.
 */

const MULTI_TABLE_URL = "http://localhost:5177";

test("multi-table: aligned timestamps after downsampling (no NaN gaps)", async ({ page }) => {
  test.setTimeout(50000);

  await page.goto(MULTI_TABLE_URL);

  // Wait for DuckDB to initialize and data to flow
  await page.waitForTimeout(3000);

  // Wait until both tables have data
  await page.waitForFunction(
    () => {
      const debug = (window as any).__multiTableDebug;
      return debug?.alphaCount > 0 && debug?.betaCount > 0;
    },
    { timeout: 15000 },
  );

  // Accumulate enough data to trigger downsampling (>500 points per table)
  // Server sends at 100pts/sec, so wait ~8 seconds for ~800 points each
  await page.waitForTimeout(8000);

  // Query alignment via the debug API (runs real DuckDB queries)
  const result = await page.evaluate(async () => {
    const debug = (window as any).__multiTableDebug;
    if (!debug?.queryAlignment) return { error: "debug API not available" };
    try {
      return await debug.queryAlignment();
    } catch (e: any) {
      return { error: e.message };
    }
  });

  console.log(
    "Alignment result:",
    JSON.stringify(
      {
        ...result,
        // Truncate arrays for readability
        alphaT: (result as any).alphaT?.slice(0, 3),
        betaT: (result as any).betaT?.slice(0, 3),
        alphaTLast: (result as any).alphaT?.slice(-3),
        betaTLast: (result as any).betaT?.slice(-3),
        alphaLen: (result as any).alphaT?.length,
        betaLen: (result as any).betaT?.length,
      },
      null,
      2,
    ),
  );

  expect(result).not.toHaveProperty("error");

  const r = result as {
    alphaT: number[];
    betaT: number[];
    alphaValues: number[];
    betaValues: number[];
    unifiedTMax: number;
    alignmentRatio: number;
    alphaNanCount: number;
    betaNanCount: number;
  };

  // Both tables should have produced downsampled data
  expect(r.alphaT.length, "alpha should have downsampled data").toBeGreaterThan(10);
  expect(r.betaT.length, "beta should have downsampled data").toBeGreaterThan(10);

  // No NaN in values
  expect(r.alphaNanCount, "alpha values should have no NaN").toBe(0);
  expect(r.betaNanCount, "beta values should have no NaN").toBe(0);

  // Timestamp alignment: with unified tMax, same-rate tables produce
  // identical timestamp sets from time-bucket downsampling.
  // Before the fix (independent tMax): alignment ~0.2-0.3
  // After the fix (unified tMax): alignment should be >0.95
  expect(
    r.alignmentRatio,
    `Alignment ratio should be > 0.95 (was ${r.alignmentRatio.toFixed(3)})`,
  ).toBeGreaterThan(0.95);
});

test("multi-table: NaN count in aligned chart data is zero", async ({ page }) => {
  test.setTimeout(50000);

  await page.goto(MULTI_TABLE_URL);
  await page.waitForTimeout(3000);

  // Wait for chart data to render
  await page.waitForFunction(
    () => {
      const debug = (window as any).__multiTableDebug;
      return debug?.alphaCount > 200 && debug?.betaCount > 200;
    },
    { timeout: 30000 },
  );

  // Wait for chart queries to stabilize with zero NaN.
  // Use polling instead of a fixed wait to avoid flaky races at chart boundaries.
  await page.waitForFunction(
    () => {
      const el = document.querySelector("[data-testid='stats']");
      if (!el) return false;
      const text = el.textContent ?? "";
      const alpha = text.match(/NaN: alpha=(\d+)/);
      const beta = text.match(/beta=(\d+)/);
      return alpha && beta && alpha[1] === "0" && beta[1] === "0";
    },
    { timeout: 15000 },
  );

  const statsText = await page.locator("[data-testid='stats']").textContent();
  console.log("Stats:", statsText);
});
