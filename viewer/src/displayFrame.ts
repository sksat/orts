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
 * Resolve the display frame for one sample. The branch order matches the
 * frame semantics: a body-fixed central-body view rotates, a satellite-centred
 * local-orbital view re-bases, anything else only shifts the origin.
 */
export function resolveDisplayFrame(
  referenceFrame: ReferenceFrame,
  { era = null, originPosition = null, lvlhAxes = null }: DisplayFrameInputs,
): DisplayFrame {
  if (isLegacyEcef(referenceFrame) && era != null) {
    return { kind: "bodyFixed", era };
  }
  if (originPosition != null && lvlhAxes != null) {
    return { kind: "localOrbital", origin: originPosition, axes: lvlhAxes };
  }
  return { kind: "inertial", origin: originPosition };
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
