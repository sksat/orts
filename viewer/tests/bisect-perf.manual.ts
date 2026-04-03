/**
 * Perf profiling: FPS vs point count curve.
 * Measures FPS at multiple point count thresholds to see degradation.
 */

import { type ChildProcess, spawn } from "node:child_process";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { test } from "@playwright/test";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// Use hardware GPU for realistic measurement
test.use({
  launchOptions: {
    args: ["--enable-gpu", "--enable-webgl", "--ignore-gpu-blocklist"],
  },
});

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
    "1",
  ]);
  ortsProcess = child;

  const port = await new Promise<number>((resolve, reject) => {
    const rl = createInterface({ input: child.stderr! });
    const timeout = setTimeout(() => {
      rl.close();
      reject(new Error("Timed out"));
    }, 30000);
    rl.on("line", (line) => {
      const m = line.match(/ws:\/\/localhost:(\d+)/);
      if (m) {
        clearTimeout(timeout);
        resolve(Number.parseInt(m[1], 10));
      }
    });
    child.on("error", (e) => {
      clearTimeout(timeout);
      reject(e);
    });
    child.on("exit", (c) => {
      clearTimeout(timeout);
      reject(new Error(`exit ${c}`));
    });
  });
  wsUrl = `ws://localhost:${port}/ws`;
});

test.afterAll(async () => {
  if (ortsProcess && !ortsProcess.killed) ortsProcess.kill("SIGTERM");
});

function getPointCount(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    const el =
      document.querySelector('[data-testid="orbit-info-points"]') ||
      document.querySelector(".orbit-info:last-of-type");
    if (!el) return 0;
    const m = el.textContent?.match(/(\d+)\s*points/);
    return m ? Number.parseInt(m[1], 10) : 0;
  });
}

async function measureFps(page: import("@playwright/test").Page) {
  await page.evaluate(() => {
    (window as any).__ft = [];
    (window as any).__ftLast = 0;
    function m(now: number) {
      if ((window as any).__ftLast > 0) (window as any).__ft.push(now - (window as any).__ftLast);
      (window as any).__ftLast = now;
      requestAnimationFrame(m);
    }
    requestAnimationFrame(m);
  });

  await page.waitForTimeout(3000);

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

test("FPS vs point count curve", async ({ page }) => {
  test.setTimeout(300_000);
  await page.goto("/?noAutoConnect=1");
  await page.waitForTimeout(3000);

  // Connect
  const urlInput = page.locator('[data-testid="ws-url-input"], .ws-url-input').first();
  await urlInput.waitFor({ timeout: 10000 });
  await urlInput.fill(wsUrl);
  const connectBtn = page.locator('[data-testid="ws-connect-btn"], .ws-connect-btn').first();
  await connectBtn.click();
  const statusText = page.locator('[data-testid="ws-status-text"], .ws-status-text').first();
  await statusText.filter({ hasText: "Connected" }).waitFor({ timeout: 10000 });

  // Collapse charts to isolate 3D perf
  const toggle = page.locator('[class*="toggle"], .graph-panel-toggle').first();
  if ((await toggle.count()) > 0) {
    await toggle.click();
    await page.waitForTimeout(500);
  }

  console.log("\n=== FPS vs Point Count (charts collapsed, HW GPU) ===\n");
  console.log("Points | FPS | avg ms | p95 ms");
  console.log("-------|-----|--------|-------");

  const thresholds = [500, 1000, 2000, 3000, 5000, 7000, 10000];

  for (const threshold of thresholds) {
    // Wait for point count to reach threshold
    try {
      await page.waitForFunction(
        (t) => {
          const el =
            document.querySelector('[data-testid="orbit-info-points"]') ||
            document.querySelector(".orbit-info:last-of-type");
          if (!el) return false;
          const m = el.textContent?.match(/(\d+)\s*points/);
          return m && Number.parseInt(m[1], 10) >= t;
        },
        threshold,
        { timeout: 60000 },
      );
    } catch {
      const pts = await getPointCount(page);
      console.log(`(stopped at ${pts} pts — could not reach ${threshold})`);
      break;
    }

    const pts = await getPointCount(page);
    const result = await measureFps(page);
    console.log(
      `${String(pts).padStart(6)} | ${String(result.fps).padStart(3)} | ${String(result.avgMs).padStart(6)} | ${String(result.p95ms).padStart(6)}`,
    );
  }

  console.log("\n=== DONE ===\n");
});
