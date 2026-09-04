import * as THREE from "three";
import { describe, expect, it } from "vitest";
import {
  type DisplayFrame,
  type DisplayFrameInputs,
  type DisplayOrientation,
  displayDirection,
  displayPosition,
  displayQuaternion,
  displayRotation,
  needsFullRewrite,
  type Quat,
  resolveDisplayFrame,
  resolveDisplayOrientation,
  trailTransformKey,
  unitAttitude,
  type Vec3,
} from "./displayFrame.js";
import type { ReferenceFrame } from "./referenceFrame.js";
import { computeLvlhAxes, type LvlhAxes } from "./sceneFrame.js";
import { isArikaReady } from "./wasm/arikaInit.js";

const EARTH_RADIUS = 6378.137;
const MU = 398600.4418;

const ECEF_FRAME: ReferenceFrame = { center: { type: "central_body" }, orientation: "body_fixed" };
const ECI_FRAME: ReferenceFrame = { center: { type: "central_body" }, orientation: "inertial" };
const SAT_LVLH_FRAME: ReferenceFrame = {
  center: { type: "satellite", id: "sat" },
  orientation: "local_orbital",
};

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

function quatFromEuler(x: number, y: number, z: number): Quat {
  const q = new THREE.Quaternion().setFromEuler(new THREE.Euler(x, y, z));
  return [q.w, q.x, q.y, q.z];
}

/** Rotate `b` by the Hamilton [w,x,y,z] quaternion `q`. */
function rotate(q: Quat, b: Vec3): Vec3 {
  const v = new THREE.Vector3(...b).applyQuaternion(new THREE.Quaternion(q[1], q[2], q[3], q[0]));
  return [v.x, v.y, v.z];
}

/**
 * Cross-path invariant: a body axis drawn by the attitude path must point where
 * the *position* path puts a point offset along that same axis.
 *
 * Take a body unit axis `b` and a small offset ε [km]. The point
 * `r + ε·(R(q_body→inertial)·b)` is a physical position, so pushing it through
 * `displayPosition` gives where the tip of the body axis really is in the scene.
 * The attitude path draws that tip at `scenePos + (ε/scale)·R(q_display)·b`.
 * `displayPosition` is affine in the offset, so for an exact frame the two agree
 * to floating-point noise; a mismatched basis (or a missing ERA rotation) leaves
 * an O(1) residual relative to the offset length.
 *
 * Returns the residual normalised by the scene-space offset length.
 */
function axisResidual(
  frame: DisplayFrame,
  r: Vec3,
  bodyToInertial: Quat,
  b: Vec3,
  scaleRadius: number,
): number {
  const eps = 1e-3; // km
  const inertialAxis = rotate(bodyToInertial, b);
  const tip = displayPosition(
    frame,
    r[0] + eps * inertialAxis[0],
    r[1] + eps * inertialAxis[1],
    r[2] + eps * inertialAxis[2],
    scaleRadius,
  );

  const base = displayPosition(frame, r[0], r[1], r[2], scaleRadius);
  const displayed = displayQuaternion(frame, bodyToInertial);
  if (displayed == null) throw new Error("attitude path returned no quaternion");
  const sceneAxis = rotate(displayed, b);
  const sceneEps = eps / scaleRadius;

  return (
    Math.hypot(
      base[0] + sceneEps * sceneAxis[0] - tip[0],
      base[1] + sceneEps * sceneAxis[1] - tip[1],
      base[2] + sceneEps * sceneAxis[2] - tip[2],
    ) / sceneEps
  );
}

const BODY_AXES: Vec3[] = [
  [1, 0, 0],
  [0, 1, 0],
  [0, 0, 1],
];

const ATTITUDES: Quat[] = [
  [1, 0, 0, 0],
  quatFromEuler(0.3, -0.5, 1.1),
  quatFromEuler(-1.7, 0.9, 0.2),
];

