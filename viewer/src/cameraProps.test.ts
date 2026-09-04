import { PerspectiveCamera, Vector3 } from "three";
import { describe, expect, it } from "vitest";
import {
  type CameraPropsInput,
  DEFAULT_FAR,
  DEFAULT_NEAR,
  usableDirection,
  usablePosition,
  usableProjection,
} from "./cameraProps.js";
import { drawnExtentForSpan, NOMINAL_SPACECRAFT_SPAN } from "./spacecraftScale.js";

/** What the attitude view judges a near plane against. */
const EXTENT = drawnExtentForSpan(NOMINAL_SPACECRAFT_SPAN);
const FOV = 50;
const FALLBACK: [number, number, number] = [3, 0, 1.5];

/**
 * Whether the renderer can use a projection built from these settings.
 *
 * Read as a `Float32Array`, the precision WebGL receives the matrix in: a
 * coefficient can be a perfectly good double and an infinite uniform, or a fine
 * double that rounds to zero. So the entries that have to carry a volume — the
 * two scales and the depth mapping — are required to be there, not merely finite.
 */
function projectionIsUsable({
  fov,
  zoom,
  near,
  far,
}: ReturnType<typeof usableProjection>): boolean {
  const camera = new PerspectiveCamera(fov, 1, near, far);
  camera.zoom = zoom;
  camera.updateProjectionMatrix();
  const e = Array.from(new Float32Array(camera.projectionMatrix.elements));
  if (!e.every(Number.isFinite)) return false;
  // x scale, y scale, depth scale, depth offset.
  return [e[0], e[5], e[10], e[14]].every((v) => v !== 0);
}

/**
 * Whether a camera at this position yields a view matrix the renderer can use.
 *
 * The view matrix is the inverse of the camera's world matrix, and WebGL receives
 * it as float32 — the same reading as the projection oracle, one matrix over. Its
 * translation has to survive that reading as something other than zero, or the
 * GPU has the camera at the origin whatever was asked for.
 */
function viewMatrixIsUsable(position: readonly number[]): boolean {
  const camera = new PerspectiveCamera(FOV, 1, DEFAULT_NEAR, DEFAULT_FAR);
  camera.position.set(position[0], position[1], position[2]);
  camera.lookAt(new Vector3(0, 0, 0));
  camera.updateMatrixWorld(true);
  const e = Array.from(new Float32Array(camera.matrixWorldInverse.elements));
  return e.every(Number.isFinite) && [e[12], e[13], e[14]].some((v) => v !== 0);
}

describe("usableDirection", () => {
  it("keeps a direction Three.js can normalise", () => {
    // `up` only names a direction, so its magnitude is free: `lookAt` normalises
    // it, and these all come back as unit vectors.
    for (const v of [
      [1, 2, 3],
      [0, 0, -1],
      [1e100, 0, 0],
      [1e-100, 0, 0],
    ] as number[][]) {
      const out = usableDirection(v, FALLBACK);
      expect(out, `${v} names a direction`).not.toBe(FALLBACK);
      // The oracle: Three.js has to get a unit vector out of whatever we accept.
      expect(new Vector3(...out).normalize().length()).toBeCloseTo(1, 6);
    }
  });

  it("replaces a vector Three.js cannot normalise, however finite its components", () => {
    // Three.js sums the squared components, so a length can overflow from finite
    // parts (`1e200` squared is infinite, and the normalised vector comes back as
    // zeros) or underflow to nothing (`1e-200` squared is zero, and the vector
    // survives normalisation unchanged at a length no rotation can use).
    for (const v of [
      undefined,
      [1, 2],
      [1, 2, 3, 4],
      [0, 0, 0],
      [Number.NaN, 1, 1],
      [Number.POSITIVE_INFINITY, 0, 0],
      [1e200, 1e200, 0],
      [1e-200, 0, 0],
    ] as (number[] | undefined)[]) {
      expect(usableDirection(v, FALLBACK), `${v} is no direction`).toBe(FALLBACK);
    }
  });

  it("keeps a signed zero component, which is a direction like any other", () => {
    expect(usableDirection([1, -0, 0], FALLBACK)).toEqual([1, -0, 0]);
  });
});

