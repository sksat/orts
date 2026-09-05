/**
 * Display-frame kernel: one definition of how the scene axes relate to the
 * central-body inertial frame, shared by positions *and* attitudes.
 *
 * A display frame is a rotation (inertial → scene axes) plus an origin. Both
 * halves of a rendered object are derived from the same {@link DisplayFrame}
 * value here, so a position and the attitude drawn at that position can never
 * follow different conventions:
 *
 * - `inertial` — scene axes are the central-body inertial axes; only the origin
 *   may be shifted (satellite-centred inertial view).
 * - `bodyFixed` — scene axes co-rotate with the central body: `R_z(−ERA)`,
 *   matching `arika`'s `SimpleEci → SimpleEcef` (`ECEF = R_z(−ERA)·ECI`).
 * - `localOrbital` — scene axes are the centred satellite's orbit frame in the
 *   viewer's [in-track, cross-track, radial] order, i.e. the basis
 *   `[Ŝ, Ŵ, R̂]`. Note this is *not* the [R̂, Ŝ, Ŵ] column order of the RSW
 *   convention used by `arika::rsw_quaternion`; the two differ by a cyclic axis
 *   permutation (a 120° rotation about (1,1,1)), which is exactly why the
 *   attitude must be built from this basis and not from an RSW quaternion.
 */

import * as THREE from "three";
import { transformToLvlh } from "./coordTransform.js";
import {
  type FrameCenter,
  type FrameOrientation,
  frameCenterEquals,
  isLegacyEcef,
  type ReferenceFrame,
} from "./referenceFrame.js";
import type { LvlhAxes } from "./sceneFrame.js";

/** Cartesian triple [x, y, z]. */
export type Vec3 = [number, number, number];

/** Hamilton scalar-first quaternion [w, x, y, z] — the viewer-wide convention. */
export type Quat = [number, number, number, number];

/** How the scene axes and origin relate to the central-body inertial frame. */
export type DisplayFrame =
  | { kind: "inertial"; origin: Vec3 | null }
  | { kind: "bodyFixed"; era: number }
  | { kind: "localOrbital"; origin: Vec3; axes: LvlhAxes };

/**
 * The rotation half of a display frame — all a *direction* depends on.
 *
 * A {@link DisplayFrame} is assignable to this, so callers holding a full frame
 * pass it straight through. Naming the rotation separately lets a consumer that
 * only rotates directions key its memoisation on the rotation alone: an origin
 * that moves with the centred satellite changes the frame on every sample
 * without changing a single direction.
 */
export type DisplayRotationFrame =
  | { kind: "inertial" }
  | { kind: "bodyFixed"; era: number }
  | { kind: "localOrbital"; axes: LvlhAxes };

/** Frame geometry a caller has already resolved for the current sample. */
export interface DisplayFrameInputs {
  /**
   * Earth Rotation Angle [rad] at the sample's own time, or null when unknown.
   * Required for (and only used by) the body-fixed frame; pass the value from
   * `earth_rotation_angle(epochJd, t)` so the same angle drives both halves.
   */
  era?: number | null;
  /** ECI position [km] of the frame centre, or null for the central body. */
  originPosition?: Vec3 | null;
  /**
   * Orbit-frame axes of the centred entity, as resolved by `resolveSceneFrame`.
   * Non-null means the local-orbital transform is active — that decision belongs
   * to the frame-resolution kernel and is not re-derived here.
   */
  lvlhAxes?: LvlhAxes | null;
}

/**
 * Which display frame a view is asking for, named after the frame it produces.
 *
 * A request is only granted when the geometry it needs is present — `bodyFixed`
 * needs an ERA, `localOrbital` needs the centred entity's basis — so the
 * requested orientation and the resolved {@link DisplayFrame} can differ, and
 * `inertial` is what a view falls back to.
 */
export type DisplayOrientation = DisplayFrame["kind"];

/**
 * Resolve the display frame from a requested orientation plus the geometry
 * available for this sample.
 *
 * This is the kernel both views resolve through. A view that centres its
 * spacecraft by construction (the attitude view) has no frame centre to resolve
 * and calls this directly; {@link resolveDisplayFrame} derives the orientation
 * from a {@link ReferenceFrame} first.
 */
export function resolveDisplayOrientation(
  orientation: DisplayOrientation,
  { era = null, originPosition = null, lvlhAxes = null }: DisplayFrameInputs,
): DisplayFrame {
  // A non-finite ERA is treated as no ERA. It would otherwise put NaN into every
  // rotation derived from this frame — the spacecraft's quaternion, each
  // direction, the camera — and the scene would come out blank rather than fall
  // back to inertial. Same reason `computeLvlhAxes` refuses non-finite state.
  if (orientation === "bodyFixed" && era != null && Number.isFinite(era)) {
    return { kind: "bodyFixed", era };
  }
  if (orientation === "localOrbital" && originPosition != null && lvlhAxes != null) {
    return { kind: "localOrbital", origin: originPosition, axes: lvlhAxes };
  }
  return { kind: "inertial", origin: originPosition };
}

