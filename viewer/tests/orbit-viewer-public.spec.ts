/**
 * The published `OrbitViewer`, in states a live run cannot produce.
 *
 * `orbit-direction-vectors.spec.ts` drives the app against a real `orts serve`,
 * where every satellite shares one clock and every sample is a number. The public
 * props allow neither: `SatelliteState.time` is per satellite, and it can arrive
 * as `NaN` from a source whose parse failed. Both decide the epoch the Sun and
 * the body rotations are evaluated at, so both are driven from a fixture here.
 *
 * Assertions read the rendered scene through the dev hooks rather than pixels.
 * See .claude/skills/playwright-viewer-testing.
 */
import path from "node:path";
import { fileURLToPath } from "node:url";
import { expect, type Page, test } from "@playwright/test";

/** Epoch used wherever the Sun has to be computable (UTC JD). */
const EPOCH = 2460000.5;

/** Equatorial circular orbit at +X, and a second one a quarter turn along. */
const SAT_A = "7000,0,0:0,7.546,0";
const SAT_B = "0,7000,0:-7.546,0,0";

/**
 * Ninety days in seconds. The Sun moves about a degree a day, so two scenes
 * evaluated this far apart cannot be confused for each other — a difference of
 * minutes would be within the noise of an assertion.
 */
const LATER = 90 * 86400;

type Drawn = { kind: string; direction: [number, number, number]; distance: number };

async function open(page: Page, query: string) {
  await page.goto(`/fixtures/orbit-viewer.html?${query}`, { waitUntil: "load" });
  await expect(page.locator("canvas")).toBeVisible();
  // Only where there is an epoch: the scene loads the arika WASM to evaluate one,
  // and a scene without an epoch never asks for it — so waiting for readiness
  // there waits for something that is not coming.
  if (query.includes("epoch=")) {
    await expect
      .poll(() => page.evaluate(() => window.__fixture_arika_ready?.() ?? false), {
        timeout: 15000,
      })
      .toBe(true);
  }
}

/** The arrows drawn at satellite `index`, once the named kinds are all there. */
async function arrowsAt(page: Page, index: number, kinds: string[]): Promise<Drawn[]> {
  const id = `fixture-sat-${index}`;
  await expect
    .poll(
      async () => {
        const drawn = await page.evaluate(
          (satId) =>
            (
              window as unknown as {
                __debug_get_direction_vectors?: (id: string) => Drawn[] | null;
              }
            ).__debug_get_direction_vectors?.(satId) ?? null,
          id,
        );
        return drawn == null ? null : drawn.map((d) => d.kind).sort();
      },
      { timeout: 15000 },
    )
    .toEqual([...kinds].sort());
  return await page.evaluate(
    (satId) =>
      (
        window as unknown as {
          __debug_get_direction_vectors?: (id: string) => Drawn[] | null;
        }
      ).__debug_get_direction_vectors?.(satId) ?? [],
    id,
  );
}

/**
 * Mesh geometries in the scene, from the live graph.
 *
 * Entered through the arrows' own registry rather than the attitude one: a
 * spacecraft whose attitude was refused registers no attitude hook, which is the
 * case this is here to read. Walking `.parent` from either reaches the scene
 * root — `canvas.__r3f` is null. See .claude/skills/playwright-viewer-testing.
 */
async function meshGeometries(page: Page, id: string): Promise<string[] | null> {
  return await page.evaluate((satId) => {
    const w = window as unknown as {
      __debug_direction_vector_registry?: Map<
        string,
        () => { origin: { parent: unknown } | null }[]
      >;
      __debug_sat_quat_registry?: Map<string, () => { parent: unknown } | null>;
    };
    const start =
      w.__debug_direction_vector_registry?.get(satId)?.()?.[0]?.origin ??
      w.__debug_sat_quat_registry?.get(satId)?.();
    if (start == null) return null;
    type Node = {
      type: string;
      parent: Node | null;
      children: Node[];
      geometry?: { type?: string };
    };
    let root = start as unknown as Node;
    while (root.parent != null) root = root.parent;
    const out: string[] = [];
    const walk = (node: Node) => {
      if (node.type === "Mesh") out.push(node.geometry?.type ?? "?");
      for (const child of node.children) walk(child);
    };
    walk(root);
    return out;
  }, id);
}

