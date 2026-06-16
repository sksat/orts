/**
 * E2E regression test for the full attitude pipeline: serve → WS → viewer *rendering*.
 *
 * `attitude-data.spec.ts` already proves the server emits an `AttitudePayload` over
 * the wire, but it opens a raw WebSocket and inspects JSON — it never touches the
 * React/Three.js viewer. This test closes Phase E by verifying the *rendered*
 * satellite actually adopts the delivered body-to-inertial attitude.
 *
 * Like the LVLH central-body test (#50, lvlh-orientation.spec.ts), it reads the
 * rendered object's world quaternion from the live Three.js scene graph (exposed
 * via `window.__debug_get_sat_world_quat(id)` in dev/E2E builds) instead of
 * reading pixels, and asserts a frame-invariant rather than the raw quaternion
 * (so it is immune to the ±q sign ambiguity).
 *
 * Scenario: the satellite starts at a known non-identity attitude — a +90° rotation
 * about the inertial Z axis — with zero angular velocity. That rotation maps the
 * body +X axis onto inertial +Y. In the default central-body *inertial* frame the
 * satellite group has no rotating parent (SmoothOriginGroup only translates), so
 * its world quaternion equals the body-to-ECI quaternion, and scene axes coincide
 * with ECI axes. Hence the rendered body +X axis must point along scene +Y.
 *
 * The invariant catches the two seams a unit test on the math alone would miss:
 *   - the Hamilton [w,x,y,z] → Three.js (x,y,z,w) re-ordering in SatelliteModel /
 *     BodyAxes (a swap would point body +X somewhere else), and
 *   - any unexpected parent rotation in the scene compositing.
 *
 * Zero angular velocity keeps the attitude ≈ constant over the short window: from
 * rest the gravity-gradient torque produces only sub-degree drift before the seek
 * time, far inside the tolerance below.
 */
