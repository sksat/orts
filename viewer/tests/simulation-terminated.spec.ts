/**
 * E2E test: viewer continues for surviving satellite after one terminates.
 *
 * Uses a mock WebSocket server to send controlled messages including
 * simulation_terminated, verifying the viewer's handling of the full
 * communication path.
 */
import { test, expect } from "@playwright/test";
import { WebSocketServer, type WebSocket as WsSocket } from "ws";
import type { AddressInfo } from "net";

/** Build a minimal state message for a given satellite. */
function stateMsg(satelliteId: string, t: number) {
  const r = 6778;
  const v = 7.669;
  const theta = (t / 5554) * 2 * Math.PI;
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

test.describe("simulation_terminated handling", () => {
  let wss: WebSocketServer;

  test.beforeEach(async () => {
    wss = new WebSocketServer({ port: 0 });
  });

  test.afterEach(async () => {
    wss.close();
  });

  test("surviving satellite continues after peer terminates", async ({ page }) => {
    const consoleLogs: string[] = [];
    page.on("console", (msg) => consoleLogs.push(msg.text()));

    // Set up mock server that sends: info → history → states → termination → more states
    wss.on("connection", (ws: WsSocket) => {
      // 1. info message with two satellites
      ws.send(JSON.stringify({
        type: "info",
        mu: 398600.4418,
        dt: 10,
        output_interval: 10,
        stream_interval: 10,
        central_body: "earth",
        central_body_radius: 6378.137,
        epoch_jd: null,
        satellites: [
          { id: "leo", name: "LEO", altitude: 200, period: 5310 },
          { id: "sso", name: "SSO", altitude: 800, period: 6052 },
        ],
      }));

      // 2. empty history
      ws.send(JSON.stringify({ type: "history", states: [] }));

      // 3. Stream state messages for both satellites
      let t = 0;
      const interval = setInterval(() => {
        if (ws.readyState !== 1) { clearInterval(interval); return; }

        t += 10;
        // LEO terminates at t=100
        if (t === 100) {
          ws.send(JSON.stringify({
            type: "simulation_terminated",
            satellite_id: "leo",
            t: 100,
            reason: "atmospheric_entry",
          }));
        }

        // SSO continues past termination
        ws.send(stateMsg("sso", t));

        if (t < 100) {
          ws.send(stateMsg("leo", t));
        }
      }, 50); // Send every 50ms for fast test
    });

    // Navigate and set WS URL to mock server
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

    // Wait for the termination to happen and more data to stream
    await page.waitForTimeout(3000);

    // Verify the console log from handleSimulationTerminated
    const terminationLog = consoleLogs.find((l) => l.includes("Satellite leo terminated"));
    expect(terminationLog, "should log termination event").toBeTruthy();
    expect(terminationLog).toContain("atmospheric_entry");

    // Verify the viewer didn't crash (UI overlay still exists)
    const uiOverlay = page.locator(".ui-overlay");
    await expect(uiOverlay).toBeVisible();

    // Verify data is still being displayed (points counter should be > 0 and growing)
    const pointsInfo = page.locator(".orbit-info").last();
    const pointsText = await pointsInfo.textContent();
    expect(pointsText).toContain("points");

    // Take snapshot and verify the points count is substantial
    // (SSO data continued after LEO terminated)
    const pointCount = parseInt(pointsText?.match(/(\d+)\s+points/)?.[1] ?? "0", 10);
    expect(pointCount, "SSO satellite should still have accumulated points").toBeGreaterThan(10);
  });
});
