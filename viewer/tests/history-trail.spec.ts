/**
 * E2E test: orbit trail is enriched after connecting to a running simulation.
 *
 * Contract under test (post-HistoryDetail removal):
 * 1. On connect the server ships a small bounded `history` overview.
 * 2. The viewer, when it has a finite `timeRange` selected, proactively
 *    fires a `query_range` request for the display window.
 * 3. The server responds with higher-resolution data for that window.
 * 4. The viewer clears + rebuilds the trail buffer from the merged
 *    (response + streaming tail) points, so the TrailBuffer ends up with
 *    substantially more points than the sparse overview would give on
 *    its own.
 *
 * Uses a mock WebSocket server that speaks the new protocol.
 */

import type { AddressInfo } from "node:net";
import { expect, test } from "@playwright/test";
import { WebSocketServer, type WebSocket as WsSocket } from "ws";

/** Build a state message for a circular orbit at the given time. */
function stateMsg(entityPath: string, t: number) {
  const r = 6778;
  const v = 7.669;
  const period = 5554;
  const theta = (t / period) * 2 * Math.PI;
  return JSON.stringify({
    type: "state",
    entity_path: entityPath,
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
function historyState(entityPath: string, t: number) {
  const r = 6778;
  const v = 7.669;
  const period = 5554;
  const theta = (t / period) * 2 * Math.PI;
  return {
    entity_path: entityPath,
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

  test("TrailBuffer is enriched with history + query_range response on connect", async ({
    page,
  }) => {
    const consoleLogs: string[] = [];
    page.on("console", (msg) => consoleLogs.push(msg.text()));

    // Simulate a server that already has 200 history points (t=0..1990),
    // then streams live data starting at t=2000. The initial bounded
    // overview is 50 points; the viewer's proactive query_range on
    // connect should pull a denser set for the current time range.
    const HISTORY_COUNT = 200;
    const HISTORY_DT = 10;
    const STREAM_START = HISTORY_COUNT * HISTORY_DT; // t=2000
    const OVERVIEW_COUNT = 50;

    // Full-resolution points the server will return to a query_range
    // request. 200 dense points covering the pre-streaming window.
    const detailStates: ReturnType<typeof historyState>[] = [];
    for (let i = 0; i < HISTORY_COUNT; i++) {
      detailStates.push(historyState("sat1", i * HISTORY_DT));
    }

    let queryRangeRequestSeen = false;

    wss.on("connection", (ws: WsSocket) => {
      // 1. info
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

      // 2. history (bounded downsampled overview — no HistoryDetail follow-up)
      const overviewStates = [];
      for (let i = 0; i < OVERVIEW_COUNT; i++) {
        const t = Math.floor((i / (OVERVIEW_COUNT - 1)) * (HISTORY_COUNT - 1)) * HISTORY_DT;
        overviewStates.push(historyState("sat1", t));
      }
      ws.send(JSON.stringify({ type: "history", states: overviewStates }));

      // 3. respond to client query_range requests with the full-resolution
      //    slice for the requested window (the viewer's proactive initial
      //    query hits this path).
      ws.on("message", (data) => {
        let msg: { type?: string; t_min?: number; t_max?: number };
        try {
          msg = JSON.parse(data.toString());
        } catch {
          return;
        }
        if (msg.type !== "query_range") return;
        queryRangeRequestSeen = true;
        const tMin = msg.t_min ?? 0;
        const tMax = msg.t_max ?? Number.POSITIVE_INFINITY;
        const states = detailStates.filter((s) => s.t >= tMin && s.t <= tMax);
        ws.send(
          JSON.stringify({
            type: "query_range_response",
            t_min: tMin,
            t_max: tMax,
            states,
          }),
        );
      });

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

    // Navigate with auto-connect suppressed and a finite time range so
    // the proactive query_range fires. 10000s is larger than the mock
    // sim history (2000s) so the request covers everything.
    await page.goto("/?noAutoConnect=1&timeRange=10000");

    // Connect to mock server
    const urlInput = page.locator('[data-testid="ws-url-input"]');
    const mockPort = (wss.address() as AddressInfo).port;
    await urlInput.fill(`ws://localhost:${mockPort}`);

    const connectBtn = page.locator('[data-testid="ws-connect-btn"]');
    await connectBtn.click();

    // Wait for connection
    const statusText = page.locator('[data-testid="ws-status-text"]');
    await expect(statusText).toHaveText("Connected", { timeout: 5000 });

    // Wait for history + query_range response + some streaming data.
    await page.waitForTimeout(3000);

    const trailInfo = await page.evaluate(() => {
      const pointsInfo = document.querySelector('[data-testid="orbit-info-points"]');
      const pointsText = pointsInfo?.textContent ?? "";
      const pointCount = parseInt(pointsText.match(/(\d+)\s+points/)?.[1] ?? "0", 10);
      return { pointCount, pointsText };
    });

    console.log("Trail info:", trailInfo);

    // The server-side proactive query_range must have been issued.
    expect(
      queryRangeRequestSeen,
      "client should proactively send query_range after overview arrives",
    ).toBe(true);

    // After the query_range response + streaming, the TrailBuffer should
    // contain the 200 dense detail points plus any streaming points that
    // arrived after the response. The sparse-overview-only path would
    // leave us at ~50 + ~60 streaming = 110 points; the denser path puts
    // us well above that.
    expect(
      trailInfo.pointCount,
      "TrailBuffer should contain the query_range detail + streaming points",
    ).toBeGreaterThan(150);

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