describe("position/attitude cross-path invariant", () => {
  it("holds in the local-orbital (LVLH) frame across orbit phases and inclinations", () => {
    for (const phase of [0, 0.7, 2.1, 4.9]) {
      for (const inc of [0, 0.6, Math.PI / 2]) {
        const { r, v } = orbitState(7378, phase, inc);
        const axes = computeLvlhAxes(r, v);
        expect(axes).not.toBeNull();
        if (axes == null) return;
        const frame = resolveDisplayFrame(SAT_LVLH_FRAME, {
          originPosition: r,
          lvlhAxes: axes,
        });
        expect(frame.kind).toBe("localOrbital");

        for (const attitude of ATTITUDES) {
          for (const b of BODY_AXES) {
            expect(axisResidual(frame, r, attitude, b, EARTH_RADIUS)).toBeLessThan(1e-6);
          }
        }
      }
    }
  });

  it("holds in the body-fixed (ECEF) frame, including ERA = 90°", () => {
    for (const era of [0, Math.PI / 2, 2.4, -1.1]) {
      const { r } = orbitState(7378, 0.9, 0.4);
      const frame = resolveDisplayFrame(ECEF_FRAME, { era });
      expect(frame.kind).toBe("bodyFixed");

      for (const attitude of ATTITUDES) {
        for (const b of BODY_AXES) {
          expect(axisResidual(frame, r, attitude, b, EARTH_RADIUS)).toBeLessThan(1e-6);
        }
      }
    }
  });

  it("holds in the satellite-centred inertial frame (origin offset only)", () => {
    const { r } = orbitState(7000, 1.3, 0.2);
    const frame = resolveDisplayFrame(
      { center: { type: "satellite", id: "sat" }, orientation: "inertial" },
      { originPosition: [r[0] - 30, r[1] + 12, r[2] - 4] },
    );
    expect(frame.kind).toBe("inertial");
    for (const attitude of ATTITUDES) {
      for (const b of BODY_AXES) {
        expect(axisResidual(frame, r, attitude, b, EARTH_RADIUS)).toBeLessThan(1e-6);
      }
    }
  });
});

describe("displayPosition", () => {
  it("applies R_z(-ERA) in the body-fixed frame", () => {
    // Pins the arika convention ECEF = R_z(−ERA)·ECI: at ERA = 90°,
    // +X_ECI → −Y_ECEF (arika/src/frame.rs::from_era_90deg).
    const frame: DisplayFrame = { kind: "bodyFixed", era: Math.PI / 2 };
    const p = displayPosition(frame, 1, 0, 0, 1);
    expect(p[0]).toBeCloseTo(0, 12);
    expect(p[1]).toBeCloseTo(-1, 12);
    expect(p[2]).toBeCloseTo(0, 12);
  });

  it("leaves the polar axis untouched in the body-fixed frame", () => {
    const p = displayPosition({ kind: "bodyFixed", era: 1.234 }, 0, 0, 500, 100);
    expect(p[0]).toBeCloseTo(0, 12);
    expect(p[1]).toBeCloseTo(0, 12);
    expect(p[2]).toBeCloseTo(5, 12);
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
    expect(unitAttitude([1e-300, 0, 0, 0])).toBeUndefined();
  });

  it("normalises a quaternion whose components would overflow when squared", () => {
    // `Math.hypot` scales before squaring, so a large but finite attitude still
    // normalises instead of dividing by an infinite norm.
    const q = unitAttitude([1e200, 1e200, 0, 0]);
    if (q == null) throw new Error("a finite attitude must normalise");
    expect(Math.hypot(...q)).toBeCloseTo(1, 12);
  });
});

