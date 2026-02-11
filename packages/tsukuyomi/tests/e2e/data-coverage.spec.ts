import { test, expect } from "@playwright/test";

/**
 * E2E test for mixed-density data coverage.
 *
 * Reproduces the chart data disappearance bug observed in the viewer
 * with long-running simulations. The scenario:
 *   - Server sends 100 sparse "overview" points covering t=[0, 5000) at 50s intervals
 *   - Server then streams dense points at t=5000+ (100 msg/sec, dt=0.1)
 *
 * After enough streaming data accumulates, the ROW_NUMBER-based downsampling
 * in buildDerivedQuery allocates most of the display budget to the dense
 * streaming region, leaving the sparse overview region nearly empty.
 *
 * Expected: the sparse region (0-5000s) which covers ~91% of the time range
 *           should have proportional representation in the chart data.
 */

const MIXED_DENSITY_URL = "http://localhost:5175";

test("mixed-density: sparse region has proportional chart coverage", async ({
  page,
}) => {
  test.setTimeout(50000);

  await page.goto(MIXED_DENSITY_URL);

  // Wait for DuckDB to initialize and data to start flowing
  await page.waitForTimeout(3000);

  // Wait until enough streaming data has accumulated (>2000 total points
  // triggers ROW_NUMBER downsampling with DISPLAY_MAX_POINTS=2000)
  await page.waitForFunction(
    () => {
      const debug = (window as any).__tsukuyomiDebug;
      return debug?.chartData?.t?.length > 0;
    },
    { timeout: 30000 },
  );

  // Let more streaming data accumulate to create the density mismatch
  // 100 msg/sec * 20 sec = ~2000 streaming points + 100 overview = ~2100 total
  await page.waitForTimeout(20000);

  // Analyze the chart data distribution
  const result = await page.evaluate(async () => {
    const debug = (window as any).__tsukuyomiDebug;
    if (!debug?.chartData?.t) return null;

    const t = debug.chartData.t;
    let sparseCount = 0;
    let denseCount = 0;
    for (let i = 0; i < t.length; i++) {
      if (t[i] < 5000) sparseCount++;
      else denseCount++;
    }

    // Also query DuckDB for the actual row count
    const dbRowCount = await debug.queryRowCount();

    return {
      chartTotal: t.length,
      sparseCount,
      denseCount,
      tMin: t[0],
      tMax: t[t.length - 1],
      dbRowCount,
    };
  });

  console.log("Data coverage result:", JSON.stringify(result, null, 2));

  expect(result).not.toBeNull();

  // Verify DuckDB has received all data (no data loss)
  expect(result!.dbRowCount).toBeGreaterThan(2000);

  // Verify chart data covers the full time range
  expect(result!.tMin).toBeLessThan(100); // Should start near t=0
  expect(result!.tMax).toBeGreaterThan(5000); // Should extend into streaming region

  // KEY ASSERTION: The sparse region (0-5000s) should retain nearly all its
  // original 100 overview points. Time-bucket downsampling allocates ~95% of
  // the display budget to this region (it covers ~95% of the time range),
  // so all 100 sparse points survive.
  //
  // Before fix (row-count-based): sparseCount ≈ 4% of total (the bug)
  // After fix (time-bucket-based): sparseCount ≈ 47% of total
  //
  // We assert sparseCount >= 80 (most of the 100 overview points survive)
  // and sparseCount > 30% of total (well above the ~4% bug level).
  const sparseRatio = result!.sparseCount / result!.chartTotal;
  console.log(
    `Sparse coverage: ${result!.sparseCount}/${result!.chartTotal} = ${(sparseRatio * 100).toFixed(1)}%`,
  );
  expect(result!.sparseCount).toBeGreaterThanOrEqual(80);
  expect(result!.sparseCount).toBeGreaterThan(result!.chartTotal * 0.3);
});

test("mixed-density: DuckDB row count is monotonically increasing (no data loss)", async ({
  page,
}) => {
  test.setTimeout(40000);

  await page.goto(MIXED_DENSITY_URL);
  await page.waitForTimeout(3000);

  // Sample DuckDB row count at intervals and verify it never decreases
  const counts: number[] = [];
  for (let i = 0; i < 5; i++) {
    await page.waitForTimeout(3000);
    const count = await page.evaluate(async () => {
      const debug = (window as any).__tsukuyomiDebug;
      if (!debug?.queryRowCount) return 0;
      return debug.queryRowCount();
    });
    counts.push(count);
  }

  console.log("DuckDB row counts over time:", counts);

  // Every sample should be >= the previous one (monotonically increasing)
  for (let i = 1; i < counts.length; i++) {
    expect(counts[i]).toBeGreaterThanOrEqual(counts[i - 1]);
  }

  // Should have accumulated substantial data by the end
  expect(counts[counts.length - 1]).toBeGreaterThan(1000);
});
