import { type ChildProcess, spawn } from "node:child_process";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// These tests require an idle server (no pre-started simulation).
// Skip when ORTS_WS_URL is set (CI uses a pre-started running server).
test.skip(!!process.env.ORTS_WS_URL, "Requires idle server; CI uses pre-started running server");

let ortsProcess: ChildProcess | undefined;
let wsUrl: string;

/** Start orts in idle mode (no simulation args). */
test.beforeAll(async () => {
  const binary = process.env.ORTS_BINARY ?? path.resolve(__dirname, "../../target/debug/orts");
  const child = spawn(binary, ["serve", "--port", "0"]);
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
  console.log(`orts idle server started at ${wsUrl}`);
});

test.afterAll(async () => {
  if (ortsProcess && !ortsProcess.killed) {
    ortsProcess.kill("SIGTERM");
  }
});

/** Connect to the idle test server. */
async function connectToIdleServer(page: import("@playwright/test").Page) {
  await page.goto("/");

  // Disconnect from auto-connected default server if needed
  const disconnectBtn = page.locator(".ws-disconnect-btn");
  try {
    await disconnectBtn.waitFor({ state: "visible", timeout: 3000 });
    await disconnectBtn.click();
  } catch {
    // Not connected; continue
  }

  const urlInput = page.locator(".ws-url-input");
  await urlInput.fill(wsUrl);
  const connectBtn = page.locator(".ws-connect-btn");
  await connectBtn.click();
}

test("idle server sends status message and viewer shows idle state", async ({ page }) => {
  await connectToIdleServer(page);

  // Should show "Connected (Idle)"
  const statusText = page.locator(".ws-status-text");
  await expect(statusText).toHaveText("Connected (Idle)", { timeout: 10000 });

  // SimConfigForm should be visible
  const form = page.locator(".sim-config-form");
  await expect(form).toBeVisible({ timeout: 5000 });

  // Preset buttons should exist
  const presetBtns = page.locator(".preset-btn");
  expect(await presetBtns.count()).toBe(3);

  // Start button should exist
  const startBtn = page.locator(".sim-config-start-btn");
  await expect(startBtn).toBeVisible();
  await expect(startBtn).toHaveText("Start Simulation");
});

test("start simulation from preset transitions to running state", async ({ page }) => {
  await connectToIdleServer(page);

  // Wait for idle state
  const statusText = page.locator(".ws-status-text");
  await expect(statusText).toHaveText("Connected (Idle)", { timeout: 10000 });

  // Select ISS preset (first one, should be active by default)
  const presetBtns = page.locator(".preset-btn");
  await expect(presetBtns.first()).toHaveClass(/active/);

  // Click start
  const startBtn = page.locator(".sim-config-start-btn");
  await startBtn.click();

  // Should transition to "Connected" (running)
  await expect(statusText).toHaveText("Connected", { timeout: 10000 });

  // SimConfigForm should disappear
  const form = page.locator(".sim-config-form");
  await expect(form).not.toBeVisible({ timeout: 5000 });

  // orbit-info should appear with simulation metadata
  const orbitInfo = page.locator(".orbit-info");
  await expect(orbitInfo.first()).toBeVisible({ timeout: 10000 });
});

test("pause and resume simulation", async ({ page }) => {
  // Server is running from the previous test
  await connectToIdleServer(page);

  const statusText = page.locator(".ws-status-text");
  // Server should be running (not idle) from the previous test
  await expect(statusText).toHaveText("Connected", { timeout: 10000 });

  // Pause button should be visible
  const pauseBtn = page.locator(".sim-pause-btn");
  await expect(pauseBtn).toBeVisible({ timeout: 5000 });

  // Click pause
  await pauseBtn.click();

  // Should show "Connected (Paused)"
  await expect(statusText).toHaveText("Connected (Paused)", { timeout: 10000 });

  // Resume button should appear
  const resumeBtn = page.locator(".sim-resume-btn");
  await expect(resumeBtn).toBeVisible({ timeout: 5000 });

  // Click resume
  await resumeBtn.click();

  // Should show "Connected" again
  await expect(statusText).toHaveText("Connected", { timeout: 10000 });

  // Pause button should be back
  await expect(pauseBtn).toBeVisible({ timeout: 5000 });
});

test("terminate simulation returns to idle and allows restart", async ({ page }) => {
  // Server is running from the previous tests
  await connectToIdleServer(page);

  const statusText = page.locator(".ws-status-text");
  await expect(statusText).toHaveText("Connected", { timeout: 10000 });

  // Click stop/terminate
  const terminateBtn = page.locator(".sim-terminate-btn");
  await expect(terminateBtn).toBeVisible({ timeout: 5000 });
  await terminateBtn.click();

  // Should return to idle
  await expect(statusText).toHaveText("Connected (Idle)", { timeout: 10000 });

  // SimConfigForm should be visible again
  const form = page.locator(".sim-config-form");
  await expect(form).toBeVisible({ timeout: 5000 });

  // Start a new simulation
  const startBtn = page.locator(".sim-config-start-btn");
  await startBtn.click();

  // Should transition back to running
  await expect(statusText).toHaveText("Connected", { timeout: 10000 });
  await expect(form).not.toBeVisible({ timeout: 5000 });
});
