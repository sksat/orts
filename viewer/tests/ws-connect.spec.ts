import { test, expect } from "@playwright/test";

const VIEWER_URL = "http://localhost:5173";
const WS_URL = "ws://localhost:9001";

test("raw WebSocket connects and receives messages", async ({ page }) => {
  await page.goto(VIEWER_URL);

  const wsResult = await page.evaluate(async (url) => {
    return new Promise<string>((resolve) => {
      const ws = new WebSocket(url);
      const events: string[] = [];

      ws.addEventListener("open", () => events.push("open"));
      ws.addEventListener("close", (e) => {
        events.push(`close:code=${e.code},reason=${e.reason},clean=${e.wasClean}`);
        resolve(events.join(" | "));
      });
      ws.addEventListener("error", () => events.push("error"));
      ws.addEventListener("message", (e) => {
        events.push(`message:${(e.data as string).substring(0, 80)}`);
        if (events.filter((ev) => ev.startsWith("message:")).length >= 3) {
          ws.close();
        }
      });

      setTimeout(() => {
        events.push(`timeout:readyState=${ws.readyState}`);
        ws.close();
        resolve(events.join(" | "));
      }, 5000);
    });
  }, WS_URL);

  console.log("Raw WS result:", wsResult);
  expect(wsResult).toContain("open");
  expect(wsResult).toContain("message:");
});

test("realtime mode auto-connects and streams orbit data", async ({ page }) => {
  const consoleLogs: string[] = [];
  page.on("console", (msg) => consoleLogs.push(`[${msg.type()}] ${msg.text()}`));
  page.on("pageerror", (err) => consoleLogs.push(`[PAGE_ERROR] ${err.message}`));

  await page.goto(VIEWER_URL);

  // Realtime mode is the default and should auto-connect.
  const statusText = page.locator(".ws-status-text");
  await expect(statusText).toHaveText("Connected", { timeout: 10000 });

  // Wait for data to stream in
  await page.waitForTimeout(3000);

  // Dump HTML after streaming
  const html = await page.evaluate(() => document.body.innerHTML.substring(0, 3000));
  console.log("HTML after streaming:", html);
  console.log("Console logs:", consoleLogs);

  await page.screenshot({ path: "/tmp/viewer-connected.png" });

  // Check if the page crashed (UI overlay gone)
  const uiOverlay = page.locator(".ui-overlay");
  const overlayCount = await uiOverlay.count();
  console.log("UI overlay elements:", overlayCount);

  expect(overlayCount, "UI overlay should still exist (React did not crash)").toBe(1);
});

test("charts render with data after streaming starts", async ({ page }) => {
  const consoleLogs: string[] = [];
  page.on("console", (msg) => consoleLogs.push(`[${msg.type()}] ${msg.text()}`));

  await page.goto(VIEWER_URL);

  // Wait for WebSocket connection
  const statusText = page.locator(".ws-status-text");
  await expect(statusText).toHaveText("Connected", { timeout: 10000 });

  // Wait for DuckDB to initialize and charts to render (may be slow in CI)
  const charts = page.locator('[data-testid="time-series-chart"]');
  await expect(charts.first()).toBeVisible({ timeout: 30000 });
  const chartCount = await charts.count();
  expect(chartCount).toBeGreaterThanOrEqual(4);

  // Each chart should contain a uPlot canvas (indicates data was rendered)
  for (let i = 0; i < chartCount; i++) {
    const canvas = charts.nth(i).locator("canvas");
    await expect(canvas).toBeVisible({ timeout: 5000 });
  }

  // Verify uPlot instances actually have data points (not just empty canvases).
  // uPlot attaches itself to the container's first child div.u-wrap.
  // We can check for non-empty canvas pixel data or the uPlot instance's data length.
  const chartDataLengths = await page.evaluate(() => {
    const containers = document.querySelectorAll('[data-testid="time-series-chart"]');
    return Array.from(containers).map((container) => {
      // uPlot sets `u-wrap` as direct child, with canvas inside
      const canvas = container.querySelector("canvas") as HTMLCanvasElement | null;
      if (!canvas) return { hasCanvas: false, width: 0, height: 0, hasPixels: false };
      // Check if canvas has been drawn to by sampling a few pixels
      const ctx = canvas.getContext("2d");
      if (!ctx) return { hasCanvas: true, width: canvas.width, height: canvas.height, hasPixels: false };
      // Sample the center area of the canvas for non-transparent pixels
      const w = canvas.width;
      const h = canvas.height;
      const imageData = ctx.getImageData(0, 0, w, h);
      let nonTransparentPixels = 0;
      for (let i = 3; i < imageData.data.length; i += 4) {
        if (imageData.data[i] > 0) nonTransparentPixels++;
      }
      return { hasCanvas: true, width: w, height: h, hasPixels: nonTransparentPixels > 100 };
    });
  });

  console.log("Chart data:", chartDataLengths);

  // At least the first 4 charts (altitude, energy, angular_momentum, velocity) should have drawn pixels
  for (let i = 0; i < Math.min(4, chartDataLengths.length); i++) {
    expect(chartDataLengths[i].hasCanvas, `chart ${i} should have canvas`).toBe(true);
    expect(chartDataLengths[i].hasPixels, `chart ${i} should have rendered pixel data`).toBe(true);
  }

  // Verify no critical errors occurred (e.g., DuckDB insert failures from undefined values)
  const criticalErrors = consoleLogs.filter((l) =>
    l.includes("undefined") && (l.includes("INSERT") || l.includes("DuckDB"))
  );
  expect(criticalErrors, `Critical errors found: ${criticalErrors.join(", ")}`).toHaveLength(0);
});