/**
 * Resolve the display frame for one sample of a centre-and-orientation frame.
 *
 * The orientation follows the frame semantics: a body-fixed central-body view
 * rotates, a satellite-centred local-orbital view re-bases, anything else only
 * shifts the origin. Non-null `lvlhAxes` is what says the local-orbital
 * transform is active — that decision belongs to the frame-resolution kernel and
 * is not re-derived here.
 */
export function resolveDisplayFrame(
  referenceFrame: ReferenceFrame,
  inputs: DisplayFrameInputs,
): DisplayFrame {
  const orientation: DisplayOrientation =
    isLegacyEcef(referenceFrame) && inputs.era != null
      ? "bodyFixed"
      : inputs.lvlhAxes != null
        ? "localOrbital"
        : "inertial";
  return resolveDisplayOrientation(orientation, inputs);
}

/**
 * How far from unit a normalised quaternion may land and still be used.
 *
 * Above where the division lands for any magnitude a double represents normally —
 * a few multiples of the machine epsilon, 4.4e-16 across the four-component cases
 * measured — and far below the 1.3e-4 of the nearest case it has to reject.
 */
const UNIT_QUATERNION_TOLERANCE = 1e-9;

/**
 * A caller's attitude as a unit quaternion, or undefined when it does not name a
 * rotation.
 *
 * Three.js applies the components with `Quaternion.set`, which does not
 * normalise: a non-unit quaternion scales and skews the very spacecraft it is
 * meant to orient, and a non-finite component spreads through the scene
 * matrices. A simulator's attitude drifts off unit norm as it integrates, so the
 * fix is to normalise what can be normalised and reject the rest — an unusable
 * attitude then reads as *no* attitude, which the views already draw (no body
 * axes, and the marker that looks the same from every side).
 */
/**
 * The attitude carried by an orbit sample, brought to unit norm, or undefined.
 *
 * Samples arrive as loose components rather than a tuple, and two decisions read
 * them: the rotation applied to the marker, and whether the marker is the shape
 * that reveals an orientation. Both go through here so they cannot disagree —
 * a sample the rotation refuses must not be drawn as an oriented cube.
 */
export function sampleAttitude(sample: {
  qw?: number | null;
  qx?: number | null;
  qy?: number | null;
  qz?: number | null;
}): Quat | undefined {
  if (sample.qw == null) return undefined;
  return unitAttitude([sample.qw, sample.qx ?? 0, sample.qy ?? 0, sample.qz ?? 0]);
}

export function unitAttitude(attitude: Quat | undefined): Quat | undefined {
  if (attitude == null) return undefined;
  const [w, x, y, z] = attitude;
  const n = Math.hypot(w, x, y, z);
  if (!(Number.isFinite(n) && n > 0)) return undefined;
  const unit: Quat = [w / n, x / n, y / n, z / n];
  // The division has to land on the unit sphere, and at subnormal magnitudes it
  // does not: `Math.hypot` answers 5e-324 for `[5e-324, 5e-324, 0, 0]`, the
  // smallest number there is, so both components divide to 1 and the result has
  // a norm of 1.414 — which Three.js would apply as written, scaling the
  // spacecraft by 41%. In the normal range the same division lands a few
  // multiples of the machine epsilon from unit, so reading the result tells the
  // two apart without a rule about the input's scale: `[1e-300, 0, 0, 0]` is the
  // identity rotation written small, and it normalises exactly.
  return Math.abs(Math.hypot(...unit) - 1) < UNIT_QUATERNION_TOLERANCE ? unit : undefined;
}

/**
 * Express an inertial position [km] in the display frame, scaled to scene units.
 */
export function displayPosition(
  frame: DisplayFrame,
  x: number,
  y: number,
  z: number,
  scaleRadius: number,
): Vec3 {
  const invScale = 1 / scaleRadius;
  switch (frame.kind) {
    case "bodyFixed": {
      // ECEF = R_z(−ERA) · ECI (arika: Rotation<SimpleEci, SimpleEcef>::from_era)
      const c = Math.cos(frame.era);
      const s = Math.sin(frame.era);
      return [(c * x + s * y) * invScale, (-s * x + c * y) * invScale, z * invScale];
    }
    case "localOrbital":
      return transformToLvlh(x, y, z, frame.origin, frame.axes, scaleRadius);
    case "inertial": {
      const o = frame.origin;
      if (o == null) return [x * invScale, y * invScale, z * invScale];
      return [(x - o[0]) * invScale, (y - o[1]) * invScale, (z - o[2]) * invScale];
    }
  }
}

// Scratch objects: the display rotation is recomputed for every satellite on
// every rendered frame, and none of them escapes (callers get a fresh
// quaternion or a fresh tuple), so reusing them keeps the intermediates off the
// per-frame allocation path.
const scratchBasis = new THREE.Matrix4();
const scratchAxisA = new THREE.Vector3();
const scratchAxisB = new THREE.Vector3();
const scratchAxisC = new THREE.Vector3();
const scratchRotation = new THREE.Quaternion();
const scratchAttitude = new THREE.Quaternion();

