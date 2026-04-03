/**
 * Regression test: query_range feedback loop prevention.
 *
 * Previously, uPlot's setScale hook fired during programmatic data updates,
 * triggering handleChartZoom → query_range → query_range_response → TrailBuffer
 * rebuild → chart update → loop (~590 requests/sec).
 *
 * This test verifies that during normal live streaming, query_range messages
 * are not sent excessively (at most a small number from initial chart setup).
 */
import { type ChildProcess, spawn } from "node:child_process";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

let ortsProcess: ChildProcess | undefined;
let wsUrl: string;

test.beforeAll(async () => {
  if (process.env.ORTS_WS_URL) {
    wsUrl = process.env.ORTS_WS_URL;
    return;
  }

  const binary = process.env.ORTS_BINARY ?? path.resolve(__dirname, "../../target/debug/orts");
  const child = spawn(binary, ["serve", "--port", "0", "--sat", "altitude=400,id=test"]);
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
        resolve(parseInt(match[1], 10));
      }
    });

    child.on("error", (err) => {
      clearTimeout(timeout);
      reject(err);
    });
    child.on("exit", (code) => {
      clearTimeout(timeout);
      reject(new Error(`orts exited with code ${code} before listening`));
    });
  });

  wsUrl = `ws://localhost:${port}/ws`;
});

test.afterAll(async () => {
  if (ortsProcess && !ortsProcess.killed) {
    ortsProcess.kill("SIGTERM");
  }
});

test("no query_range feedback loop during live streaming", async ({ page }) => {
  await page.goto("/?noAutoConnect=1");

  // Monitor outgoing WebSocket messages by intercepting WebSocket.send
  await page.evaluate(() => {
    const origSend = WebSocket.prototype.send;
    (window as any).__queryRangeCount = 0;
    WebSocket.prototype.send = function (data: string | ArrayBuffer | Blob) {
      try {
        if (typeof data === "string") {
          const msg = JSON.parse(data);
          if (msg.type === "query_range") {
            (window as any).__queryRangeCount++;
          }
        }
      } catch {
        // ignore parse errors
      }
      return origSend.call(this, data);
    };
  });

  // Connect to the test server
  const urlInput = page.locator('[data-testid="ws-url-input"]');
  await urlInput.fill(wsUrl);
  const connectBtn = page.locator('[data-testid="ws-connect-btn"]');
  await connectBtn.click();

  const statusText = page.locator('[data-testid="ws-status-text"]');
  await expect(statusText).toHaveText("Connected", { timeout: 30000 });

  // Wait for DuckDB + charts to initialize and data to stream
  const charts = page.locator('[data-testid="time-series-chart"]');
  await expect(charts.first()).toBeVisible({ timeout: 30000 });

  // Let it stream for 5 seconds to give the feedback loop time to manifest
  await page.waitForTimeout(5000);

  const queryRangeCount = await page.evaluate(() => (window as any).__queryRangeCount);
  console.log(`query_range messages sent in 5 seconds: ${queryRangeCount}`);

  // During normal live streaming without user interaction, query_range should
  // not be sent at all, or at most a small number from initial chart setup.
  // The feedback loop previously caused ~590/sec = ~2950 in 5 seconds.
  expect(
    queryRangeCount,
    `Expected few query_range messages but got ${queryRangeCount} — possible feedback loop`,
  ).toBeLessThan(10);
});