/**
 * How far the central body sits from the centred spacecraft, in scene units.
 *
 * Centring converts positions with a radius the amplification divides, so this
 * distance is where the chosen amplification shows up in the rendered graph. Read
 * off the largest sphere, which is the body itself — the markers are far smaller.
 */
async function centralBodyDistance(page: Page, id: string): Promise<number | null> {
  return await page.evaluate((satId) => {
    const w = window as unknown as {
      __debug_direction_vector_registry?: Map<
        string,
        () => { origin: { parent: unknown } | null }[]
      >;
    };
    const start = w.__debug_direction_vector_registry?.get(satId)?.()?.[0]?.origin;
    if (start == null) return null;
    type Node = {
      type: string;
      parent: Node | null;
      children: Node[];
      matrixWorld: { elements: number[] };
      geometry?: { type?: string; parameters?: { radius?: number } };
    };
    let root = start as unknown as Node;
    while (root.parent != null) root = root.parent;
    let largest: { radius: number; node: Node } | null = null;
    const walk = (node: Node) => {
      if (node.geometry?.type === "SphereGeometry") {
        const r = node.geometry.parameters?.radius;
        if (r != null && (largest == null || r > largest.radius)) largest = { radius: r, node };
      }
      for (const child of node.children) walk(child);
    };
    walk(root);
    if (largest == null) return null;
    const e = (largest as { node: Node }).node.matrixWorld.elements;
    return Math.hypot(e[12], e[13], e[14]);
  }, id);
}

/**
 * Rotation of the group holding the cube marker, read from the live graph.
 *
 * Not through the attitude hook: that is registered by the body axes, which are
 * gone the moment the attitude is, and this is about what remains on screen
 * afterwards.
 */
async function cubeGroupRotation(page: Page, id: string): Promise<number[] | null> {
  return await page.evaluate((satId) => {
    const w = window as unknown as {
      __debug_direction_vector_registry?: Map<
        string,
        () => { origin: { parent: unknown } | null }[]
      >;
    };
    const start = w.__debug_direction_vector_registry?.get(satId)?.()?.[0]?.origin;
    if (start == null) return null;
    type Node = {
      type: string;
      parent: Node | null;
      children: Node[];
      quaternion: { x: number; y: number; z: number; w: number };
      geometry?: { type?: string };
    };
    let root = start as unknown as Node;
    while (root.parent != null) root = root.parent;
    let found: Node | null = null;
    const walk = (node: Node) => {
      if (node.geometry?.type === "BoxGeometry" && node.parent != null) found = node.parent;
      for (const child of node.children) walk(child);
    };
    walk(root);
    if (found == null) return null;
    const q = (found as Node).quaternion;
    return [q.w, q.x, q.y, q.z];
  }, id);
}

function sunDirection(arrows: Drawn[]): [number, number, number] {
  const sun = arrows.find((a) => a.kind === "sun");
  if (sun == null) throw new Error("no Sun arrow was drawn");
  return sun.direction;
}

test("the Sun is drawn at the centred satellite's own time", async ({ page }) => {
  // Two satellites at times ninety days apart, centred on the second. The Sun
  // has to be the Sun at *that* satellite's time: the scene used to take
  // whichever time the position Map yielded first, which is the other one.
  await open(page, `sats=0:${SAT_A};${LATER}:${SAT_B}&centre=1&epoch=${EPOCH}&arrows=sun`);
  const centred = sunDirection(await arrowsAt(page, 1, ["sun"]));

  // The same satellite alone, at the same time: the direction to compare with.
  await open(page, `sats=${LATER}:${SAT_B}&centre=0&epoch=${EPOCH}&arrows=sun`);
  const alone = sunDirection(await arrowsAt(page, 0, ["sun"]));

  for (const axis of [0, 1, 2]) {
    expect(
      centred[axis],
      `Sun component ${axis} should match the centred satellite's time`,
    ).toBeCloseTo(alone[axis], 6);
  }

  // And the other satellite's time really does give a different Sun, so the
  // comparison above could have failed.
  await open(page, `sats=0:${SAT_B}&centre=0&epoch=${EPOCH}&arrows=sun`);
  const atZero = sunDirection(await arrowsAt(page, 0, ["sun"]));
  const cosine = atZero[0] * alone[0] + atZero[1] * alone[1] + atZero[2] * alone[2];
  expect(cosine, "ninety days apart the Sun is nowhere near the same place").toBeLessThan(0.5);
});

