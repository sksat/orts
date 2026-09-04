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
 * coefficient can be a perfectly good double and an infinite uniform.
 */
function projectionIsFinite({
  fov,
  zoom,
  near,
  far,
}: ReturnType<typeof usableProjection>): boolean {
  const camera = new PerspectiveCamera(fov, 1, near, far);
  camera.zoom = zoom;
  camera.updateProjectionMatrix();
  const uploaded = new Float32Array(camera.projectionMatrix.elements);
  return Array.from(uploaded).every(Number.isFinite);
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
      [1e-100, 0, 0],
    ] as number[][]) {
      const out = usablePosition(v, FALLBACK);
      expect(out, `${v} is a position`).not.toBe(FALLBACK);
      // The oracle: the view matrix carries this as a float32 translation.
      expect(Array.from(new Float32Array(out)).every(Number.isFinite)).toBe(true);
    }
  });

  it("replaces a position that cannot survive the float32 the GPU receives", () => {
    // The view matrix reaches WebGL as float32, where anything past 3.4e38 is
    // `Infinity` and blanks the scene — `[1e100, 0, 0]` is a fine double and no
    // place to put a camera. It is still a usable *direction*, which is why the
    // two are checked apart.
    for (const v of [
      [1e100, 0, 0],
      [1e39, 1e39, 0],
      [0, 0, 0],
      [Number.NaN, 1, 1],
    ] as number[][]) {
      expect(usablePosition(v, FALLBACK), `${v} is no camera position`).toBe(FALLBACK);
    }
    expect(usableDirection([1e100, 0, 0], FALLBACK)).toEqual([1e100, 0, 0]);
  });
});

describe("usableProjection", () => {
  it("passes through settings that describe a frustum", () => {
    const asked: CameraPropsInput = { zoom: 2, near: 0.5, far: 50 };
    expect(usableProjection(asked, FOV, EXTENT)).toEqual({ fov: FOV, zoom: 2, near: 0.5, far: 50 });
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
              projectionIsFinite(projection),
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
    // And one that can: 1e30 leaves the coefficient at 2.1e30, inside float32.
    expect(usableProjection({ zoom: 1e30 }, FOV, EXTENT).zoom).toBe(1e30);
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
