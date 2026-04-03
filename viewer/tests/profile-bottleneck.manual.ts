/**
 * Profiling: isolate whether bottleneck is 3D (Three.js/GPU) or charts (uPlot).
 * Compare FPS with GraphPanel visible vs collapsed.
 */

import { type ChildProcess, spawn } from "node:child_process";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { test } from "@playwright/test";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

let ortsProcess: ChildProcess | undefined;
let wsUrl: string;

test.beforeAll(async () => {
  if (process.env.ORTS_WS_URL) {
    wsUrl = process.env.ORTS_WS_URL;
    return;
  }

  const binary = process.env.ORTS_BINARY ?? path.resolve(__dirname, "../../target/debug/orts");
  const child = spawn(binary, [
    "serve",
    "--port",
    "0",
    "--sat",
    "altitude=400,id=perf-test",
    "--dt",
    "1",
    "--output-interval",
    "10",
  ]);
  ortsProcess = child;

  const port = await new Promise<number>((resolve, reject) => {
    const rl = createInterface({ input: child.stderr! });
    const timeout = setTimeout(() => {
      rl.close();
      reject(new Error("Timed out waiting for orts server to start"));
    }, 30000);

    rl.on("line", (line) => {
      const match = line.match(/ws:\/\/localhost:(\d+)/);
      if (match) {
        clearTimeout(timeout);
        resolve(Number.parseInt(match[1], 10));
      }
    });

    child.on("error", (err) => {
      clearTimeout(timeout);
      reject(err);
    });
    child.on("exit", (code) => {
      clearTimeout(timeout);
      reject(new Error(`orts exited with code ${code}`));
    });
  });

  wsUrl = `ws://localhost:${port}/ws`;
});

test.afterAll(async () => {
  if (ortsProcess && !ortsProcess.killed) {
    ortsProcess.kill("SIGTERM");
  }
});

async function measureFps(page: import("@playwright/test").Page, durationMs: number) {
  await page.evaluate(() => {
    (window as any).__ft = [];
    (window as any).__ftLast = 0;
  });

  await page.evaluate(() => {
    function m(now: number) {
      if ((window as any).__ftLast > 0) (window as any).__ft.push(now - (window as any).__ftLast);
      (window as any).__ftLast = now;
      requestAnimationFrame(m);
    }
    requestAnimationFrame(m);
  });

  await page.waitForTimeout(durationMs);

  return page.evaluate(() => {
    const ft: number[] = (window as any).__ft;
    (window as any).__ft = [];
    (window as any).__ftLast = 0;
    if (ft.length === 0) return { fps: 0, avgMs: 0, p95ms: 0, frames: 0 };
    const avg = ft.reduce((a, b) => a + b, 0) / ft.length;
    const sorted = [...ft].sort((a, b) => a - b);
    const p95 = sorted[Math.floor(sorted.length * 0.95)];
    return {
      fps: Math.round(1000 / avg),
      avgMs: Math.round(avg),
      p95ms: Math.round(p95),
      frames: ft.length,
    };
  });
}

test("isolate: charts vs 3D", async ({ page }) => {
  test.setTimeout(90_000);

  await page.goto("/?noAutoConnect=1");

  // Connect
  await page.locator('[data-testid="ws-url-input"]').fill(wsUrl);
  await page.locator('[data-testid="ws-connect-btn"]').click();
  await page
    .locator('[data-testid="ws-status-text"]')
    .filter({ hasText: "Connected" })
    .waitFor({ timeout: 10000 });

  // Wait for 500+ points
  await page.waitForFunction(
    () => {
      const el = document.querySelector('[data-testid="orbit-info-points"]');
      if (!el) return false;
      const m = el.textContent?.match(/(\d+)\s*points/);
      return m && Number.parseInt(m[1], 10) > 500;
    },
    { timeout: 60000 },
  );

  const points = await page.evaluate(() => {
    const el = document.querySelector('[data-testid="orbit-info-points"]');
    const m = el?.textContent?.match(/(\d+)\s*points/);
    return m ? Number.parseInt(m[1], 10) : 0;
  });

  console.log(`\nData points: ${points}`);

  // --- Test 1: Charts visible (default) ---
  console.log("\n--- Charts VISIBLE ---");
  const withCharts = await measureFps(page, 3000);
  console.log(JSON.stringify(withCharts));

  // --- Test 2: Collapse GraphPanel ---
  console.log("\n--- Charts COLLAPSED ---");
  const toggleBtn = page.locator('[class*="toggle"]').first();
  await toggleBtn.click();
  await page.waitForTimeout(500);
  const withoutCharts = await measureFps(page, 3000);
  console.log(JSON.stringify(withoutCharts));

  // --- Test 3: Hide 3D canvas entirely via CSS ---
  console.log("\n--- 3D HIDDEN (canvas display:none) ---");
  await toggleBtn.click(); // re-open charts
  await page.waitForTimeout(500);
  await page.evaluate(() => {
    const canvas = document.querySelector("canvas[data-engine]");
    if (canvas) (canvas as HTMLElement).style.display = "none";
  });
  await page.waitForTimeout(500);
  const without3D = await measureFps(page, 3000);
  console.log(JSON.stringify(without3D));

  // --- Test 4: Both hidden ---
  console.log("\n--- BOTH HIDDEN ---");
  await toggleBtn.click(); // collapse charts
  const withoutBoth = await measureFps(page, 3000);
  console.log(JSON.stringify(withoutBoth));

  // Restore
  await page.evaluate(() => {
    const canvas = document.querySelector("canvas[data-engine]");
    if (canvas) (canvas as HTMLElement).style.display = "";
  });

  console.log("\n=== SUMMARY ===");
  console.log(`Points:          ${points}`);
  console.log(`Charts+3D:       ${withCharts.fps} fps (${withCharts.avgMs}ms avg)`);
  console.log(`3D only:         ${withoutCharts.fps} fps (${withoutCharts.avgMs}ms avg)`);
  console.log(`Charts only:     ${without3D.fps} fps (${without3D.avgMs}ms avg)`);
  console.log(`Neither:         ${withoutBoth.fps} fps (${withoutBoth.avgMs}ms avg)`);
  console.log("");
});