test("a satellite whose time is not a number still leaves a drawable scene", async ({ page }) => {
  // `SatelliteState.time` reaching the scene as `NaN` is what a failed parse
  // upstream looks like. It decides the epoch of the Earth rotation angle and of
  // the body orientations, and a NaN there renders nothing at all — a group
  // carrying a NaN quaternion disappears.
  await open(page, `sats=nan:${SAT_A}&centre=0&frame=localOrbital&epoch=${EPOCH}&arrows=sun,nadir`);
  const arrows = await arrowsAt(page, 0, ["nadir", "sun"]);
  for (const arrow of arrows) {
    expect(
      arrow.direction.every(Number.isFinite),
      `${arrow.kind} arrow's direction should be finite`,
    ).toBe(true);
    expect(arrow.distance, `${arrow.kind} arrow should have a length`).toBeGreaterThan(0);
  }
});

/**
 * The rendered attitude of `id`, once a frame rotation has been applied to it.
 *
 * A body-fixed frame turns the scene by the Earth rotation angle, and that angle
 * comes from the WASM: for a render or two after the module is ready the frame is
 * still the inertial fallback, and the quaternion read then is the attitude as
 * supplied. Waiting for it to differ from `supplied` is what makes the two
 * documents comparable — reading straight after readiness catches whichever
 * render happened to be current.
 */
async function rotatedQuat(
  page: Page,
  id: string,
  supplied: [number, number, number, number],
): Promise<[number, number, number, number]> {
  await expect
    .poll(
      async () => {
        const q = await page.evaluate(
          (satId) => window.__debug_get_sat_world_quat?.(satId) ?? null,
          id,
        );
        if (q == null) return false;
        return [0, 1, 2, 3].some((i) => Math.abs(q[i] - supplied[i]) > 1e-6);
      },
      { timeout: 15000 },
    )
    .toBe(true);
  const q = await page.evaluate((satId) => window.__debug_get_sat_world_quat?.(satId) ?? null, id);
  if (q == null) throw new Error("no rendered attitude");
  return q;
}

test("a satellite with no usable time stays in the scene's frame", async ({ page }) => {
  // A body-fixed view rotates everything by the Earth rotation angle at the
  // scene's time. A satellite whose own `time` is `NaN` has to be rotated the
  // same way — dropping the rotation for it would leave that one marker in the
  // inertial frame while the body, the trails and every other satellite are
  // body-fixed, which is a picture of two conventions at once.
  // +90° about Z, as [w, x, y, z] for the fixture and as Three's [x, y, z, w] for
  // the comparison below.
  const attitude = "0.7071067811865476,0,0,0.7071067811865476";
  const supplied: [number, number, number, number] = [0, 0, Math.SQRT1_2, Math.SQRT1_2];

  await open(page, `sats=nan:${SAT_A}&frame=bodyFixed&epoch=${EPOCH}&att=${attitude}&arrows=none`);
  const withoutTime = await rotatedQuat(page, "fixture-sat-0", supplied);

  // The scene's own time is what it falls back to, and with this one satellite
  // carrying no usable time that is 0 — so a satellite explicitly at 0 has to
  // come out identical.
  await open(page, `sats=0:${SAT_A}&frame=bodyFixed&epoch=${EPOCH}&att=${attitude}&arrows=none`);
  const atZero = await rotatedQuat(page, "fixture-sat-0", supplied);
  for (const i of [0, 1, 2, 3]) {
    expect(withoutTime[i], `quaternion component ${i}`).toBeCloseTo(atZero[i], 6);
  }

  // The inertial frame leaves the attitude as supplied, which is what the wait
  // above rules out for the two body-fixed reads.
  await open(page, `sats=0:${SAT_A}&frame=inertial&epoch=${EPOCH}&att=${attitude}&arrows=none`);
  // `open()` waits for the canvas and for arika, not for the effect that registers
  // this hook, so the read has to wait for the hook itself — as the body-fixed
  // reads above do for their own reason.
  await expect
    .poll(
      () => page.evaluate((id) => window.__debug_get_sat_world_quat?.(id) != null, "fixture-sat-0"),
      { timeout: 15000 },
    )
    .toBe(true);
  const inertial = await page.evaluate(
    (id) => window.__debug_get_sat_world_quat?.(id) ?? null,
    "fixture-sat-0",
  );
  expect(inertial, "the inertial frame renders the attitude as given").not.toBeNull();
  for (const i of [0, 1, 2, 3]) {
    expect(inertial?.[i], `inertial component ${i}`).toBeCloseTo(supplied[i], 6);
  }
  const same = [0, 1, 2, 3].every((i) => Math.abs((inertial?.[i] ?? 0) - atZero[i]) < 1e-6);
  expect(same, "the body-fixed frame should not equal the inertial one").toBe(false);
});

