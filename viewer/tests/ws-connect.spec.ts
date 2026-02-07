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

  await page.screenshot({ path: "/tmp/claude-1000/-home-sksat-prog-orts/d2b40273-c290-43a8-8982-658581609b13/scratchpad/viewer-connected.png" });

  // Check if the page crashed (UI overlay gone)
  const uiOverlay = page.locator(".ui-overlay");
  const overlayCount = await uiOverlay.count();
  console.log("UI overlay elements:", overlayCount);

  expect(overlayCount, "UI overlay should still exist (React did not crash)").toBe(1);
});