describe("displayQuaternion", () => {
  it("passes the body-to-inertial attitude through unchanged in the inertial frame", () => {
    // The inertial view is what tests/attitude-rendering.spec.ts pins: scene
    // axes coincide with ECI, so the delivered quaternion must reach the mesh
    // untouched.
    const attitude = quatFromEuler(0.3, -0.5, 1.1);
    const expected: Quat = [...attitude];
    expect(displayQuaternion({ kind: "inertial", origin: null }, attitude)).toEqual(expected);
    expect(displayQuaternion({ kind: "inertial", origin: [1, 2, 3] }, attitude)).toEqual(expected);
  });

  it("returns undefined when there is no attitude", () => {
    expect(displayQuaternion({ kind: "bodyFixed", era: 1 }, undefined)).toBeUndefined();
  });

  it("maps the body axes onto the LVLH scene axes [in-track, cross-track, radial]", () => {
    // Equatorial orbit at +X: radial = +X, in-track = +Y, cross-track = +Z.
    // A body aligned with the orbit frame in *RSW* order (body X = radial) must
    // render along the scene's radial axis, which is +Z — not +X. Using arika's
    // [R,S,W] quaternion here instead of this basis rotates every axis by a
    // 120° cyclic permutation.
    const { r, v } = orbitState(7378, 0, 0);
    const axes = computeLvlhAxes(r, v);
    if (axes == null) throw new Error("degenerate orbit");
    const frame: DisplayFrame = { kind: "localOrbital", origin: r, axes };

    const radialInScene = rotate(displayQuaternion(frame, [1, 0, 0, 0]) as Quat, [1, 0, 0]);
    expect(radialInScene[0]).toBeCloseTo(0, 12);
    expect(radialInScene[1]).toBeCloseTo(0, 12);
    expect(radialInScene[2]).toBeCloseTo(1, 12);
  });

  it("builds the local-orbital attitude without the arika WASM module", () => {
    // The LVLH attitude used to come from arika's `body_quat_to_rsw`, which an
    // embedder's first render reaches before `initArika()` resolves — throwing
    // "not a function" and taking down the React subtree. Deriving it from the
    // LVLH basis removes the dependency entirely.
    expect(isArikaReady(), "this test is only meaningful with the WASM unloaded").toBe(false);
    const { r, v } = orbitState(7378, 1.1, 0.3);
    const axes = computeLvlhAxes(r, v);
    if (axes == null) throw new Error("degenerate orbit");
    const frame = resolveDisplayFrame(SAT_LVLH_FRAME, { originPosition: r, lvlhAxes: axes });
    expect(displayQuaternion(frame, quatFromEuler(0.1, 0.2, 0.3))).toBeDefined();
  });

  it("does not mutate the input quaternion", () => {
    const { r, v } = orbitState(6900, 0.5, 0.2);
    const axes = computeLvlhAxes(r, v);
    if (axes == null) throw new Error("degenerate orbit");
    const frames: DisplayFrame[] = [
      { kind: "bodyFixed", era: 0.7 },
      { kind: "localOrbital", origin: r, axes },
      { kind: "inertial", origin: null },
    ];
    for (const frame of frames) {
      const attitude: Quat = quatFromEuler(0.2, 0.4, 0.6);
      const copy: Quat = [...attitude];
      displayQuaternion(frame, attitude);
      expect(attitude).toEqual(copy);
    }
  });
});