/**
 * Rotation taking inertial vector components to display-frame components.
 *
 * Writes into `out` when given (avoiding an allocation) and returns it.
 * Applying this to a position is equivalent to {@link displayPosition} with a
 * null origin.
 */
export function displayRotation(frame: DisplayFrame, out?: THREE.Quaternion): THREE.Quaternion {
  const q = out ?? new THREE.Quaternion();
  switch (frame.kind) {
    case "bodyFixed":
      // R_z(−ERA), the rotation displayPosition applies to the coordinates.
      return q.setFromAxisAngle(scratchAxisA.set(0, 0, 1), -frame.era);
    case "localOrbital": {
      const { inTrack, crossTrack, radial } = frame.axes;
      // Basis columns [in-track, cross-track, radial] map scene → inertial, so
      // its conjugate maps inertial → scene, exactly as the dot products in
      // transformToLvlh do for positions.
      scratchBasis.makeBasis(
        scratchAxisA.set(...inTrack),
        scratchAxisB.set(...crossTrack),
        scratchAxisC.set(...radial),
      );
      return q.setFromRotationMatrix(scratchBasis).conjugate();
    }
    case "inertial":
      return q.identity();
  }
}

/**
 * Express an inertial direction in the display frame.
 *
 * The same rotation the position and the attitude go through, so a direction
 * drawn at a spacecraft (Sun, nadir, …) can never follow a different convention
 * from the spacecraft it is drawn at. The input need not be a unit vector; the
 * rotation preserves its length either way.
 *
 * Only the frame's rotation is read, so a full {@link DisplayFrame} and its
 * {@link DisplayRotationFrame} projection give the same answer.
 *
 * The inertial frame returns the input array itself — the scene axes are the
 * inertial axes. Never mutates the input.
 */
export function displayDirection(frame: DisplayRotationFrame | DisplayFrame, inertial: Vec3): Vec3 {
  const [x, y, z] = inertial;
  switch (frame.kind) {
    case "bodyFixed": {
      // R_z(−ERA), the same rotation displayPosition applies to the coordinates.
      const c = Math.cos(frame.era);
      const s = Math.sin(frame.era);
      return [c * x + s * y, -s * x + c * y, z];
    }
    case "localOrbital": {
      // Project onto the [in-track, cross-track, radial] basis, exactly as the
      // dot products in transformToLvlh do for positions (minus the origin).
      const { inTrack, crossTrack, radial } = frame.axes;
      return [
        inTrack[0] * x + inTrack[1] * y + inTrack[2] * z,
        crossTrack[0] * x + crossTrack[1] * y + crossTrack[2] * z,
        radial[0] * x + radial[1] * y + radial[2] * z,
      ];
    }
    case "inertial":
      return inertial;
  }
}

/**
 * Express a body-to-inertial attitude quaternion in the display frame.
 *
 * `q_display = R_inertial→display ⊗ q_body→inertial`, with the rotation taken
 * from the same {@link DisplayFrame} the position uses. Returns undefined for an
 * undefined input so callers can pass an optional attitude straight through.
 *
 * The inertial frame returns the input array itself — the scene axes are the
 * inertial axes, and keeping the reference lets React skip re-applying an
 * unchanged quaternion. Never mutates the input.
 */
export function displayQuaternion(
  frame: DisplayFrame,
  bodyToInertial: Quat | undefined,
): Quat | undefined {
  if (bodyToInertial == null) return undefined;
  if (frame.kind === "inertial") return bodyToInertial;
  const [w, x, y, z] = bodyToInertial;
  const q = displayRotation(frame, scratchRotation).multiply(scratchAttitude.set(x, y, z, w));
  return [q.w, q.x, q.y, q.z];
}

/**
 * Identity of the transform a trail's vertices were *baked* with.
 *
 * The trail stores encoded vertices, so a change here invalidates everything
 * already written — unlike the origin/rotation uniforms, which are per-frame.
 */
export interface TrailTransformKey {
  orientation: FrameOrientation;
  center: FrameCenter;
  /**
   * Epoch the ECEF encoding was baked with, or null when the vertices are in
   * inertial axes. Carrying the epoch (not just a boolean) also catches an
   * epoch that arrives late or changes — a CSV load streams its points before
   * the `info` event that reveals the epoch.
   */
  ecefEpochJd: number | null;
}

/** Resolve the trail transform key for the current frame + epoch. */
export function trailTransformKey(
  referenceFrame: ReferenceFrame,
  epochJd: number | null | undefined,
): TrailTransformKey {
  return {
    orientation: referenceFrame.orientation,
    center: referenceFrame.center,
    ecefEpochJd: isLegacyEcef(referenceFrame) && epochJd != null ? epochJd : null,
  };
}

/** Whether the baked vertices must be re-encoded because the transform changed. */
export function needsFullRewrite(prev: TrailTransformKey, next: TrailTransformKey): boolean {
  return (
    prev.orientation !== next.orientation ||
    !frameCenterEquals(prev.center, next.center) ||
    prev.ecefEpochJd !== next.ecefEpochJd
  );
}
