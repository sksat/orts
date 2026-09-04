/**
 * E2E for the app's attitude view: the switch, what the view drops, and the
 * rendered orientation in each display frame.
 *
 * The same reference attitude as `attitude-rendering.spec.ts` — a +90° rotation
 * about the inertial Z axis on an equatorial circular orbit — so the invariants
 * are the same ones, asserted in a scene that has no central body:
 *
 * - inertial: scene axes coincide with ECI, so the rendered body +X points along
 *   scene +Y.
 * - local-orbital: the basis is [in-track, cross-track, radial] = scene [X, Y, Z]
 *   and the orbit is equatorial, so the cross-track axis is inertial +Z. The
 *   attitude is a rotation *about* inertial Z, so the body +Z stays on inertial
 *   +Z: the rendered body +Z points along scene +Y. Nadir is exactly scene -Z.
 *
 * Read from the live Three.js scene graph rather than from pixels, and stated as
 * frame invariants rather than raw quaternions, so the ±q sign ambiguity and the
 * camera framing cannot affect the result.
 */
import { type ChildProcess, spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { expect, type Page, test } from "@playwright/test";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

const SAT_ID = "att-view";
/** Entity path the viewer keys the satellite by (`/world/sat/{id}`). */
const SAT_ENTITY_PATH = `/world/sat/${SAT_ID}`;

/** sin(45°) = cos(45°): components of a +90° rotation quaternion. */
const SQRT_HALF = Math.SQRT1_2;

let ortsProcess: ChildProcess | undefined;
let wsUrl: string;
let configPath: string;

function spawnServer(configFile: string): Promise<{ child: ChildProcess; port: number }> {
  const binary = process.env.ORTS_BINARY ?? path.resolve(__dirname, "../../target/debug/orts");
  const child = spawn(binary, ["serve", "--port", "0", "--config", configFile], {
    // stderr is where the server announces its URL. Named explicitly so the
    // reader below cannot end up on the runner's own stdin, which never ends.
    stdio: ["ignore", "ignore", "pipe"],
    env: { ...process.env, ORTS_DISABLE_TEXTURE_DOWNLOAD: "1" },
  });
  return new Promise((resolve, reject) => {
    const serverLog = child.stderr;
    if (serverLog == null) {
      reject(new Error("orts serve was spawned without a stderr pipe"));
      return;
    }
    const rl = createInterface({ input: serverLog });
    const onError = (err: Error) => {
      settle();
      reject(err);
    };
    const onExit = (code: number | null) => {
      settle();
      reject(new Error(`orts exited with code ${code} before listening`));
    };
    const timeout = setTimeout(() => {
      settle();
      // Nothing else will: `beforeAll` throws before it can record the child, so
      // `afterAll` has nothing to kill and the server would outlive the run.
      child.kill("SIGTERM");
      reject(new Error("Timed out waiting for orts server to start"));
    }, 30000);
    /**
     * Stop watching for the startup line. The readline interface stays attached
     * on purpose: it is what drains the child's stderr, and a paused pipe fills
     * up and blocks the server on its next log line.
     */
    function settle() {
      clearTimeout(timeout);
      rl.removeAllListeners("line");
      child.off("error", onError);
      child.off("exit", onExit);
    }
    rl.on("line", (line) => {
      const match = line.match(/ws:\/\/localhost:(\d+)/);
      if (match) {
        settle();
        resolve({ child, port: parseInt(match[1], 10) });
      }
    });
    child.on("error", onError);
    child.on("exit", onExit);
  });
}

test.beforeAll(async () => {
  const config = {
    body: "earth",
    dt: 1.0,
    output_interval: 10,
    satellites: [
      {
        id: SAT_ID,
        name: "Attitude View Test",
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
  configPath = path.join(os.tmpdir(), `orts-attitude-view-e2e-${Date.now()}.json`);
  fs.writeFileSync(configPath, JSON.stringify(config));

  const { child, port } = await spawnServer(configPath);
  ortsProcess = child;
  wsUrl = `ws://localhost:${port}/ws`;
  console.log(`orts attitude-view server started at ${wsUrl}`);
});

test.afterAll(async () => {
  if (ortsProcess && !ortsProcess.killed) ortsProcess.kill("SIGTERM");
  if (configPath) {
    try {
      fs.unlinkSync(configPath);
    } catch {
      // ignore
    }
  }
});

async function connect(page: Page) {
  await page.locator('[data-testid="ws-url-input"]').fill(wsUrl);
  await page.locator('[data-testid="ws-connect-btn"]').click();
  await expect(page.locator('[data-testid="ws-status-text"]')).toContainText("Connected", {
    timeout: 15000,
  });
  await expect(page.locator("canvas").first()).toBeVisible();
}

/** Rendered world quaternion of the spacecraft, once the scene graph has it. */
async function worldQuat(page: Page): Promise<[number, number, number, number]> {
  const quat = await page.evaluate(async (id) => {
    const w = window as unknown as Record<string, unknown>;
    // Re-read the hook each pass: it is installed when the first body axes mount,
    // which can be after the connection is up.
    for (let i = 0; i < 40; i++) {
      const get = w.__debug_get_sat_world_quat as
        | ((id: string) => [number, number, number, number] | null)
        | undefined;
      const q = get?.(id) ?? null;
      if (q) return q;
      await new Promise((r) => setTimeout(r, 100));
    }
    return null;
  }, SAT_ENTITY_PATH);
  expect(quat, "the rendered world quaternion should be exposed").not.toBeNull();
  if (quat == null) throw new Error("unreachable");
  return quat;
}

/** World direction of a body axis, from the rendered quaternion (x, y, z, w). */
function bodyAxisInWorld(
  [qx, qy, qz, qw]: [number, number, number, number],
  axis: "x" | "z",
): [number, number, number] {
  if (axis === "x") {
    return [1 - 2 * (qy * qy + qz * qz), 2 * (qx * qy + qz * qw), 2 * (qx * qz - qy * qw)];
  }
  return [2 * (qx * qz + qy * qw), 2 * (qy * qz - qx * qw), 1 - 2 * (qx * qx + qy * qy)];
}

test("the attitude view drops the central body, the trails and the charts", async ({ page }) => {
  await page.goto("/?noAutoConnect=1");
  await connect(page);

  // The orbit view has all three. Establishing that first is what makes the
  // absences below evidence of the attitude view dropping them, rather than of a
  // debug hook that was never installed.
  await expect(page.locator('[data-testid="time-series-chart"]').first()).toBeVisible({
    timeout: 30000,
  });
  await page.waitForFunction(
    (id) => {
      const w = window as Record<string, unknown>;
      const debug = w.__debug_orbit_viewer as { trail?: (id: string) => unknown } | undefined;
      return (
        typeof w.__debug_get_earth_world_quat === "function" &&
        typeof debug?.trail === "function" &&
        debug.trail(id) != null
      );
    },
    SAT_ENTITY_PATH,
    { timeout: 15000 },
  );

  await page.locator('[data-testid="view-attitude"]').click();
  await expect(page.locator('[data-testid="attitude-info"]')).toBeVisible();

  // The Earth mesh and the trail buffers deregister their debug hooks on unmount,
  // so their absence is the scene actually having dropped them.
  await page.waitForFunction(
    () => {
      const w = window as Record<string, unknown>;
      return w.__debug_get_earth_world_quat === undefined && w.__debug_orbit_viewer === undefined;
    },
    { timeout: 15000 },
  );
  await expect(page.locator('[data-testid="time-series-chart"]')).toHaveCount(0);

  // The switch is recorded in the URL, so a reload or a shared link keeps it.
  expect(page.url()).toContain("view=attitude");
  await page.reload();
  await expect(page.locator('[data-testid="attitude-info"]')).toBeVisible();
  await connect(page);

  // Switching back restores the orbit view.
  await page.locator('[data-testid="view-orbit"]').click();
  await page.waitForFunction(
    () => typeof (window as Record<string, unknown>).__debug_get_earth_world_quat === "function",
    { timeout: 15000 },
  );
  await expect(page.locator('[data-testid="time-series-chart"]').first()).toBeVisible({
    timeout: 30000,
  });
  expect(page.url()).not.toContain("view=attitude");
});

test("switching from a satellite-centred orbit view carries that satellite over", async ({
  page,
}) => {
  await page.goto("/?noAutoConnect=1");
  await connect(page);

  await page
    .locator('[data-testid="frame-selector-select"]')
    .selectOption(`satellite:${SAT_ENTITY_PATH}`);
  await page.locator('[data-testid="view-attitude"]').click();

  // The spacecraft the reader was already centred on is the one shown.
  await expect(page.locator('[data-testid="attitude-spacecraft-select"]')).toHaveValue(
    SAT_ENTITY_PATH,
  );
});

test("inertial: the rendered body +X points along scene +Y", async ({ page }) => {
  await page.goto("/?noAutoConnect=1&view=attitude");
  await connect(page);
  await expect(page.locator('[data-testid="attitude-info"]')).toBeVisible();
  await expect(page.locator('[data-testid="attitude-orientation-inertial"]')).toBeVisible();

  const bodyX = bodyAxisInWorld(await worldQuat(page), "x");
  expect(
    bodyX[1],
    `body +X should point along scene +Y for a +90°-about-Z attitude, got ` +
      `(${bodyX.map((c) => c.toFixed(3)).join(", ")})`,
  ).toBeGreaterThan(0.9);
  expect(Math.abs(bodyX[0]), "body +X should have no scene-X component").toBeLessThan(0.1);
  expect(Math.abs(bodyX[2]), "body +X should have no scene-Z component").toBeLessThan(0.1);
});

test("local-orbital: the attitude and the nadir arrow share the basis", async ({ page }) => {
  await page.goto("/?noAutoConnect=1&view=attitude");
  await connect(page);
  await page.locator('[data-testid="attitude-orientation-lvlh"]').click();

  // Poll until the local-orbital basis is in effect: it needs both a position and
  // a velocity for the spacecraft, which arrive with the stream.
  const nadir = await page.evaluate(async (id) => {
    const w = window as unknown as Record<string, unknown>;
    for (let i = 0; i < 60; i++) {
      const get = w.__debug_get_direction_vectors as
        | ((id: string) => { kind: string; direction: [number, number, number] }[] | null)
        | undefined;
      const drawn = get?.(id) ?? null;
      const found = drawn?.find((v) => v.kind === "nadir");
      // -Z only once the basis is local-orbital; in inertial it points at the
      // central body in ECI axes instead.
      if (found && found.direction[2] < -0.99) return found.direction;
      await new Promise((r) => setTimeout(r, 100));
    }
    const get = w.__debug_get_direction_vectors as
      | ((id: string) => { kind: string; direction: [number, number, number] }[] | null)
      | undefined;
    return get?.(id)?.find((v) => v.kind === "nadir")?.direction ?? null;
  }, SAT_ENTITY_PATH);

  expect(nadir, "the nadir arrow should be drawn").not.toBeNull();
  if (nadir == null) return;
  expect(nadir[2], "nadir should point along scene -Z in the local-orbital basis").toBeLessThan(
    -0.99,
  );

  // Same basis for the spacecraft itself: the attitude rotates about inertial Z,
  // the equatorial orbit's cross-track axis *is* inertial Z, and cross-track is
  // scene Y — so the rendered body +Z must point along scene +Y.
  const bodyZ = bodyAxisInWorld(await worldQuat(page), "z");
  expect(
    bodyZ[1],
    `body +Z should point along scene +Y in the local-orbital basis, got ` +
      `(${bodyZ.map((c) => c.toFixed(3)).join(", ")}). Scene +Z would mean the attitude was ` +
      `built in RSW order while the arrows and positions use [in-track, cross-track, radial].`,
  ).toBeGreaterThan(0.9);
});

test("turning an arrow off stops it being drawn", async ({ page }) => {
  await page.goto("/?noAutoConnect=1&view=attitude");
  await connect(page);

  const kinds = async () =>
    page.evaluate(async (id) => {
      const w = window as unknown as Record<string, unknown>;
      for (let i = 0; i < 40; i++) {
        const get = w.__debug_get_direction_vectors as
          | ((id: string) => { kind: string }[] | null)
          | undefined;
        const drawn = get?.(id) ?? null;
        if (drawn != null) return drawn.map((v) => v.kind).sort();
        await new Promise((r) => setTimeout(r, 100));
      }
      return null;
    }, SAT_ENTITY_PATH);

  expect(await kinds()).toEqual(["nadir", "sun"]);

  await page.locator('[data-testid="direction-vector-sun"]').click();
  await expect.poll(async () => await kinds(), { timeout: 10000 }).toEqual(["nadir"]);
});