import { type ChildProcess, spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

/** Config satellite id. */
const SAT_ID = "att-test";
/** Entity path the viewer keys the satellite by (`/world/sat/{id}`). */
const SAT_ENTITY_PATH = `/world/sat/${SAT_ID}`;

/** sin(45°) = cos(45°): components of a +90° rotation quaternion. */
const SQRT_HALF = Math.SQRT1_2;

let ortsProcess: ChildProcess | undefined;
let wsUrl: string;
let configPath: string;

/** Spawn an `orts serve` instance and resolve once it prints its ws:// URL. */
function spawnServer(configFile: string): Promise<{ child: ChildProcess; port: number }> {
  const binary = process.env.ORTS_BINARY ?? path.resolve(__dirname, "../../target/debug/orts");
  const child = spawn(binary, ["serve", "--port", "0", "--config", configFile]);
  return new Promise((resolve, reject) => {
    const rl = createInterface({ input: child.stderr ?? process.stdin });
    const timeout = setTimeout(() => {
      rl.close();
      reject(new Error("Timed out waiting for orts server to start"));
    }, 30000);
    rl.on("line", (line) => {
      const match = line.match(/ws:\/\/localhost:(\d+)/);
      if (match) {
        clearTimeout(timeout);
        resolve({ child, port: parseInt(match[1], 10) });
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
}

test.beforeAll(async () => {
  // +90° rotation about the inertial Z axis, body-to-inertial, scalar-first [w,x,y,z].
  const config = {
    central_body: "earth",
    dt: 1.0,
    output_interval: 10,
    satellites: [
      {
        id: SAT_ID,
        name: "Attitude Render Test",
        orbit: { type: "circular", altitude: 500 },
        attitude: {
          mass: 500,
          inertia_diag: [100, 100, 50],
          initial_quaternion: [SQRT_HALF, 0, 0, SQRT_HALF],
          initial_angular_velocity: [0, 0, 0],
        },
      },
    ],
  };
  configPath = path.join(os.tmpdir(), `orts-attitude-render-e2e-${Date.now()}.json`);
  fs.writeFileSync(configPath, JSON.stringify(config));

  const { child, port } = await spawnServer(configPath);
  ortsProcess = child;
  wsUrl = `ws://localhost:${port}/ws`;
  console.log(`orts attitude-render server started at ${wsUrl}`);
});

test.afterAll(async () => {
  if (ortsProcess && !ortsProcess.killed) {
    ortsProcess.kill("SIGTERM");
  }
  if (configPath) {
    try {
      fs.unlinkSync(configPath);
    } catch {
      // ignore
    }
  }
});

test("rendered satellite adopts the delivered body-to-inertial attitude", async ({ page }) => {
  await page.goto("/?noAutoConnect=1");

  // Connect the viewer to the attitude server through the normal UI path.
  await page.locator('[data-testid="ws-url-input"]').fill(wsUrl);
  await page.locator('[data-testid="ws-connect-btn"]').click();
  await expect(page.locator('[data-testid="ws-status-text"]')).toContainText("Connected", {
    timeout: 15000,
  });

  await expect(page.locator("canvas").first()).toBeVisible();

  // Let the stream deliver several attitude samples so the trail buffer has data.
  await page.waitForFunction(
    () =>
      typeof (window as unknown as Record<string, unknown>).__debug_get_sat_world_quat ===
      "function",
    { timeout: 15000 },
  );

  // Deterministic state: pause playback so the captured frame is stable.
  const playPause = page.locator('[data-testid="play-pause-btn"]');
  if ((await playPause.textContent())?.includes("Pause")) {
    await playPause.click();
  }
  await page.waitForTimeout(1000); // let the paused frame render with attitude applied

  // Read the rendered satellite's world quaternion, retrying until the scene graph
  // has the group mounted and matrices updated. Falls back to the first registered
  // satellite if the exact entity-path key changes shape in the future.
  const quat = await page.evaluate(async (id) => {
    const w = window as unknown as Record<string, unknown>;
    const get = w.__debug_get_sat_world_quat as
      | ((id: string) => [number, number, number, number] | null)
      | undefined;
    if (!get) return null;
    for (let i = 0; i < 40; i++) {
      const direct = get(id);
      if (direct) return direct;
      const reg = w.__debug_sat_quat_registry as Map<string, unknown> | undefined;
      const firstKey = reg && reg.size > 0 ? [...reg.keys()][0] : undefined;
      if (firstKey) {
        const q = get(firstKey);
        if (q) return q;
      }
      await new Promise((r) => setTimeout(r, 100));
    }
    return null;
  }, SAT_ENTITY_PATH);

  expect(quat, "rendered satellite world quaternion should be exposed").not.toBeNull();
  if (quat == null) return; // narrows type; unreachable — expect() throws above
  const [qx, qy, qz, qw] = quat; // Three.js order (x, y, z, w)

  // World direction of the body +X axis = R(q)·(1,0,0) = first column of the
  // rotation matrix. For the +90°-about-Z attitude this must be inertial/scene +Y.
  const bodyXinWorld: [number, number, number] = [
    1 - 2 * (qy * qy + qz * qz),
    2 * (qx * qy + qz * qw),
    2 * (qx * qz - qy * qw),
  ];

  expect(
    bodyXinWorld[1],
    `body +X should point along scene +Y for a +90°-about-Z attitude, got ` +
      `(${bodyXinWorld.map((c) => c.toFixed(3)).join(", ")}). A value near 0 means the ` +
      `attitude quaternion never reached the rendered mesh (or the [w,x,y,z]→(x,y,z,w) ` +
      `re-order is wrong).`,
  ).toBeGreaterThan(0.9);
  expect(Math.abs(bodyXinWorld[0]), "body +X should have no scene-X component").toBeLessThan(0.1);
  expect(Math.abs(bodyXinWorld[2]), "body +X should have no scene-Z component").toBeLessThan(0.1);
});

test("marker shape: global default persists to URL and per-satellite override is offered", async ({
  page,
}) => {
  await page.goto("/?noAutoConnect=1");
  await page.locator('[data-testid="ws-url-input"]').fill(wsUrl);
  await page.locator('[data-testid="ws-connect-btn"]').click();
  await expect(page.locator('[data-testid="ws-status-text"]')).toContainText("Connected", {
    timeout: 15000,
  });

  // Expand the (collapsed) Markers panel, then change the global default.
  await page.locator('[data-testid="marker-shape-selector"] summary').click();
  await page.locator('[data-testid="marker-shape-default"]').selectOption("sphere");
  await expect.poll(() => new URL(page.url()).searchParams.get("satShape")).toBe("sphere");

  // A per-satellite override control is offered for the streamed satellite.
  await expect(
    page.locator(`[data-testid="marker-shape-override-${SAT_ENTITY_PATH}"]`),
  ).toBeVisible();
});