describe("displayRotation", () => {
  it("rotates inertial coordinates exactly like displayPosition", () => {
    const { r, v } = orbitState(7100, 2.2, 0.5);
    const axes = computeLvlhAxes(r, v);
    if (axes == null) throw new Error("degenerate orbit");
    const frames: DisplayFrame[] = [
      { kind: "bodyFixed", era: 1.9 },
      { kind: "localOrbital", origin: [0, 0, 0], axes },
      { kind: "inertial", origin: null },
    ];
    for (const frame of frames) {
      const viaPosition = displayPosition(frame, r[0], r[1], r[2], 1);
      const viaRotation = new THREE.Vector3(...r).applyQuaternion(displayRotation(frame));
      expect(viaRotation.x).toBeCloseTo(viaPosition[0], 6);
      expect(viaRotation.y).toBeCloseTo(viaPosition[1], 6);
      expect(viaRotation.z).toBeCloseTo(viaPosition[2], 6);
    }
  });

  it("writes into the provided output quaternion", () => {
    const out = new THREE.Quaternion();
    expect(displayRotation({ kind: "bodyFixed", era: 0.5 }, out)).toBe(out);
  });

  it("agrees with displayDirection", () => {
    // A direction can be rotated through the quaternion (what an attitude goes
    // through) or through displayDirection (what the drawn Sun/nadir arrows go
    // through). Pin the two paths against each other so the lighting, the
    // arrows and the geometry cannot drift apart.
    const { r, v } = orbitState(7200, 3.3, 0.8);
    const axes = computeLvlhAxes(r, v);
    if (axes == null) throw new Error("degenerate orbit");
    const dir: Vec3 = [0.4, -0.7, 0.59];
    const frames: DisplayFrame[] = [
      { kind: "bodyFixed", era: 1.4 },
      { kind: "localOrbital", origin: r, axes },
      { kind: "inertial", origin: r },
    ];
    for (const frame of frames) {
      const expected = displayDirection(frame, dir);
      const got = new THREE.Vector3(...dir).applyQuaternion(displayRotation(frame));
      expect(got.x).toBeCloseTo(expected[0], 12);
      expect(got.y).toBeCloseTo(expected[1], 12);
      expect(got.z).toBeCloseTo(expected[2], 12);
    }
  });
});

describe("displayDirection", () => {
  const ECI_SUN: Vec3 = [0.6, 0, 0.8];
  const mag = (v: Vec3): number => Math.sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);

  it("returns the inertial direction unchanged in the inertial frame", () => {
    expect(displayDirection({ kind: "inertial", origin: null }, ECI_SUN)).toEqual(ECI_SUN);
    // An origin offset moves positions, never directions.
    expect(displayDirection({ kind: "inertial", origin: [7000, 0, 0] }, ECI_SUN)).toEqual(ECI_SUN);
  });

  it("rotates by -ERA about Z in the body-fixed frame", () => {
    // ERA = +π/2: an inertial +X direction maps to -Y in the Earth-fixed frame.
    const out = displayDirection({ kind: "bodyFixed", era: Math.PI / 2 }, [1, 0, 0]);
    expect(out[0]).toBeCloseTo(0, 12);
    expect(out[1]).toBeCloseTo(-1, 12);
    expect(out[2]).toBeCloseTo(0, 12);
  });

  it("projects onto the LVLH basis [in-track, cross-track, radial]", () => {
    // Permuted orthonormal basis: inertial +X lands on the radial (3rd) component.
    const axes: LvlhAxes = { inTrack: [0, 1, 0], crossTrack: [0, 0, 1], radial: [1, 0, 0] };
    const out = displayDirection({ kind: "localOrbital", origin: [7000, 0, 0], axes }, [1, 0, 0]);
    expect(out[0]).toBeCloseTo(0, 12);
    expect(out[1]).toBeCloseTo(0, 12);
    expect(out[2]).toBeCloseTo(1, 12);
  });

  it("preserves magnitude under the orthonormal LVLH projection", () => {
    const { r, v } = orbitState(7000, 1.1, 0.6);
    const axes = computeLvlhAxes(r, v);
    if (axes == null) throw new Error("degenerate orbit");
    const out = displayDirection({ kind: "localOrbital", origin: r, axes }, ECI_SUN);
    expect(mag(out)).toBeCloseTo(mag(ECI_SUN), 12);
  });

  it("does not mutate the input", () => {
    const input: Vec3 = [0.6, 0, 0.8];
    displayDirection({ kind: "bodyFixed", era: 0.9 }, input);
    expect(input).toEqual([0.6, 0, 0.8]);
  });

  it("depends on the frame's rotation, not its origin", () => {
    // The invariant behind keying the Sun direction's memo on the rotation
    // alone: a satellite-centred origin moves with every sample, and moving it
    // must not change a single direction.
    const { r, v } = orbitState(7000, 2.4, 0.5);
    const axes = computeLvlhAxes(r, v);
    if (axes == null) throw new Error("degenerate orbit");
    expect(displayDirection({ kind: "localOrbital", origin: r, axes }, ECI_SUN)).toEqual(
      displayDirection({ kind: "localOrbital", origin: [0, 0, 0], axes }, ECI_SUN),
    );
    expect(displayDirection({ kind: "inertial", origin: r }, ECI_SUN)).toEqual(
      displayDirection({ kind: "inertial", origin: null }, ECI_SUN),
    );
  });
});