test("a central-body view is drawn at the scene's time, not a satellite's", async ({ page }) => {
  // With no satellite centred there is nothing for the Sun to be drawn *at*, so
  // the epoch belongs to the scene: `OrbitSceneDataProps.time` drives the
  // lighting and the central body's rotation. The scene used to take whichever
  // time the position Map yielded first instead.
  //
  // The central body's rendered rotation is what reports the epoch the scene
  // chose — a satellite's own rotation would not, since each marker turns by its
  // own sample time.
  // The body's hook is installed before the WASM is ready, and until it is the
  // body is drawn unrotated. Reading then would compare the fallback against the
  // fallback, so each read waits for a rotation to have been applied — the same
  // reason `rotatedQuat` above waits.
  const identity: [number, number, number, number] = [0, 0, 0, 1];
  const earthQuat = async () => {
    await expect
      .poll(
        async () => {
          const q = await page.evaluate(() => window.__debug_get_earth_world_quat?.() ?? null);
          return q != null && [0, 1, 2, 3].some((i) => Math.abs(q[i] - identity[i]) > 1e-6);
        },
        { timeout: 15000 },
      )
      .toBe(true);
    const q = await page.evaluate(() => window.__debug_get_earth_world_quat?.() ?? null);
    if (q == null) throw new Error("no rendered central body");
    return q;
  };

  await open(page, `sats=0:${SAT_A}&epoch=${EPOCH}&t=0&arrows=none`);
  const sceneAtZero = await earthQuat();

  // The satellites are ninety days along while the scene is still at zero. The
  // central body has to be where the scene's time puts it.
  await open(page, `sats=${LATER}:${SAT_A};${LATER}:${SAT_B}&epoch=${EPOCH}&t=0&arrows=none`);
  const satellitesLater = await earthQuat();
  for (const i of [0, 1, 2, 3]) {
    expect(satellitesLater[i], `quaternion component ${i}`).toBeCloseTo(sceneAtZero[i], 6);
  }

  // And the scene's own time does move it, so the comparison could have failed.
  await open(page, `sats=0:${SAT_A}&epoch=${EPOCH}&t=${LATER}&arrows=none`);
  const sceneLater = await earthQuat();
  const same = [0, 1, 2, 3].every((i) => Math.abs(sceneLater[i] - sceneAtZero[i]) < 1e-6);
  expect(same, "ninety days of rotation should not leave the body where it was").toBe(false);
});

test("an embedder who says nothing about the arrows gets none", async ({ page }) => {
  // `directionVectors` is opt-in: the prop is omitted here, which is what every
  // embedder written before it existed passes. The centred satellite is the one
  // case that *would* draw them, so this is where the default has to hold.
  // The attitude is here to make the spacecraft register its own hook, which is
  // the only way to know the subtree mounted before reading an absence.
  await open(page, `sats=0:${SAT_A}&centre=0&frame=localOrbital&epoch=${EPOCH}&att=1,0,0,0`);

  // The scene has to be up before absence means anything: the spacecraft's own
  // hook says so, and it is registered by the same subtree that would carry the
  // arrows.
  await expect
    .poll(
      () => page.evaluate((id) => window.__debug_get_sat_world_quat?.(id) != null, "fixture-sat-0"),
      { timeout: 15000 },
    )
    .toBe(true);

  const drawn = await page.evaluate(
    (satId) =>
      (
        window as unknown as {
          __debug_get_direction_vectors?: (id: string) => unknown[] | null;
        }
      ).__debug_get_direction_vectors?.(satId) ?? null,
    "fixture-sat-0",
  );
  // Null rather than an empty list: with nothing to draw the component is not
  // mounted, so it registers no hook at all.
  expect(drawn, "no arrows are registered when the prop is left out").toBeNull();
});

