import * as THREE from "three";
import { describe, expect, it } from "vitest";
import {
  DIRECTION_VECTOR_COLORS,
  type DirectionVector,
  type DirectionVectorKind,
  resolveDirectionVectors,
} from "./directionVectors.js";
import { type DisplayFrame, displayDirection, type Vec3 } from "./displayFrame.js";
import { computeLvlhAxes } from "./sceneFrame.js";

const MU = 398600.4418;

/** Circular orbit state [km, km/s] at true anomaly `phase` with inclination `inc`. */
function orbitState(radius: number, phase: number, inc: number): { r: Vec3; v: Vec3 } {
  const speed = Math.sqrt(MU / radius);
  const inPlane = new THREE.Vector3(Math.cos(phase), Math.sin(phase), 0);
  const inPlaneVel = new THREE.Vector3(-Math.sin(phase), Math.cos(phase), 0);
  const tilt = new THREE.Matrix4().makeRotationX(inc);
  const r = inPlane.clone().multiplyScalar(radius).applyMatrix4(tilt);
  const v = inPlaneVel.clone().multiplyScalar(speed).applyMatrix4(tilt);
  return { r: [r.x, r.y, r.z], v: [v.x, v.y, v.z] };
}

function localOrbitalFrame(r: Vec3, v: Vec3): DisplayFrame {
  const axes = computeLvlhAxes(r, v);
  if (axes == null) throw new Error("degenerate orbit");
  return { kind: "localOrbital", origin: r, axes };
}

function find(vectors: DirectionVector[], kind: DirectionVectorKind): DirectionVector {
  const found = vectors.find((v) => v.kind === kind);
  if (found == null) throw new Error(`no ${kind} vector`);
  return found;
}

