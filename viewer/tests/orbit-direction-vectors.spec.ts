/**
 * E2E for the reference-direction arrows in the *orbit* view.
 *
 * The arrows are drawn only at a centred satellite: this view can hold many
 * satellites, and a pair of arrows on each of them fills the screen; in a
 * central-body view the body itself is on screen, so a nadir arrow repeats what
 * the picture already shows. Both halves of that rule are asserted here.
 *
 * Directions are read from the live Three.js scene graph via
 * `window.__debug_get_direction_vectors(id)`, which measures each arrow as the
 * unit vector from its origin to its head. Reading the resolved input instead
 * would pass even with the geometry's own axis, the rotation onto it, or the
 * head's placement wrong — and an arrow is a few triangles on a dark background,
 * so a pixel test on it would fail for reasons unrelated to the geometry.
 *
 * The satellite is on an equatorial circular orbit, so in the satellite-centred
 * local-orbital view — whose basis is [in-track, cross-track, radial] = scene
 * [X, Y, Z] — nadir is exactly scene -Z, at every orbital phase. That invariant
 * is what catches a nadir arrow built from a different axis order than the
 * positions use.
 */
import { type ChildProcess, spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

const SAT_ID = "dir-test";
/** Entity path the viewer keys the satellite by (`/world/sat/{id}`). */
const SAT_ENTITY_PATH = `/world/sat/${SAT_ID}`;

interface DrawnVector {
  kind: string;
  direction: [number, number, number];
  /** Distance from the arrow's origin to its head's centre, in scene units. */
  distance: number;
}

let ortsProcess: ChildProcess | undefined;
let wsUrl: string;
let configPath: string;

/** Spawn an `orts serve` instance and resolve once it prints its ws:// URL. */
function spawnServer(configFile: string): Promise<{ child: ChildProcess; port: number }> {
  const binary = process.env.ORTS_BINARY ?? path.resolve(__dirname, "../../target/debug/orts");
  const child = spawn(binary, ["serve", "--port", "0", "--config", configFile], {
    env: { ...process.env, ORTS_DISABLE_TEXTURE_DOWNLOAD: "1" },
  });
  return new Promise((resolve, reject) => {
    const rl = createInterface({ input: child.stderr ?? process.stdin });
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
        name: "Direction Vector Test",
        orbit: { type: "circular", altitude: 500 },
        attitude: {
          mass: 500,
          inertia_diag: [100, 100, 50],
          initial_angular_velocity: [0, 0, 0],
        },
      },
    ],
  };
  configPath = path.join(os.tmpdir(), `orts-direction-vectors-e2e-${Date.now()}.json`);
  fs.writeFileSync(configPath, JSON.stringify(config));

  const { child, port } = await spawnServer(configPath);
  ortsProcess = child;
  wsUrl = `ws://localhost:${port}/ws`;
  console.log(`orts direction-vector server started at ${wsUrl}`);
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

/** Connect the viewer to the test server through the normal UI path. */
async function connect(page: import("@playwright/test").Page) {
  await page.goto("/?noAutoConnect=1");
  await page.locator('[data-testid="ws-url-input"]').fill(wsUrl);
  await page.locator('[data-testid="ws-connect-btn"]').click();
  await expect(page.locator('[data-testid="ws-status-text"]')).toContainText("Connected", {
    timeout: 15000,
  });
  await expect(page.locator("canvas").first()).toBeVisible();
  await page.waitForFunction(
    () =>
      typeof (window as unknown as Record<string, unknown>).__debug_get_sat_world_quat ===
      "function",
    { timeout: 15000 },
  );
}

/** Read the drawn arrows, retrying while the scene graph settles. */
async function drawnVectors(
  page: import("@playwright/test").Page,
  id: string,
  { expectSome }: { expectSome: boolean },
): Promise<DrawnVector[] | null> {
  return page.evaluate(
    async ([entityId, wantSome]) => {
      const w = window as unknown as Record<string, unknown>;
      // Re-read the hook each pass: it is installed when the arrows first mount,
      // which can be after the connection is up. Reading it once and giving up
      // would make this pass or fail on scene-mount timing.
      const read = () =>
        (
          w.__debug_get_direction_vectors as
            | ((
                id: string,
              ) => { kind: string; direction: [number, number, number]; distance: number }[] | null)
            | undefined
        )?.(entityId as string) ?? null;
      for (let i = 0; i < 40; i++) {
        const drawn = read();
        if (!wantSome) {
          // Absence has to settle too: give the scene the same window to draw
          // something before concluding it drew nothing.
          if (i === 39) return drawn ?? [];
        } else if (drawn != null && drawn.length > 0) {
          return drawn;
        }
        await new Promise((r) => setTimeout(r, 100));
      }
      return read() ?? [];
    },
    [id, expectSome] as [string, boolean],
  );
}

test("centred satellite gets Sun and nadir arrows, in the local-orbital basis", async ({
  page,
}) => {
  await connect(page);

  // Centre on the satellite; the selector defaults a satellite centre to LVLH.
  await page
    .locator('[data-testid="frame-selector-select"]')
    .selectOption(`satellite:${SAT_ENTITY_PATH}`);
  await expect(page.locator('[data-testid="frame-orientation-lvlh"]')).toBeVisible();

  const vectors = await drawnVectors(page, SAT_ENTITY_PATH, { expectSome: true });
  expect(vectors, "the debug hook should expose the drawn arrows").not.toBeNull();
  if (vectors == null) return;

  expect(
    vectors.map((v) => v.kind).sort(),
    "both reference directions should be drawn once the epoch and position are known",
  ).toEqual(["nadir", "sun"]);

  const nadir = vectors.find((v) => v.kind === "nadir")?.direction;
  expect(nadir).toBeDefined();
  if (nadir == null) return;

  // [in-track, cross-track, radial] = scene [X, Y, Z], so the central body is
  // exactly along scene -Z whatever the orbital phase.
  expect(
    nadir[2],
    `nadir should point along scene -Z in the local-orbital view, got ` +
      `(${nadir.map((c) => c.toFixed(3)).join(", ")}). A non-Z direction means the arrow ` +
      `was built from a different axis order than the positions use.`,
  ).toBeLessThan(-0.99);
  expect(Math.abs(nadir[0]), "nadir should have no in-track component").toBeLessThan(0.02);
  expect(Math.abs(nadir[1]), "nadir should have no cross-track component").toBeLessThan(0.02);

  // The head's distance from the origin pins the rendered proportions, which the
  // normalised direction cannot: this satellite has no 3D model, so it is drawn
  // as the orientation cube (half-extent 0.008 → apparent size 0.016), and the
  // head's centre sits at `startOffset + length - headLength / 2` = 1.9 spans.
  const EXPECTED_HEAD_DISTANCE = 1.9 * 0.016;
  for (const v of vectors) {
    expect(
      v.distance,
      `${v.kind} arrow's head should sit 1.9 spans out; a different distance means the ` +
        `offset or the arrow proportions changed`,
    ).toBeCloseTo(EXPECTED_HEAD_DISTANCE, 6);
  }
});

test("a central-body view draws no arrows", async ({ page }) => {
  // The body is on screen there, so a nadir arrow repeats the picture; and with
  // many satellites a pair of arrows on each would fill the screen.
  await connect(page);

  // Draw them first. Asserting the absence straight away would also pass if the
  // arrows had never been reachable at all — the hook is only installed once they
  // mount, and it is never installed in a central-body view.
  await page
    .locator('[data-testid="frame-selector-select"]')
    .selectOption(`satellite:${SAT_ENTITY_PATH}`);
  const drawn = await drawnVectors(page, SAT_ENTITY_PATH, { expectSome: true });
  expect(drawn?.length ?? 0, "arrows should be drawn while a satellite is centred").toBeGreaterThan(
    0,
  );

  await page.locator('[data-testid="frame-selector-select"]').selectOption("central_body");
  const vectors = await drawnVectors(page, SAT_ENTITY_PATH, { expectSome: false });
  expect(vectors ?? [], "no arrows should be registered for a central-body view").toHaveLength(0);
});
