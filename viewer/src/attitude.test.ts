import * as THREE from "three";
import { describe, expect, it } from "vitest";

import { type AttitudeComponents, resolveAttitude, unitAttitude } from "./attitude.js";
import type { Quat, Vec3 } from "./lib/types.js";

function quatFromEuler(x: number, y: number, z: number): Quat {
  const q = new THREE.Quaternion().setFromEuler(new THREE.Euler(x, y, z));
  return [q.w, q.x, q.y, q.z];
}

/** Rotate `b` by the Hamilton [w,x,y,z] quaternion `q`. */
function rotate(q: Quat, b: Vec3): Vec3 {
  const v = new THREE.Vector3(...b).applyQuaternion(new THREE.Quaternion(q[1], q[2], q[3], q[0]));
  return [v.x, v.y, v.z];
}

describe("resolveAttitude", () => {
  /** Each row is what the viewer reads from one sample. */
  const cases: [string, AttitudeComponents, "absent" | "refused" | "usable"][] = [
    ["nothing arrived", {}, "absent"],
    // `null` is how a file says "no value here", not a claim of one.
    ["every component null", { qw: null, qx: null, qy: null, qz: null }, "absent"],
    ["one component null", { qw: null }, "absent"],
    // A claim the viewer cannot turn into a rotation. Filling the missing
    // components with zero would make `{ qw: 0.5 }` the identity.
    ["a partial claim", { qw: 0.5 }, "refused"],
    ["a partial claim of one axis", { qx: 1 }, "refused"],
    // Non-finite, tested on a complete tuple so it is the finiteness that
    // decides and not the incompleteness.
    ["a complete tuple with NaN", { qw: 1, qx: Number.NaN, qy: 0, qz: 0 }, "refused"],
    [
      "a complete tuple with an infinity",
      { qw: 1, qx: Number.POSITIVE_INFINITY, qy: 0, qz: 0 },
      "refused",
    ],
    ["a zero quaternion", { qw: 0, qx: 0, qy: 0, qz: 0 }, "refused"],
    // Subnormal: `Math.hypot` answers the smallest number there is, and both
    // components divide to 1, leaving a norm of 1.414.
    ["subnormal components", { qw: 5e-324, qx: 5e-324, qy: 0, qz: 0 }, "refused"],
    ["a unit quaternion", { qw: 1, qx: 0, qy: 0, qz: 0 }, "usable"],
    ["a quaternion to normalise", { qw: 2, qx: 0, qy: 0, qz: 0 }, "usable"],
    // The identity written small normalises exactly.
    ["a small identity", { qw: 1e-300, qx: 0, qy: 0, qz: 0 }, "usable"],
  ];

  for (const [name, sample, kind] of cases) {
    it(`reads ${name} as ${kind}`, () => {
      expect(resolveAttitude(sample).kind).toBe(kind);
    });
  }

  it("brings a usable quaternion to unit norm", () => {
    const state = resolveAttitude({ qw: 2, qx: 0, qy: 0, qz: 0 });

    expect(state.kind).toBe("usable");
    if (state.kind !== "usable") return;
    expect(state.quaternion).toEqual([1, 0, 0, 0]);
  });

  // The three states are what every consequence reads, so a consumer cannot
  // reconstruct them from two predicates and get a different answer.
  it("answers exactly one state per sample", () => {
    for (const [, sample, kind] of cases) {
      const state = resolveAttitude(sample);
      expect(state.kind).toBe(kind);
      expect(state.kind === "usable" ? "quaternion" in state : !("quaternion" in state)).toBe(true);
    }
  });
});

describe("an incomplete sample", () => {
  /** The rotation a sample resolves to, or undefined when it is not usable. */
  function quaternionOf(sample: AttitudeComponents) {
    const state = resolveAttitude(sample);
    return state.kind === "usable" ? state.quaternion : undefined;
  }

  it("refuses a quaternion that is missing components", () => {
    // Zero-filling would make each of these the identity after normalisation, so
    // a sample carrying one number would be drawn as a measured orientation.
    expect(quaternionOf({ qw: 0.5 })).toBeUndefined();
    expect(quaternionOf({ qw: 1, qx: 0 })).toBeUndefined();
    expect(quaternionOf({ qw: 1, qx: 0, qy: 0 })).toBeUndefined();
    // A complete tuple still resolves, including one that needs normalising.
    expect(quaternionOf({ qw: 1, qx: 0, qy: 0, qz: 0 })).toEqual([1, 0, 0, 0]);
    expect(quaternionOf({ qw: 2, qx: 0, qy: 0, qz: 0 })).toEqual([1, 0, 0, 0]);
  });

  it("counts an incomplete quaternion as an attitude that was refused", () => {
    // The sample claimed one — `qw` is there — and the viewer cannot use it, which
    // is the state that suppresses the model and the orientation-revealing cube.
    expect(resolveAttitude({ qw: 0.5 }).kind).toBe("refused");
    // Any component is the claim, `qw` included but not required — a sample that
    // carries one of the others is a partly decoded attitude, not an absent one,
    // and reading it as absent would leave a registered model on screen at its own
    // orientation with the scene amplified to match.
    expect(resolveAttitude({ qx: 0.5 }).kind).toBe("refused");
    expect(resolveAttitude({ qy: Number.NaN }).kind).toBe("refused");
    expect(resolveAttitude({ qz: 0 }).kind).toBe("refused");
    expect(resolveAttitude({ qx: 0, qy: 0, qz: 0 }).kind).toBe("refused");
    // Nothing claimed, nothing refused.
    expect(resolveAttitude({}).kind).toBe("absent");
    expect(resolveAttitude({ qw: 1, qx: 0, qy: 0, qz: 0 }).kind).toBe("usable");
  });
});

