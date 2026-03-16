import { describe, expect, it } from "vitest";
import { computeCameraUp, computeLvlhAxes, SCENE_UP } from "./sceneFrame.js";

type Vec3 = [number, number, number];
function dot(a: Vec3, b: Vec3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
}
function mag(v: Vec3): number {
  return Math.sqrt(dot(v, v));
}

describe("computeCameraUp", () => {
  it("returns SCENE_UP when originPosition is null", () => {
    expect(computeCameraUp(null)).toEqual(SCENE_UP);
  });

  it("returns SCENE_UP when originPosition is near-zero", () => {
    expect(computeCameraUp([0, 0, 0])).toEqual(SCENE_UP);
    expect(computeCameraUp([1e-15, 0, 0])).toEqual(SCENE_UP);
  });

  it("returns normalized radial direction for +X position", () => {
    const up = computeCameraUp([7000, 0, 0]);
    expect(up[0]).toBeCloseTo(1, 10);
    expect(up[1]).toBeCloseTo(0, 10);
    expect(up[2]).toBeCloseTo(0, 10);
  });

  it("returns normalized radial direction for +Z position", () => {
    const up = computeCameraUp([0, 0, 42164]);
    expect(up[0]).toBeCloseTo(0, 10);
    expect(up[1]).toBeCloseTo(0, 10);
    expect(up[2]).toBeCloseTo(1, 10);
  });

  it("returns normalized direction for arbitrary position", () => {
    const up = computeCameraUp([3000, 4000, 0]);
    // length = 5000, so normalized = [0.6, 0.8, 0]
    expect(up[0]).toBeCloseTo(0.6, 10);
    expect(up[1]).toBeCloseTo(0.8, 10);
    expect(up[2]).toBeCloseTo(0, 10);
  });

  it("returns unit vector for inclined orbit position", () => {
    const pos: [number, number, number] = [1000, 2000, 3000];
    const up = computeCameraUp(pos);
    const len = Math.sqrt(up[0] ** 2 + up[1] ** 2 + up[2] ** 2);
    expect(len).toBeCloseTo(1, 10);
    // Direction matches input
    const inputLen = Math.sqrt(1000 ** 2 + 2000 ** 2 + 3000 ** 2);
    expect(up[0]).toBeCloseTo(1000 / inputLen, 10);
    expect(up[1]).toBeCloseTo(2000 / inputLen, 10);
    expect(up[2]).toBeCloseTo(3000 / inputLen, 10);
  });
});

