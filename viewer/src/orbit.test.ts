import { describe, expect, it } from "vitest";
import { lerpPoint, type OrbitPoint } from "./orbit.js";

/** A minimal OrbitPoint with all required fields zeroed; override as needed. */
function pt(overrides: Partial<OrbitPoint> = {}): OrbitPoint {
  return {
    t: 0,
    x: 0,
    y: 0,
    z: 0,
    vx: 0,
    vy: 0,
    vz: 0,
    a: 7000,
    e: 0,
    inc: 0,
    raan: 0,
    omega: 0,
    nu: 0,
    ...overrides,
  };
}

const S = Math.SQRT1_2; // sin/cos(45°)

describe("lerpPoint quaternion handling", () => {
  it("slerps when both points carry a complete quaternion", () => {
    // identity → 90° about Z, halfway = 45° about Z (normalized)
    const a = pt({ qw: 1, qx: 0, qy: 0, qz: 0 });
    const b = pt({ qw: S, qx: 0, qy: 0, qz: S });
    const r = lerpPoint(a, b, 0.5);
    for (const c of [r.qw, r.qx, r.qy, r.qz]) expect(c).toBeTypeOf("number");
    const mag = Math.hypot(r.qw as number, r.qx as number, r.qy as number, r.qz as number);
    expect(mag).toBeCloseTo(1, 6);
    expect(r.qz as number).toBeGreaterThan(0); // rotated partway toward b
  });

  it("skips attitude interpolation when a quaternion is incomplete (qw only)", () => {
    // Guarding on qw alone would build an un-normalized (0,0,0,qw) rotation.
    const a = pt({ qw: 0.5 }); // qx/qy/qz missing
    const b = pt({ qw: S, qx: 0, qy: 0, qz: S });
    const r = lerpPoint(a, b, 0.5);
    expect(r.qw).toBeUndefined();
    expect(r.qx).toBeUndefined();
    expect(r.qy).toBeUndefined();
    expect(r.qz).toBeUndefined();
  });

  it("leaves the result quaternion-free when neither point has one", () => {
    const r = lerpPoint(pt(), pt(), 0.5);
    expect(r.qw).toBeUndefined();
    expect(r.qz).toBeUndefined();
  });

  it("interpolates the non-quaternion fields regardless", () => {
    const r = lerpPoint(pt({ t: 0, x: 0 }), pt({ t: 10, x: 100 }), 0.3);
    expect(r.t).toBeCloseTo(3);
    expect(r.x).toBeCloseTo(30);
  });

  it("does not reject a present-but-NaN component (only missing ones are guarded)", () => {
    // Characterization: the guard checks presence, not finiteness; a NaN that is
    // *present* still enters the slerp path (out of scope for the missing-field fix).
    const a = pt({ qw: Number.NaN, qx: 0, qy: 0, qz: 0 });
    const b = pt({ qw: 1, qx: 0, qy: 0, qz: 0 });
    const r = lerpPoint(a, b, 0.5);
    expect(r.qw).toBeDefined();
  });
});