describe("resolveDisplayFrame", () => {
  it("needs an ERA to rotate: an ECEF frame without one stays inertial", () => {
    // The epoch (hence ERA) can arrive after the first points do; falling back
    // to inertial keeps position and attitude in one basis either way.
    expect(resolveDisplayFrame(ECEF_FRAME, {}).kind).toBe("inertial");
    expect(resolveDisplayFrame(ECEF_FRAME, { era: null }).kind).toBe("inertial");
  });

  it("ignores an ERA in frames that are not body-fixed", () => {
    expect(resolveDisplayFrame(ECI_FRAME, { era: 1.2 }).kind).toBe("inertial");
  });

  it("requires both an origin and axes for the local-orbital frame", () => {
    const { r, v } = orbitState(7000, 0.4, 0.1);
    const axes = computeLvlhAxes(r, v);
    if (axes == null) throw new Error("degenerate orbit");
    expect(resolveDisplayFrame(SAT_LVLH_FRAME, { lvlhAxes: axes }).kind).toBe("inertial");
    expect(resolveDisplayFrame(SAT_LVLH_FRAME, { originPosition: r }).kind).toBe("inertial");
    expect(resolveDisplayFrame(SAT_LVLH_FRAME, { originPosition: r, lvlhAxes: axes }).kind).toBe(
      "localOrbital",
    );
  });

  it("keeps the local-orbital transform when an ERA is also available", () => {
    // A satellite-centred LVLH view re-bases the data; the central body's
    // rotation must not also be applied on top of it.
    const { r, v } = orbitState(7000, 2.1, 0.9);
    const axes = computeLvlhAxes(r, v);
    if (axes == null) throw new Error("degenerate orbit");
    expect(
      resolveDisplayFrame(SAT_LVLH_FRAME, { era: Math.PI / 3, originPosition: r, lvlhAxes: axes })
        .kind,
    ).toBe("localOrbital");
  });

  it("resolves through resolveDisplayOrientation for every frame", () => {
    // resolveDisplayFrame derives the orientation from the reference frame and
    // then defers to the shared kernel — the same kernel the attitude view calls
    // with an orientation alone. Pin the two so the views cannot diverge.
    const { r, v } = orbitState(7100, 0.7, 0.35);
    const axes = computeLvlhAxes(r, v);
    if (axes == null) throw new Error("degenerate orbit");
    const cases: {
      frame: ReferenceFrame;
      orientation: DisplayOrientation;
      inputs: DisplayFrameInputs;
    }[] = [
      { frame: ECEF_FRAME, orientation: "bodyFixed", inputs: { era: 1.4 } },
      { frame: ECEF_FRAME, orientation: "bodyFixed", inputs: { era: null } },
      // Non-finite ERA. The gate is `era != null`, so these reach the kernel as
      // they did before the wrapper existed; a refactor of floating-point code
      // has to say what it does with them rather than leave it to be discovered.
      { frame: ECEF_FRAME, orientation: "bodyFixed", inputs: { era: Number.NaN } },
      { frame: ECEF_FRAME, orientation: "bodyFixed", inputs: { era: Number.POSITIVE_INFINITY } },
      { frame: ECEF_FRAME, orientation: "bodyFixed", inputs: { era: Number.NEGATIVE_INFINITY } },
      { frame: ECI_FRAME, orientation: "inertial", inputs: { era: 1.4 } },
      { frame: ECI_FRAME, orientation: "inertial", inputs: { originPosition: r } },
      {
        frame: SAT_LVLH_FRAME,
        orientation: "localOrbital",
        inputs: { originPosition: r, lvlhAxes: axes },
      },
      { frame: SAT_LVLH_FRAME, orientation: "inertial", inputs: { originPosition: r } },
      // A body-fixed frame without an ERA cannot rotate, so the local-orbital
      // geometry (if any) still decides — the ERA gate is part of choosing the
      // orientation, not only of granting it.
      {
        frame: ECEF_FRAME,
        orientation: "localOrbital",
        inputs: { era: null, originPosition: r, lvlhAxes: axes },
      },
    ];
    for (const { frame, orientation, inputs } of cases) {
      expect(resolveDisplayFrame(frame, inputs)).toEqual(
        resolveDisplayOrientation(orientation, inputs),
      );
    }
  });

  it("treats a non-finite ERA as no ERA, in both entry points", () => {
    // An ERA of NaN would otherwise reach every rotation the frame produces —
    // the spacecraft's quaternion, each direction, the camera — and the scene
    // would come out blank instead of falling back to inertial.
    const { r, v } = orbitState(7100, 0.7, 0.35);
    const axes = computeLvlhAxes(r, v);
    if (axes == null) throw new Error("degenerate orbit");
    for (const era of [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY]) {
      expect(resolveDisplayOrientation("bodyFixed", { era }).kind).toBe("inertial");
      expect(resolveDisplayFrame(ECEF_FRAME, { era }).kind).toBe("inertial");
      // With the local-orbital geometry present it is that frame, not a
      // body-fixed one built on a NaN angle, that the request falls back to.
      expect(
        resolveDisplayFrame(SAT_LVLH_FRAME, { era, originPosition: r, lvlhAxes: axes }).kind,
      ).toBe("localOrbital");
    }
    // A finite ERA of zero is a real angle and still grants the request.
    expect(resolveDisplayOrientation("bodyFixed", { era: 0 })).toEqual({
      kind: "bodyFixed",
      era: 0,
    });
  });
});

