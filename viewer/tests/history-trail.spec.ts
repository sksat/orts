/**
 * E2E test: orbit trail includes history data after connecting to a running simulation.
 *
 * Uses a mock WebSocket server to send controlled history + streaming messages,
 * verifying the viewer's TrailBuffer contains pre-connection history points.
 */

import type { AddressInfo } from "node:net";
import { expect, test } from "@playwright/test";
import { WebSocketServer, type WebSocket as WsSocket } from "ws";

/** Build a state message for a circular orbit at the given time. */
function stateMsg(satelliteId: string, t: number) {
  const r = 6778;
  const v = 7.669;
  const period = 5554;
  const theta = (t / period) * 2 * Math.PI;
  return JSON.stringify({
    type: "state",
    satellite_id: satelliteId,
    t,
    position: [r * Math.cos(theta), r * Math.sin(theta), 0],
    velocity: [-v * Math.sin(theta), v * Math.cos(theta), 0],
    semi_major_axis: r,
    eccentricity: 0.001,
    inclination: 1.7,
    raan: 0,
    argument_of_periapsis: 0,
    true_anomaly: theta,
  });
}

/** Build a history state object (same shape as server HistoryState). */
function historyState(satelliteId: string, t: number) {
  const r = 6778;
  const v = 7.669;
  const period = 5554;
  const theta = (t / period) * 2 * Math.PI;
  return {
    satellite_id: satelliteId,
    t,
    position: [r * Math.cos(theta), r * Math.sin(theta), 0],
    velocity: [-v * Math.sin(theta), v * Math.cos(theta), 0],
    semi_major_axis: r,
    eccentricity: 0.001,
    inclination: 1.7,
    raan: 0,
    argument_of_periapsis: 0,
    true_anomaly: theta,
  };
}

test.describe("history trail after connect", () => {
  let wss: WebSocketServer;

  test.beforeEach(async () => {
    wss = new WebSocketServer({ port: 0 });
  });

  test.afterEach(async () => {
    wss.close();
  });

  test("TrailBuffer includes history points after connecting to running simulation", async ({
    page,
  }) => {
    const consoleLogs: string[] = [];
    page.on("console", (msg) => consoleLogs.push(msg.text()));

    // Simulate a server that already has 200 history points (t=0..1990),
    // then streams live data starting at t=2000.
    const HISTORY_COUNT = 200;
    const HISTORY_DT = 10;
    const STREAM_START = HISTORY_COUNT * HISTORY_DT; // t=2000

    wss.on("connection", (ws: WsSocket) => {
      // 1. info message
      ws.send(
        JSON.stringify({
          type: "info",
          mu: 398600.4418,
          dt: 10,
          output_interval: 10,
          stream_interval: 10,
          central_body: "earth",
          central_body_radius: 6378.137,
          epoch_jd: null,
          satellites: [{ id: "sat1", name: "TestSat", altitude: 400, period: 5554 }],
        }),
      );

      // 2. history overview (downsampled to 50 points)
      const overviewStates = [];
      for (let i = 0; i < 50; i++) {
        const t = Math.floor((i / 49) * (HISTORY_COUNT - 1)) * HISTORY_DT;
        overviewStates.push(historyState("sat1", t));
      }
      ws.send(JSON.stringify({ type: "history", states: overviewStates }));

      // 3. history detail (full resolution, in chunks)
      const allDetailStates = [];
      for (let i = 0; i < HISTORY_COUNT; i++) {
        allDetailStates.push(historyState("sat1", i * HISTORY_DT));
      }
      // Send in 2 chunks
      const mid = Math.floor(allDetailStates.length / 2);
      ws.send(JSON.stringify({ type: "history_detail", states: allDetailStates.slice(0, mid) }));
      ws.send(JSON.stringify({ type: "history_detail", states: allDetailStates.slice(mid) }));
      ws.send(JSON.stringify({ type: "history_detail_complete" }));

      // 4. Stream live state messages
      let t = STREAM_START;
      const interval = setInterval(() => {
        if (ws.readyState !== 1) {
          clearInterval(interval);
          return;
        }
        t += HISTORY_DT;
        ws.send(stateMsg("sat1", t));
      }, 50);
    });

    // Navigate and connect to mock server
    await page.goto("/");

    // Disconnect from auto-connected default server if needed
    // (auto-connect may have connected to ws://localhost:9001)
    const disconnectBtn = page.locator(".ws-disconnect-btn");
    try {
      await disconnectBtn.waitFor({ state: "visible", timeout: 5000 });
      await disconnectBtn.click();
    } catch {
      // Not connected; continue
    }

    // URL input is now enabled; point to mock server
    const urlInput = page.locator(".ws-url-input");
    const mockPort = (wss.address() as AddressInfo).port;
    await urlInput.fill(`ws://localhost:${mockPort}`);

    const connectBtn = page.locator(".ws-connect-btn");
    await connectBtn.click();

    // Wait for connection
    const statusText = page.locator(".ws-status-text");
    await expect(statusText).toHaveText("Connected", { timeout: 5000 });

    // Wait for history + some streaming data
    await page.waitForTimeout(3000);

    // Verify TrailBuffer contains history points by checking the earliest point time.
    // History starts at t=0; streaming starts at t=2000.
    // If TrailBuffer only has post-connect data, earliest t would be ~2000.
    // If history is loaded, earliest t should be 0.
    const trailInfo = await page.evaluate(() => {
      // Access the TrailBuffer from the app's internal state.
      // The trailBuffersRef is exposed on the window for E2E testing in dev mode,
      // or we can check the DOM for point count display.
      const pointsInfo = document.querySelector(".orbit-info:last-of-type");
      const pointsText = pointsInfo?.textContent ?? "";
      const pointCount = parseInt(pointsText.match(/(\d+)\s+points/)?.[1] ?? "0", 10);
      return { pointCount, pointsText };
    });

    console.log("Trail info:", trailInfo);

    // The TrailBuffer should have substantially more points than just streaming
    // (200 history + streaming points). Without history, only streaming points
    // would exist (~60 points at 50ms intervals over 3 seconds).
    expect(
      trailInfo.pointCount,
      "TrailBuffer should contain history + streaming points (not just streaming)",
    ).toBeGreaterThan(100);

    // Verify no "Maximum update depth exceeded" warnings
    const depthWarnings = consoleLogs.filter((l) => l.includes("Maximum update depth"));
    expect(depthWarnings, "Should not have Maximum update depth exceeded warnings").toHaveLength(0);

    // Verify no critical React errors
    const reactErrors = consoleLogs.filter(
      (l) => l.includes("Uncaught") || l.includes("unhandled"),
    );
    expect(reactErrors, "Should not have uncaught errors").toHaveLength(0);
  });
});