const dot = (a: Vec3, b: Vec3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const norm = (v: Vec3): number => Math.sqrt(dot(v, v));

const INERTIAL: DisplayFrame = { kind: "inertial", origin: null };
const SUN_ECI: Vec3 = [0.6, 0, 0.8];

describe("resolveDirectionVectors", () => {
  it("points nadir at scene -Z in the local-orbital frame, at any phase and inclination", () => {
    // The viewer's local-orbital basis is [in-track, cross-track, radial], so the
    // central body is exactly along scene -Z there — the invariant that catches a
    // nadir arrow built from a different axis order than the positions use.
    for (const phase of [0, 0.7, 2.1, 4.9]) {
      for (const inc of [0, 0.6, Math.PI / 2]) {
        const { r, v } = orbitState(7000, phase, inc);
        const nadir = find(
          resolveDirectionVectors({ frame: localOrbitalFrame(r, v), positionEci: r }),
          "nadir",
        ).direction;
        expect(nadir[0]).toBeCloseTo(0, 9);
        expect(nadir[1]).toBeCloseTo(0, 9);
        expect(nadir[2]).toBeCloseTo(-1, 9);
      }
    }
  });

  it("points nadir back along the position in the inertial frame", () => {
    const { r } = orbitState(7000, 1.1, 0.4);
    const nadir = find(
      resolveDirectionVectors({ frame: INERTIAL, positionEci: r }),
      "nadir",
    ).direction;
    const len = norm(r);
    expect(nadir[0]).toBeCloseTo(-r[0] / len, 12);
    expect(nadir[1]).toBeCloseTo(-r[1] / len, 12);
    expect(nadir[2]).toBeCloseTo(-r[2] / len, 12);
  });

  it("leaves the Sun as the lighting aimed it, in every frame", () => {
    // The lighting has already turned this vector into the display frame, so
    // turning it again here would rotate it twice: in the body-fixed frame below
    // a second -ERA would take +X to -Y instead of leaving it where it is.
    const { r, v } = orbitState(7000, 1.4, 0.6);
    const aimed: Vec3 = [1, 0, 0];
    for (const frame of [
      INERTIAL,
      { kind: "bodyFixed", era: Math.PI / 2 } as DisplayFrame,
      localOrbitalFrame(r, v),
    ]) {
      const sun = find(resolveDirectionVectors({ frame, sunDisplay: aimed }), "sun").direction;
      expect(sun).toEqual(aimed);
    }
  });

  it("returns unit vectors in every frame, from unnormalised inputs", () => {
    const { r, v } = orbitState(7000, 2.2, 0.9);
    const frames: DisplayFrame[] = [
      INERTIAL,
      { kind: "bodyFixed", era: 1.3 },
      localOrbitalFrame(r, v),
    ];
    for (const frame of frames) {
      // A position in km and a Sun direction scaled by 5: neither is a unit vector.
      const vectors = resolveDirectionVectors({
        frame,
        sunDisplay: [SUN_ECI[0] * 5, SUN_ECI[1] * 5, SUN_ECI[2] * 5],
        positionEci: r,
      });
      expect(vectors).toHaveLength(2);
      for (const vec of vectors) {
        expect(norm(vec.direction)).toBeCloseTo(1, 12);
      }
    }
  });

  it("preserves the angle between the Sun and nadir through the frame transform", () => {
    // The relative geometry — the thing an operator reads off the picture —
    // cannot depend on the chosen frame. The Sun reaches the resolver already
    // transformed, so the invariant is checked over the composition the scene
    // builds: the lighting's transform, then this resolver's.
    const { r, v } = orbitState(7200, 3.3, 0.8);
    const asLit = (frame: DisplayFrame) =>
      resolveDirectionVectors({
        frame,
        sunDisplay: displayDirection(frame, SUN_ECI),
        positionEci: r,
      });
    const inertial = asLit(INERTIAL);
    const expected = dot(find(inertial, "sun").direction, find(inertial, "nadir").direction);
    for (const frame of [
      { kind: "bodyFixed", era: 2.1 } as DisplayFrame,
      localOrbitalFrame(r, v),
    ]) {
      const vectors = asLit(frame);
      expect(dot(find(vectors, "sun").direction, find(vectors, "nadir").direction)).toBeCloseTo(
        expected,
        12,
      );
    }
  });

  it("omits the Sun when no Sun direction is known", () => {
    // No epoch means no Sun direction. A fixed arrow would read as a measurement.
    const { r } = orbitState(7000, 0.5, 0.2);
    const vectors = resolveDirectionVectors({ frame: INERTIAL, positionEci: r });
    expect(vectors.map((v) => v.kind)).toEqual(["nadir"]);
  });

  it("omits nadir when the position is absent, zero or non-finite", () => {
    for (const positionEci of [
      null,
      [0, 0, 0],
      [Number.NaN, 0, 0],
      [Number.POSITIVE_INFINITY, 1, 2],
    ] as (Vec3 | null)[]) {
      const vectors = resolveDirectionVectors({
        frame: INERTIAL,
        sunDisplay: SUN_ECI,
        positionEci,
      });
      expect(vectors.map((v) => v.kind)).toEqual(["sun"]);
      for (const vec of vectors) {
        expect(vec.direction.every(Number.isFinite)).toBe(true);
      }
    }
  });

  it("omits a direction whose length overflows from finite components", () => {
    // Each component is finite but the squares are not, so the length comes out
    // infinite and dividing by it would give a zero vector — which a rotation
    // onto it turns into an invalid quaternion rather than a dropped arrow.
    const huge: Vec3 = [1e200, 1e200, 1e200];
    expect(
      resolveDirectionVectors({ frame: INERTIAL, sunDisplay: huge, positionEci: huge }),
    ).toEqual([]);
  });

  it("omits the Sun when its direction is zero or non-finite", () => {
    const { r } = orbitState(7000, 0.5, 0.2);
    for (const sunDisplay of [
      [0, 0, 0],
      [0, Number.NaN, 0],
    ] as Vec3[]) {
      const vectors = resolveDirectionVectors({ frame: INERTIAL, sunDisplay, positionEci: r });
      expect(vectors.map((v) => v.kind)).toEqual(["nadir"]);
    }
  });

  it("honours the per-direction options", () => {
    const { r } = orbitState(7000, 0.5, 0.2);
    const inputs = { frame: INERTIAL, sunDisplay: SUN_ECI, positionEci: r };
    expect(
      resolveDirectionVectors({ ...inputs, options: { sun: false } }).map((v) => v.kind),
    ).toEqual(["nadir"]);
    expect(
      resolveDirectionVectors({ ...inputs, options: { nadir: false } }).map((v) => v.kind),
    ).toEqual(["sun"]);
    expect(
      resolveDirectionVectors({ ...inputs, options: { sun: false, nadir: false } }),
    ).toHaveLength(0);
    // An empty options object leaves both on: absent is not "off".
    expect(resolveDirectionVectors({ ...inputs, options: {} }).map((v) => v.kind)).toEqual([
      "sun",
      "nadir",
    ]);
  });

  it("tags each direction with the colour the legend uses", () => {
    const { r } = orbitState(7000, 0.5, 0.2);
    const vectors = resolveDirectionVectors({
      frame: INERTIAL,
      sunDisplay: SUN_ECI,
      positionEci: r,
    });
    expect(find(vectors, "sun").color).toBe(DIRECTION_VECTOR_COLORS.sun);
    expect(find(vectors, "nadir").color).toBe(DIRECTION_VECTOR_COLORS.nadir);
  });
});