describe("unitAttitude", () => {
  it("normalises an attitude that has drifted off unit norm", () => {
    // Three.js applies the components without normalising, so a norm of 1.05
    // would scale the spacecraft by 1.05² through its rotation matrix.
    const drifted: Quat = [1.05, 0, 0, 0];
    const q = unitAttitude(drifted);
    if (q == null) throw new Error("a finite non-zero attitude must normalise");
    expect(Math.hypot(...q)).toBeCloseTo(1, 12);
    // The rotation itself is unchanged: scaling a quaternion scales its norm,
    // not the rotation it names.
    const axis: Vec3 = [0, 1, 0];
    expect(rotate(q, axis)).toEqual(rotate([1, 0, 0, 0], axis));
  });

  it("leaves a unit attitude alone", () => {
    const q = quatFromEuler(0.3, -0.5, 1.1);
    const out = unitAttitude(q);
    if (out == null) throw new Error("a unit attitude must survive");
    for (const i of [0, 1, 2, 3]) expect(out[i]).toBeCloseTo(q[i], 12);
  });

  it("reports no attitude for input that names no rotation", () => {
    // Each of these would otherwise reach the scene matrices: the zero
    // quaternion collapses the spacecraft, a NaN component spreads.
    expect(unitAttitude(undefined)).toBeUndefined();
    expect(unitAttitude([0, 0, 0, 0])).toBeUndefined();
    expect(unitAttitude([Number.NaN, 0, 0, 0])).toBeUndefined();
    expect(unitAttitude([1, Number.POSITIVE_INFINITY, 0, 0])).toBeUndefined();
  });

  it("normalises a rotation written at a tiny scale", () => {
    // A small magnitude is not a missing rotation: these divide to the identity
    // exactly. The previous cutoff at 1e-10 turned them into "no attitude" on
    // nothing but their scale — `[1e-300, 0, 0, 0]` was pinned as invalid here.
    for (const q of [
      [1e-11, 0, 0, 0],
      [1e-300, 0, 0, 0],
      [5e-324, 0, 0, 0],
    ] as Quat[]) {
      const out = unitAttitude(q);
      if (out == null) throw new Error(`${q.join(",")} names the identity rotation`);
      expect(out).toEqual([1, 0, 0, 0]);
    }
  });

  it("reports no attitude when the normalised result is not a unit quaternion", () => {
    // At subnormal magnitudes the division does not land on the unit sphere:
    // `Math.hypot([5e-324, 5e-324, 0, 0])` is 5e-324, the smallest number there
    // is, so both components divide to 1 and the norm comes out at 1.414. Three
    // applies the components as written, so that would scale the spacecraft by
    // 41% — the check reads the result rather than the input's scale.
    for (const q of [
      [5e-324, 5e-324, 0, 0],
      [1e-320, 1e-320, 0, 0],
    ] as Quat[]) {
      expect(unitAttitude(q), `${q.join(",")} cannot be normalised`).toBeUndefined();
    }
  });

  it("normalises a quaternion whose components would overflow when squared", () => {
    // `Math.hypot` scales before squaring, which covers 1e200; its own result
    // overflows at the top of the range — measured: the norm of
    // `[MAX_VALUE, MAX_VALUE, 0, 0]` is Infinity. Both name the same rotation as
    // `[1, 1, 0, 0]`, since scaling a quaternion scales its norm and not the
    // rotation, so both normalise, and to that rotation.
    for (const large of [1e200, Number.MAX_VALUE]) {
      const q = unitAttitude([large, large, 0, 0]);
      if (q == null) throw new Error(`a finite attitude must normalise (${large})`);
      expect(Math.hypot(...q), `norm for ${large}`).toBeCloseTo(1, 12);
      for (const [i, want] of [Math.SQRT1_2, Math.SQRT1_2, 0, 0].entries()) {
        expect(q[i], `component ${i} for ${large}`).toBeCloseTo(want, 12);
      }
    }
  });
});