describe("trail transform key", () => {
  it("requires a rewrite when the ECEF epoch arrives late (null → value)", () => {
    // A CSV load streams every point before the `info` event that carries the
    // epoch, so vertices baked in ECI must be re-encoded once it lands.
    const before = trailTransformKey(ECEF_FRAME, undefined);
    const after = trailTransformKey(ECEF_FRAME, 2451545.0);
    expect(before.ecefEpochJd).toBeNull();
    expect(needsFullRewrite(before, after)).toBe(true);
    expect(needsFullRewrite(after, before)).toBe(true);
  });

  it("requires a rewrite when the ECEF epoch changes", () => {
    expect(
      needsFullRewrite(
        trailTransformKey(ECEF_FRAME, 2451545.0),
        trailTransformKey(ECEF_FRAME, 2460000.5),
      ),
    ).toBe(true);
  });

  it("does not rewrite as time advances with the same frame and epoch", () => {
    const key = trailTransformKey(ECEF_FRAME, 2451545.0);
    expect(needsFullRewrite(key, trailTransformKey(ECEF_FRAME, 2451545.0))).toBe(false);
    expect(needsFullRewrite(key, key)).toBe(false);
  });

  it("ignores the epoch outside the body-fixed frame (inertial vertices are epoch-free)", () => {
    expect(
      needsFullRewrite(
        trailTransformKey(ECI_FRAME, undefined),
        trailTransformKey(ECI_FRAME, 2451545.0),
      ),
    ).toBe(false);
  });

  it("requires a rewrite when the frame orientation or centre changes", () => {
    expect(
      needsFullRewrite(
        trailTransformKey(ECI_FRAME, 2451545.0),
        trailTransformKey(ECEF_FRAME, 2451545.0),
      ),
    ).toBe(true);
    expect(
      needsFullRewrite(
        trailTransformKey(
          { center: { type: "satellite", id: "a" }, orientation: "inertial" },
          null,
        ),
        trailTransformKey(
          { center: { type: "satellite", id: "b" }, orientation: "inertial" },
          null,
        ),
      ),
    ).toBe(true);
  });
});
