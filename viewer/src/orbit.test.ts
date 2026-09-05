import { describe, expect, it } from "vitest";
import { sampleAttitude } from "./displayFrame.js";
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

  it("interpolates the same rotation whatever norms the endpoints drifted to", () => {
    // A simulator's attitude drifts off unit norm as it integrates, and the two
    // endpoints drift by different amounts. Slerp assumes unit inputs, so the
    // rotation it picks depends on the ratio between the norms — normalising the
    // result afterwards cannot recover it, because what went wrong is which
    // rotation was chosen, not how long it is.
    const unit = lerpPoint(
      pt({ qw: 1, qx: 0, qy: 0, qz: 0 }),
      pt({ qw: S, qx: 0, qy: 0, qz: S }),
      0.5,
    );
    const scaled = (ka: number, kb: number) =>
      lerpPoint(
        pt({ qw: 1 * ka, qx: 0, qy: 0, qz: 0 }),
        pt({ qw: S * kb, qx: 0, qy: 0, qz: S * kb }),
        0.5,
      );

    // Measured against the raw-endpoint slerp this replaces: norms of 1 and 2 put
    // each component 1.2e-1 out, and a drift of a thousandth 1.9e-4 out.
    for (const [ka, kb] of [
      [2, 2],
      [1, 2],
      [2, 1],
      [1.001, 1],
      [0.5, 1],
    ]) {
      const r = scaled(ka, kb);
      const norm = Math.hypot(r.qw as number, r.qx as number, r.qy as number, r.qz as number);
      expect(norm, `norms ${ka}/${kb} should give a unit result`).toBeCloseTo(1, 9);
      for (const [name, got, want] of [
        ["qw", r.qw, unit.qw],
        ["qx", r.qx, unit.qx],
        ["qy", r.qy, unit.qy],
        ["qz", r.qz, unit.qz],
      ] as const) {
        expect(got as number, `${name} for norms ${ka}/${kb}`).toBeCloseTo(want as number, 9);
      }
    }
  });

  it("keeps a refused endpoint refused at every fraction between the samples", () => {
    // The interior is the case that matters, and testing only the endpoint hid
    // this: slerp from a zero quaternion returns a multiple of the *other*
    // endpoint, so a quarter of the way along the result normalised to that
    // endpoint's rotation exactly — a refused sample presented as a measurement
    // taken next door. Normalising the zero endpoint instead is no answer:
    // `THREE.Quaternion.normalize` answers the identity for it, which invents a
    // rotation of its own.
    const valid = pt({ qw: S, qx: 0, qy: 0, qz: S });
    for (const refused of [
      pt({ qw: 0, qx: 0, qy: 0, qz: 0 }),
      pt({ qw: Number.NaN, qx: 0, qy: 0, qz: 0 }),
      pt({ qw: Number.POSITIVE_INFINITY, qx: 0, qy: 0, qz: 0 }),
    ]) {
      for (const frac of [0, 0.25, 0.5, 0.75]) {
        const r = lerpPoint(refused, valid, frac);
        expect(
          sampleAttitude(r),
          `frac ${frac} should carry no usable attitude away from a refused sample`,
        ).toBeUndefined();
      }
      // The far endpoint is exact, which is what a reader at that sample's own
      // timestamp should see. Compared componentwise: normalising a unit
      // quaternion moves its last bit (measured 0.7071067811865475 for `S`).
      const atFar = sampleAttitude(lerpPoint(refused, valid, 1));
      expect(atFar, "the valid endpoint keeps its own attitude").not.toBeUndefined();
      for (const [i, want] of [S, 0, 0, S].entries()) {
        expect((atFar as number[])[i]).toBeCloseTo(want, 12);
      }
      // And in the other order, so the refusal is not tied to being first.
      for (const frac of [0.25, 0.5, 0.75, 1]) {
        expect(sampleAttitude(lerpPoint(valid, refused, frac)), `reversed frac ${frac}`) //
          .toBeUndefined();
      }
      const atNear = sampleAttitude(lerpPoint(valid, refused, 0));
      expect(atNear, "and in the other order").not.toBeUndefined();
      for (const [i, want] of [S, 0, 0, S].entries()) {
        expect((atNear as number[])[i]).toBeCloseTo(want, 12);
      }
    }
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