test("state messages include Keplerian elements", async ({ page }) => {
  await page.goto(VIEWER_URL);

  const keplerian = await page.evaluate(async (url) => {
    return new Promise<Record<string, boolean>>((resolve) => {
      const ws = new WebSocket(url);
      ws.addEventListener("message", (e) => {
        try {
          const msg = JSON.parse(e.data as string);
          if (msg.type === "state") {
            ws.close();
            resolve({
              has_semi_major_axis: typeof msg.semi_major_axis === "number",
              has_eccentricity: typeof msg.eccentricity === "number",
              has_inclination: typeof msg.inclination === "number",
              has_raan: typeof msg.raan === "number",
              has_argument_of_periapsis: typeof msg.argument_of_periapsis === "number",
              has_true_anomaly: typeof msg.true_anomaly === "number",
            });
          }
        } catch {
          // ignore
        }
      });
      setTimeout(() => {
        ws.close();
        resolve({});
      }, 10000);
    });
  }, WS_URL);

  expect(keplerian.has_semi_major_axis, "state must include semi_major_axis").toBe(true);
  expect(keplerian.has_eccentricity, "state must include eccentricity").toBe(true);
  expect(keplerian.has_inclination, "state must include inclination").toBe(true);
  expect(keplerian.has_raan, "state must include raan").toBe(true);
  expect(keplerian.has_argument_of_periapsis, "state must include argument_of_periapsis").toBe(true);
  expect(keplerian.has_true_anomaly, "state must include true_anomaly").toBe(true);
});

test("history message arrives after info before state", async ({ page }) => {
  await page.goto(VIEWER_URL);

  const messageTypes = await page.evaluate(async (url) => {
    return new Promise<string[]>((resolve) => {
      const ws = new WebSocket(url);
      const types: string[] = [];

      ws.addEventListener("message", (e) => {
        try {
          const msg = JSON.parse(e.data as string);
          types.push(msg.type);
          // Collect until we see at least info + history + one state
          if (
            types.includes("info") &&
            types.includes("history") &&
            types.filter((t) => t === "state").length >= 1
          ) {
            ws.close();
          }
        } catch {
          // ignore
        }
      });

      ws.addEventListener("close", () => resolve(types));
      ws.addEventListener("error", () => resolve(types));

      setTimeout(() => {
        ws.close();
        resolve(types);
      }, 10000);
    });
  }, WS_URL);

  console.log("Message types:", messageTypes);

  // First message must be info
  expect(messageTypes[0]).toBe("info");
  // Second message must be history
  expect(messageTypes[1]).toBe("history");
  // After that, should have state (possibly with history_detail interleaved)
  expect(messageTypes.some((t) => t === "state")).toBe(true);
});

test("3D scene renders orbit trails and satellite markers", async ({ page }) => {
  const consoleLogs: string[] = [];
  page.on("console", (msg) => consoleLogs.push(`[${msg.type()}] ${msg.text()}`));

  await page.goto(VIEWER_URL);

  // Wait for WebSocket connection
  const statusText = page.locator(".ws-status-text");
  await expect(statusText).toHaveText("Connected", { timeout: 10000 });

  // Wait for enough data to stream so trails are visible
  await page.waitForTimeout(4000);

  // Verify Three.js canvas exists and is rendering.
  // We use Playwright's screenshot (captures composited output) instead of readPixels
  // because Three.js defaults to preserveDrawingBuffer=false which clears the GL buffer.
  const canvas = page.locator("canvas[data-engine]");
  await expect(canvas).toBeVisible({ timeout: 5000 });

  const engine = await canvas.getAttribute("data-engine");
  expect(engine, "canvas should have three.js engine").toContain("three.js");

  // Take a screenshot of the canvas and verify it has non-uniform pixels
  // (Playwright captures composited output regardless of preserveDrawingBuffer).
  const screenshot = await canvas.screenshot();
  expect(screenshot.byteLength, "canvas screenshot should not be empty").toBeGreaterThan(0);

  // Decode PNG to check for non-uniform content: a blank/black canvas compresses
  // to a very small PNG. A rendered scene with lights, meshes, and trails is larger.
  // Empirically, a blank 1280x720 canvas PNG is ~2-5 KB, a rendered scene is >10 KB.
  console.log("Canvas screenshot size:", screenshot.byteLength, "bytes");
  expect(screenshot.byteLength, "canvas should have rendered content (not blank)").toBeGreaterThan(5000);
});