test("the Sun arrow is left out where the direction would be a guess", async ({ page }) => {
  // `sun_direction_from_body` answers +X — the vernal equinox — for a body it
  // cannot place, and with no epoch there is nothing to evaluate at all. The
  // lighting keeps that fallback so a 3D model is not left black; the *arrow*
  // has to be dropped, or the picture shows a guess as a measurement.
  //
  // Nadir is the control: it needs only a position, so its presence says the
  // arrows reached this scene and the Sun's absence is a decision.
  await open(page, `sats=0:${SAT_A}&centre=0&frame=localOrbital&arrows=sun,nadir`);
  await arrowsAt(page, 0, ["nadir"]);

  // Uranus has no elements in arika, so the Sun cannot be placed relative to it
  // even with an epoch. Its radius is given because the scene has none to look up.
  await open(
    page,
    `sats=0:${SAT_A}&centre=0&frame=localOrbital&epoch=${EPOCH}&body=uranus&radius=25559&arrows=sun,nadir`,
  );
  await arrowsAt(page, 0, ["nadir"]);

  // And Earth with the same epoch does draw it, so the two above are not passing
  // for want of a Sun arrow anywhere.
  await open(page, `sats=0:${SAT_A}&centre=0&frame=localOrbital&epoch=${EPOCH}&arrows=sun,nadir`);
  await arrowsAt(page, 0, ["nadir", "sun"]);
});

test("no arrows are drawn at a centred spacecraft that has no position", async ({ page }) => {
  // A position that cannot be used — non-finite from a source — leaves the frame
  // without an origin for this spacecraft, and the marker is not drawn either.
  // Nadir needs that position and drops itself, but the Sun's direction does not,
  // so without the gate an arrow points out the Sun beside nothing at all.
  await open(page, `sats=0:nan,0,0:0,7.546,0&centre=0&epoch=${EPOCH}&att=1,0,0,0&arrows=sun,nadir`);

  // The scene is up: this satellite registers its attitude hook whether or not
  // its position can be used.
  await expect
    .poll(
      () => page.evaluate((id) => window.__debug_get_sat_world_quat?.(id) != null, "fixture-sat-0"),
      { timeout: 15000 },
    )
    .toBe(true);

  const drawn = await page.evaluate(
    (satId) =>
      (
        window as unknown as {
          __debug_get_direction_vectors?: (id: string) => unknown[] | null;
        }
      ).__debug_get_direction_vectors?.(satId) ?? null,
    "fixture-sat-0",
  );
  expect(drawn, "an unplaceable spacecraft gets no arrows").toBeNull();

  // The same scene with a usable position draws both, so the assertion above is
  // not passing for want of arrows anywhere.
  await open(page, `sats=0:${SAT_A}&centre=0&epoch=${EPOCH}&att=1,0,0,0&arrows=sun,nadir`);
  await arrowsAt(page, 0, ["nadir", "sun"]);
});

test("an attitude that names no rotation is refused, not applied", async ({ page }) => {
  // `SatelliteState.attitude` reaches Three.js through `Quaternion.set`, which
  // does not normalise: a zero quaternion names no rotation, and applying it
  // collapses the marker it was meant to orient. The attitude view already
  // refuses one; the orbit view handed it straight through.
  await open(page, `sats=0:${SAT_A}&centre=0&epoch=${EPOCH}&att=0,0,0,0&arrows=sun,nadir`);

  // The arrows are the proof of mount here: they depend on the position, not on
  // the attitude, so they are drawn either way.
  await arrowsAt(page, 0, ["nadir", "sun"]);

  const quat = await page.evaluate(
    (id) => window.__debug_get_sat_world_quat?.(id) ?? null,
    "fixture-sat-0",
  );
  expect(quat, "no orientation is registered for an attitude that names none").toBeNull();

  // A usable attitude is still applied, so the check above is not refusing
  // everything.
  await open(page, `sats=0:${SAT_A}&centre=0&epoch=${EPOCH}&att=1,0,0,0&arrows=sun,nadir`);
  await expect
    .poll(
      () => page.evaluate((id) => window.__debug_get_sat_world_quat?.(id) != null, "fixture-sat-0"),
      { timeout: 15000 },
    )
    .toBe(true);
});