describe("usablePosition", () => {
  it("keeps a place the renderer can put a camera", () => {
    for (const v of [
      [3, 0, 1.5],
      [200, 0, 0],
      [1e30, 0, 0],
      [3e38, 0, 0],
      [1e-30, 0, 0],
    ] as number[][]) {
      const out = usablePosition(v, FALLBACK);
      expect(out, `${v} is a position`).not.toBe(FALLBACK);
      expect(viewMatrixIsUsable(out), `${v} yields a usable view matrix`).toBe(true);
    }
  });

  it("replaces a position that cannot survive the float32 the GPU receives", () => {
    // The view matrix reaches WebGL as float32, where anything past 3.4e38 is
    // `Infinity` and blanks the scene. What it carries is the distance, not the
    // components: `[3e38, 3e38, 0]` is inside float32 on every axis and 4.24e38
    // long. `[1e100, 0, 0]` is a fine double and no place to put a camera — and
    // still a usable *direction*, which is why the two are checked apart.
    for (const v of [
      [1e100, 0, 0],
      [3e38, 3e38, 0],
      [2e38, 2e38, 2e38],
      [1e-100, 0, 0],
      [0, 0, 0],
      [Number.NaN, 1, 1],
    ] as number[][]) {
      expect(usablePosition(v, FALLBACK), `${v} is no camera position`).toBe(FALLBACK);
    }
    expect(usableDirection([1e100, 0, 0], FALLBACK)).toEqual([1e100, 0, 0]);
    expect(usableDirection([1e-100, 0, 0], FALLBACK)).toEqual([1e-100, 0, 0]);
    // The oracle agrees about the two a per-component check would pass: one
    // overflows the uploaded distance, the other rounds it away to nothing.
    expect(viewMatrixIsUsable([3e38, 3e38, 0])).toBe(false);
    expect(viewMatrixIsUsable([1e-100, 0, 0])).toBe(false);
  });
});

describe("usableProjection", () => {
  it("passes through settings that describe a frustum", () => {
    const asked: CameraPropsInput = { zoom: 2, near: 0.5, far: 50 };
    expect(usableProjection(asked, FOV, EXTENT)).toEqual({
      fov: FOV,
      zoom: 2,
      near: 0.5,
      far: 50,
      // The caller named this plane, so it is not the viewer's to keep moving.
      farIsDefault: false,
    });
    expect(usableProjection({ zoom: 2 }, FOV, EXTENT).farIsDefault).toBe(true);
    // A far plane that cannot describe a frustum is replaced, and the replacement
    // is the default — the viewer's to maintain again.
    expect(usableProjection({ near: 1, far: 0.5 }, FOV, EXTENT).farIsDefault).toBe(true);
  });

  it("yields a projection matrix Three.js can build, whatever it is given", () => {
    // The contract in one assertion: every answer this function gives has to
    // survive `updateProjectionMatrix`, including for inputs it had to replace.
    const values = [
      undefined,
      0,
      -1,
      Number.NaN,
      Number.POSITIVE_INFINITY,
      Number.MIN_VALUE,
      1e-300,
      1,
      1e17,
      1e200,
      Number.MAX_VALUE,
    ];
    // Every field of view `usableFovDegrees` can hand over, against every
    // combination of the three depths.
    for (const fov of [0.1, FOV, 179.9]) {
      for (const zoom of values) {
        for (const near of values) {
          for (const far of values) {
            const projection = usableProjection({ zoom, near, far }, fov, EXTENT);
            expect(
              projectionIsUsable(projection),
              `fov=${fov} zoom=${zoom} near=${near} far=${far} gave ${JSON.stringify(projection)}`,
            ).toBe(true);
          }
        }
      }
    }
  });

  it("replaces a zoom whose projection coefficient overflows", () => {
    // `zoom: MAX_VALUE` leaves the half height at the near plane positive, finite
    // and subnormal — and `near / halfHeight`, the coefficient Three.js scales y
    // by, infinite. Checking the half height alone accepts it.
    //
    // `zoom: 1e200` is the same failure one precision down: the coefficient comes
    // out at 2.1e200, a fine double that is `Infinity` in the float32 uniform.
    for (const zoom of [Number.MAX_VALUE, 1e200]) {
      const projection = usableProjection({ zoom }, FOV, EXTENT);
      expect(projection.zoom, `zoom: ${zoom} cannot be built`).toBe(1);
      expect(projection.near).toBe(DEFAULT_NEAR);
    }
    // And the other end: 1e-300 leaves it at 2.1e-300, which rounds to zero and
    // flattens the frustum. 1e30 and 1e-30 both stay inside float32.
    expect(usableProjection({ zoom: 1e-300 }, FOV, EXTENT).zoom).toBe(1);
    expect(usableProjection({ zoom: 1e30 }, FOV, EXTENT).zoom).toBe(1e30);
    expect(usableProjection({ zoom: 1e-30 }, FOV, EXTENT).zoom).toBe(1e-30);
  });

  it("replaces a near plane the scene's own size vanishes against", () => {
    // From 1e17 spans, adding the drawn extent to the near plane changes nothing,
    // so no camera distance puts the scene between the planes.
    expect(usableProjection({ near: 1e17 }, FOV, EXTENT).near).toBe(DEFAULT_NEAR);
    expect(usableProjection({ near: 1e16 }, FOV, EXTENT).near).toBe(1e16);
  });

  it("opens a depth range when the caller's far plane is inside the near one", () => {
    // Two planes in the wrong order enclose no volume. A near plane past the
    // default far plane still has to end up with something in front of it.
    expect(usableProjection({ near: 1, far: 0.5 }, FOV, EXTENT).far).toBe(DEFAULT_FAR);
    const beyond = usableProjection({ near: 1000, far: 1 }, FOV, EXTENT);
    expect(beyond.far).toBeGreaterThan(beyond.near);
  });
});
