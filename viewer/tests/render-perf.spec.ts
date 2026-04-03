/**
 * E2E: Render performance regression test.
 *
 * Measures FPS with charts collapsed as data accumulates.
 * Uses hardware GPU acceleration for realistic measurement.
 * Baseline: 37a37c7 achieved 288 fps at 5000 pts, 344 fps at 7000 pts.
 * Regression commit 6cf34b8 dropped to 62 fps at 5000 pts.
 *
 * We require at least 200 fps at 5000 pts (collapsed charts, HW GPU)
 * to prevent the chartBufferVersion/liveChartData regression from returning.
 *
 * Skipped automatically when hardware GPU is not available (e.g. swiftshader).
 */

import { type ChildProcess, spawn } from "node:child_process";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

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

  const binary =
    process.env.ORTS_BINARY ??
    path.resolve(__dirname, "../../target/debug/orts");
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

test("charts-collapsed FPS stays above 200 at 5000 points", async ({
  page,
}) => {
  test.setTimeout(180_000);
  await page.goto("/?noAutoConnect=1");

  // Detect software rendering (swiftshader) — skip if no hardware GPU
  const renderer = await page.evaluate(() => {
    const canvas = document.createElement("canvas");
    const gl =
      canvas.getContext("webgl2") || canvas.getContext("webgl");
    if (!gl) return "none";
    const ext = gl.getExtension("WEBGL_debug_renderer_info");
    return ext
      ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL)
      : "unknown";
  });
  console.log(`GPU renderer: ${renderer}`);
  test.skip(
    /swiftshader|llvmpipe|software/i.test(renderer),
    `Software renderer detected (${renderer}); skipping perf test`,
  );

  await page.waitForTimeout(3000);

  // Connect
  const urlInput = page
    .locator('[data-testid="ws-url-input"], .ws-url-input')
    .first();
  await urlInput.waitFor({ timeout: 10000 });
  await urlInput.fill(wsUrl);
  await page
    .locator('[data-testid="ws-connect-btn"], .ws-connect-btn')
    .first()
    .click();
  await page
    .locator('[data-testid="ws-status-text"], .ws-status-text')
    .first()
    .filter({ hasText: "Connected" })
    .waitFor({ timeout: 10000 });

  // Collapse charts
  const toggle = page
    .locator('[class*="toggle"], .graph-panel-toggle')
    .first();
  if ((await toggle.count()) > 0) {
    await toggle.click();
    await page.waitForTimeout(500);
  }

  // Wait for 5000+ points
  await page.waitForFunction(
    () => {
      const el =
        document.querySelector('[data-testid="orbit-info-points"]') ||
        document.querySelector(".orbit-info:last-of-type");
      if (!el) return false;
      const m = el.textContent?.match(/(\d+)\s*points/);
      return m && Number.parseInt(m[1], 10) >= 5000;
    },
    undefined,
    { timeout: 120000 },
  );

  // Measure FPS over 5 seconds
  await page.evaluate(() => {
    const w = window as any;
    w.__ft = [];
    w.__ftLast = 0;
    function m(now: number) {
      if (w.__ftLast > 0) w.__ft.push(now - w.__ftLast);
      w.__ftLast = now;
      requestAnimationFrame(m);
    }
    requestAnimationFrame(m);
  });

  await page.waitForTimeout(5000);

  const result = await page.evaluate(() => {
    const ft: number[] = (window as any).__ft;
    if (ft.length === 0) return { fps: 0, avgMs: 0, frames: 0, points: 0 };
    const avg = ft.reduce((a, b) => a + b, 0) / ft.length;

    const el =
      document.querySelector('[data-testid="orbit-info-points"]') ||
      document.querySelector(".orbit-info:last-of-type");
    const m = el?.textContent?.match(/(\d+)\s*points/);
    const points = m ? Number.parseInt(m[1], 10) : 0;

    return {
      fps: Math.round(1000 / avg),
      avgMs: Math.round(avg * 10) / 10,
      frames: ft.length,
      points,
    };
  });

  console.log(
    `PERF: ${result.points} pts | ${result.fps} fps | ${result.avgMs}ms avg | ${result.frames} frames`,
  );

  // Baseline: 37a37c7 = 288 fps at 5000 pts
  // Regression: 6cf34b8 = 62 fps at 5000 pts
  // Threshold: 200 fps (70% of baseline, well above regression)
  expect(result.fps).toBeGreaterThanOrEqual(200);
});