test("an unusable attitude gets the marker that shows no orientation", async ({ page }) => {
  // The automatic marker is a cube when a satellite has attitude, because its
  // faces show which way the body points, and a sphere when it has none. That
  // choice has to follow the *usable* attitude: a zero quaternion drawn as a cube
  // shows an orientation nobody measured.
  // The cube is *asked for* here, which is the harder half: the resolver has to
  // refuse a request, not merely decline to choose the cube on its own.
  await open(
    page,
    `sats=0:${SAT_A}&centre=0&epoch=${EPOCH}&att=0,0,0,0&shape=axes-cube&arrows=sun,nadir`,
  );
  await arrowsAt(page, 0, ["nadir", "sun"]);
  const refused = await meshGeometries(page, "fixture-sat-0");
  expect(refused, "the scene graph should be reachable").not.toBeNull();
  // The cube is the discriminator. A sphere would be the other half of the
  // statement, but the central body draws spheres too and this inventory cannot
  // tell them apart from a marker's — so the absence of the cube is what is
  // asserted, and the presence of one below.
  expect(refused, "an unusable attitude draws no orientation-revealing cube").not.toContain(
    "BoxGeometry",
  );

  // A usable one does get the cube, so the check above is not describing every
  // scene.
  await open(page, `sats=0:${SAT_A}&centre=0&epoch=${EPOCH}&att=1,0,0,0&arrows=sun,nadir`);
  await arrowsAt(page, 0, ["nadir", "sun"]);
  const usable = await meshGeometries(page, "fixture-sat-0");
  expect(usable, "a usable attitude draws the cube").toContain("BoxGeometry");
});

test("the environment is scaled for the spacecraft that is drawn", async ({ page }) => {
  // Centring on a satellite amplifies everything around it, because its model is
  // drawn far larger than scale — the ratio keeps Earth and the trails at the
  // right proportions *relative to that model*. A registered spacecraft whose
  // attitude was refused is drawn as the marker instead, and the marker's ratio
  // is a different number, so the amplification has to follow which of the two
  // arrived.
  // Local-orbital, because that is the frame in which the amplification reaches
  // the graph: the scene applies it as the environment group's scale, and leaves
  // that at 1 in the inertial frame (measured: the body sits at 1.0975 either way
  // there, and at 2157.6 or 3500 here).
  const centred = `centre=0&frame=localOrbital&epoch=${EPOCH}&arrows=sun,nadir`;

  await open(page, `sats=0:${SAT_A}&${centred}&name=ISS&att=1,0,0,0`);
  await arrowsAt(page, 0, ["nadir", "sun"]);
  const model = await centralBodyDistance(page, "fixture-sat-0");
  expect(model, "the central body should be in the scene").not.toBeNull();
  expect(model, "and drawn away from the centred spacecraft").toBeGreaterThan(0);

  // No registered name, so no model config resolves and the marker stands in.
  // This is the amplification a refused attitude has to land on.
  await open(page, `sats=0:${SAT_A}&${centred}&att=1,0,0,0`);
  await arrowsAt(page, 0, ["nadir", "sun"]);
  const marker = await centralBodyDistance(page, "fixture-sat-0");

  // The two ratios differ, without which the assertion below would hold whatever
  // the scene did.
  expect((model as number) / (marker as number)).not.toBeCloseTo(1, 6);

  await open(page, `sats=0:${SAT_A}&${centred}&name=ISS&att=0,0,0,0`);
  await arrowsAt(page, 0, ["nadir", "sun"]);
  const refused = await centralBodyDistance(page, "fixture-sat-0");
  // A ratio, so compare relatively: the distances run to thousands of units.
  expect(
    (refused as number) / (marker as number),
    "a refused attitude scales the scene for the marker it draws",
  ).toBeCloseTo(1, 6);
});