describe("computeLvlhAxes", () => {
  it("returns null when position is null", () => {
    expect(computeLvlhAxes(null, null)).toBeNull();
  });

  it("returns null when velocity is null", () => {
    expect(computeLvlhAxes([7000, 0, 0], null)).toBeNull();
  });

  it("returns null when position is near-zero", () => {
    expect(computeLvlhAxes([0, 0, 0], [0, 7.5, 0])).toBeNull();
  });

  it("returns null when velocity is near-zero", () => {
    expect(computeLvlhAxes([7000, 0, 0], [0, 0, 0])).toBeNull();
  });

  // Circular equatorial orbit: r = +X, v = +Y
  // radial = +X, crossTrack = +Z (r×v = X×Y = Z), inTrack = Z×X = +Y
  it("computes correct axes for equatorial circular orbit at +X", () => {
    const axes = computeLvlhAxes([7000, 0, 0], [0, 7.5, 0])!;
    expect(axes).not.toBeNull();

    // radial = +X
    expect(axes.radial[0]).toBeCloseTo(1, 10);
    expect(axes.radial[1]).toBeCloseTo(0, 10);
    expect(axes.radial[2]).toBeCloseTo(0, 10);

    // inTrack ≈ +Y (along velocity for circular orbit)
    expect(axes.inTrack[0]).toBeCloseTo(0, 10);
    expect(axes.inTrack[1]).toBeCloseTo(1, 10);
    expect(axes.inTrack[2]).toBeCloseTo(0, 10);

    // crossTrack = +Z (orbit normal for equatorial prograde)
    expect(axes.crossTrack[0]).toBeCloseTo(0, 10);
    expect(axes.crossTrack[1]).toBeCloseTo(0, 10);
    expect(axes.crossTrack[2]).toBeCloseTo(1, 10);
  });

  // r = +Y, v = -X (90° later in equatorial prograde orbit)
  it("computes correct axes at 90° in equatorial orbit", () => {
    const axes = computeLvlhAxes([0, 7000, 0], [-7.5, 0, 0])!;
    expect(axes).not.toBeNull();

    // radial = +Y
    expect(axes.radial[0]).toBeCloseTo(0, 10);
    expect(axes.radial[1]).toBeCloseTo(1, 10);
    expect(axes.radial[2]).toBeCloseTo(0, 10);

    // inTrack ≈ -X
    expect(axes.inTrack[0]).toBeCloseTo(-1, 10);
    expect(axes.inTrack[1]).toBeCloseTo(0, 10);
    expect(axes.inTrack[2]).toBeCloseTo(0, 10);

    // crossTrack = +Z
    expect(axes.crossTrack[0]).toBeCloseTo(0, 10);
    expect(axes.crossTrack[1]).toBeCloseTo(0, 10);
    expect(axes.crossTrack[2]).toBeCloseTo(1, 10);
  });

  // Polar orbit: r = +X, v = +Z
  // crossTrack = r×v = X×Z = -Y
  // inTrack = crossTrack × radial = (-Y)×X = +Z
  it("computes correct axes for polar orbit", () => {
    const axes = computeLvlhAxes([7000, 0, 0], [0, 0, 7.5])!;

    expect(axes.radial[0]).toBeCloseTo(1, 10);
    expect(axes.radial[1]).toBeCloseTo(0, 10);
    expect(axes.radial[2]).toBeCloseTo(0, 10);

    expect(axes.inTrack[0]).toBeCloseTo(0, 10);
    expect(axes.inTrack[1]).toBeCloseTo(0, 10);
    expect(axes.inTrack[2]).toBeCloseTo(1, 10);

    expect(axes.crossTrack[0]).toBeCloseTo(0, 10);
    expect(axes.crossTrack[1]).toBeCloseTo(-1, 10);
    expect(axes.crossTrack[2]).toBeCloseTo(0, 10);
  });

  it("all axes are unit vectors", () => {
    const axes = computeLvlhAxes([3000, 4000, 1000], [1.5, -2.0, 6.0])!;
    expect(mag(axes.radial)).toBeCloseTo(1, 10);
    expect(mag(axes.inTrack)).toBeCloseTo(1, 10);
    expect(mag(axes.crossTrack)).toBeCloseTo(1, 10);
  });

  it("all axes are mutually orthogonal", () => {
    const axes = computeLvlhAxes([3000, 4000, 1000], [1.5, -2.0, 6.0])!;
    expect(dot(axes.radial, axes.inTrack)).toBeCloseTo(0, 10);
    expect(dot(axes.radial, axes.crossTrack)).toBeCloseTo(0, 10);
    expect(dot(axes.inTrack, axes.crossTrack)).toBeCloseTo(0, 10);
  });

  it("forms a right-handed coordinate system (R × I = C... actually C × R = I)", () => {
    const axes = computeLvlhAxes([3000, 4000, 1000], [1.5, -2.0, 6.0])!;
    // crossTrack × radial = inTrack
    const cx = axes.crossTrack[1] * axes.radial[2] - axes.crossTrack[2] * axes.radial[1];
    const cy = axes.crossTrack[2] * axes.radial[0] - axes.crossTrack[0] * axes.radial[2];
    const cz = axes.crossTrack[0] * axes.radial[1] - axes.crossTrack[1] * axes.radial[0];
    expect(cx).toBeCloseTo(axes.inTrack[0], 10);
    expect(cy).toBeCloseTo(axes.inTrack[1], 10);
    expect(cz).toBeCloseTo(axes.inTrack[2], 10);
  });
});
