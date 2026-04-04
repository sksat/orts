import { expect, test } from "@playwright/test";

/**
 * Verify that the Worker-based chart data pipeline does NOT block
 * the main thread during data streaming.
 *
 * Uses requestAnimationFrame frame-interval jitter to detect blocking:
 * if heavy processing runs on the main thread, rAF callbacks are delayed
 * and frame intervals spike well above 16ms.
 *
 * Threshold: fewer than 5% of frames should exceed 100ms interval.
 * A warm-up period is excluded (Worker/DuckDB initialization).
 */

// FIXME: all E2E tests hardcode ports; should use playwright projects with baseURL
// to avoid port collisions. See sine-chart.spec.ts, data-coverage.spec.ts, etc.
const SINE_WAVE_URL = "http://localhost:5174";

test("main thread stays responsive during Worker data streaming", async ({ page }) => {
  test.setTimeout(40000);

  // Navigate to the sine-wave example (uses Worker hook) in mock mode
  await page.goto(`${SINE_WAVE_URL}?mock`);

  // Wait for charts to appear (Worker is initialized and producing data)
  await page.waitForSelector(".uplot", { timeout: 15000 });

  // Start measuring frame intervals via rAF
  const result = await page.evaluate(() => {
    return new Promise<{
      totalFrames: number;
      jankFrames: number;
      jankRatio: number;
      maxFrameMs: number;
      avgFrameMs: number;
    }>((resolve) => {
      const frameTimes: number[] = [];
      let prevTime = performance.now();
      let measuring = false;

      // Warm-up: skip the first 2 seconds (Worker/DuckDB init)
      setTimeout(() => {
        measuring = true;
        prevTime = performance.now();
      }, 2000);

      // Measure for 5 seconds after warm-up
      const measureDuration = 5000;
      const startTimeout = setTimeout(() => {
        // This timeout fires 2s (warmup) + 5s (measure) after page load
        const intervals = frameTimes;
        const jankThreshold = 100; // ms
        const jankFrames = intervals.filter((t) => t > jankThreshold).length;
        const maxFrameMs = intervals.length > 0 ? Math.max(...intervals) : 0;
        const avgFrameMs =
          intervals.length > 0 ? intervals.reduce((a, b) => a + b, 0) / intervals.length : 0;

        resolve({
          totalFrames: intervals.length,
          jankFrames,
          jankRatio: intervals.length > 0 ? jankFrames / intervals.length : 0,
          maxFrameMs: Math.round(maxFrameMs * 10) / 10,
          avgFrameMs: Math.round(avgFrameMs * 10) / 10,
        });
      }, 2000 + measureDuration);

      function tick(now: number) {
        if (measuring) {
          const dt = now - prevTime;
          frameTimes.push(dt);
        }
        prevTime = now;
        requestAnimationFrame(tick);
      }

      requestAnimationFrame(tick);
    });
  });

  console.log(
    `Frame stats: total=${result.totalFrames}, jank=${result.jankFrames} ` +
      `(${(result.jankRatio * 100).toFixed(1)}%), max=${result.maxFrameMs}ms, avg=${result.avgFrameMs}ms`,
  );

  // At least some frames were measured
  expect(result.totalFrames).toBeGreaterThan(50);

  // Fewer than 5% of frames should exceed 100ms (jank threshold)
  expect(result.jankRatio).toBeLessThan(0.05);
});

test("Worker produces chart data from streamed points", async ({ page }) => {
  test.setTimeout(30000);

  await page.goto(`${SINE_WAVE_URL}?mock`);

  // Wait for uPlot containers (charts rendering with data)
  await page.waitForSelector(".uplot", { timeout: 15000 });

  // Wait a bit for data to accumulate, then check canvas has rendered
  await page.waitForTimeout(3000);

  const canvasCount = await page.locator(".uplot canvas").count();
  expect(canvasCount).toBeGreaterThanOrEqual(3);
});