test("a marker that keeps its cube gives up its rotation with the attitude", async ({ page }) => {
  // The reset that runs when an attitude stops being usable is only observable
  // where the same object stays on screen across the change. An explicit
  // `axes-cube` is that case: the request outranks the automatic choice, so the
  // cube is still drawn once the attitude is gone, and it must not go on showing
  // the orientation the last usable sample gave it.
  await open(
    page,
    `sats=0:${SAT_A}&centre=0&shape=axes-cube&att=${Math.SQRT1_2},0,0,${Math.SQRT1_2}&arrows=nadir`,
  );
  await arrowsAt(page, 0, ["nadir"]);

  const rotated = await cubeGroupRotation(page, "fixture-sat-0");
  expect(rotated, "the cube should be in the scene").not.toBeNull();
  // 90° about Z, as supplied.
  expect(rotated?.[0] ?? 0, "w").toBeCloseTo(Math.SQRT1_2, 6);
  expect(rotated?.[3] ?? 0, "z").toBeCloseTo(Math.SQRT1_2, 6);

  await page.evaluate(() => window.__fixture_set_attitude?.(null));

  await expect
    .poll(
      async () => {
        const q = await cubeGroupRotation(page, "fixture-sat-0");
        return q == null ? null : Math.abs(q[0] - 1) < 1e-6 && Math.abs(q[3]) < 1e-6;
      },
      { timeout: 15000 },
    )
    .toBe(true);

  // Still the cube: the shape follows the request, and it is the rotation alone
  // that the withdrawal took away.
  const geometries = await meshGeometries(page, "fixture-sat-0");
  expect(geometries, "an explicitly requested cube stays without an attitude").toContain(
    "BoxGeometry",
  );
});

/**
 * A glTF served in place of the registry's model.
 *
 * The registry points at a GLB on an external host, which does load under test —
 * but a test that depends on someone else's server fails for reasons that have
 * nothing to do with the viewer. This one triangle stands in: `GLTFLoader` reads
 * JSON as readily as the binary container, and what the test needs from a model
 * is that it be a loaded object with a group above it.
 */
const STAND_IN_MODEL = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "../fixtures/test-spacecraft.gltf",
);

/** Every ancestor rotation above the loaded model, innermost first. */
async function modelAncestorRotations(page: Page): Promise<number[][] | null> {
  return await page.evaluate(() => {
    const w = window as unknown as {
      __debug_direction_vector_registry?: Map<
        string,
        () => { origin: { parent: unknown } | null }[]
      >;
    };
    const start = w.__debug_direction_vector_registry?.get("fixture-sat-0")?.()?.[0]?.origin;
    if (start == null) return null;
    type Node = {
      name?: string;
      type: string;
      parent: Node | null;
      children: Node[];
      quaternion: { x: number; y: number; z: number; w: number };
    };
    let root = start as unknown as Node;
    while (root.parent != null) root = root.parent;
    let mesh: Node | null = null;
    const walk = (node: Node) => {
      if (node.name === "test-spacecraft") mesh = node;
      for (const child of node.children) walk(child);
    };
    walk(root);
    if (mesh == null) return null;
    const out: number[][] = [];
    for (let n: Node | null = mesh; n != null; n = n.parent) {
      out.push([n.quaternion.w, n.quaternion.x, n.quaternion.y, n.quaternion.z]);
    }
    return out;
  });
}

test("a model gives up its rotation when the attitude stops being usable", async ({ page }) => {
  // The registered-model path, which is where this matters most: the model is not
  // replaced when its attitude goes — a spacecraft with no attitude is drawn as a
  // position marker, which is the documented behaviour — so the same object stays
  // on screen and would go on showing the orientation the last usable sample gave
  // it. The marker case is covered above; this is the one whose ref used to be
  // detached before the reset could run.
  await page.route("**/*.glb", (route) =>
    route.fulfill({ path: STAND_IN_MODEL, contentType: "model/gltf+json" }),
  );

  const supplied = [Math.SQRT1_2, 0, 0, Math.SQRT1_2];
  await open(page, `sats=0:${SAT_A}&centre=0&name=ISS&att=${supplied.join(",")}&arrows=nadir`);
  await arrowsAt(page, 0, ["nadir"]);

  const carries = (rotations: number[][] | null) =>
    rotations?.some((q) => q.every((c, i) => Math.abs(c - supplied[i]) < 1e-6)) ?? false;

  // The model has to arrive before anything can be said about its rotation.
  await expect
    .poll(async () => carries(await modelAncestorRotations(page)), { timeout: 20000 })
    .toBe(true);

  await page.evaluate(() => window.__fixture_set_attitude?.(null));

  await expect
    .poll(
      async () => {
        const rotations = await modelAncestorRotations(page);
        // Still loaded, and no ancestor carries the withdrawn rotation any more.
        return rotations != null && !carries(rotations);
      },
      { timeout: 15000 },
    )
    .toBe(true);
});
