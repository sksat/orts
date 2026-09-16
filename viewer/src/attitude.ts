/**
 * How the viewer reads the attitude a sample carries.
 *
 * This is the interpretation of a raw sample, which happens before anything is
 * placed in a display frame — `displayFrame.ts` converts what this resolves.
 * Not a general quaternion utility either: the tolerance and the refusal rules
 * are this viewer's policy about what it will draw.
 */

import type { Quat } from "./lib/types.js";

/**
 * How far a normalised quaternion may sit from the unit sphere.
 *
 * A few multiples of the machine epsilon: the division that normalises leaves
 * that much in the normal range, and far below the 1.3e-4 of the nearest case
 * it has to reject.
 */
const UNIT_QUATERNION_TOLERANCE = 1e-9;

/**
 * What a sample says about its orientation.
 *
 * Three states, because two facts have to stay apart: a sample that carries no
 * attitude — ordinary in the orbit view, where the marker is a position marker
 * — and one that claimed an orientation the viewer cannot use. The second has
 * consequences that must agree with each other: the registered model is not
 * drawn, the marker takes the shape that shows no orientation, and the
 * environment is amplified for whichever of the two is on screen.
 */
export type AttitudeState =
  /** No component arrived: the sample says nothing about orientation. */
  | { kind: "absent" }
  /** Components arrived but name no rotation: incomplete, zero, or non-finite. */
  | { kind: "refused" }
  /** A usable rotation, brought to unit norm. */
  | { kind: "usable"; quaternion: Quat };

const ABSENT: AttitudeState = { kind: "absent" };
const REFUSED: AttitudeState = { kind: "refused" };

/** One sample's loose quaternion components, as the wire and the files carry them. */
export interface AttitudeComponents {
  qw?: number | null;
  qx?: number | null;
  qy?: number | null;
  qz?: number | null;
}

/**
 * Read what a sample says about its orientation.
 *
 * One resolver rather than a pair of predicates: every consequence — the
 * rotation applied to the marker, the marker's shape, whether the registered
 * model is drawn, the amplification — reads the state this returns, so they
 * cannot disagree. Combining two predicates at each consumer is what let the
 * three states be reconstructed differently in different places.
 */
export function resolveAttitude(sample: AttitudeComponents): AttitudeState {
  const claimed = sample.qw != null || sample.qx != null || sample.qy != null || sample.qz != null;
  if (!claimed) return ABSENT;

  // All four components or none. Filling the missing ones with zero turns a
  // sample such as `{ qw: 0.5 }` into the identity once normalised, which draws
  // an orientation nobody supplied — and `hasQuaternion` in `orbit.ts`, which
  // decides whether two samples can be slerped, already requires the complete
  // tuple. A partial claim is a claim, so it is refused rather than absent:
  // calling it "no attitude at all" would draw the registered model at its own
  // orientation and amplify the scene for it.
  if (sample.qw == null || sample.qx == null || sample.qy == null || sample.qz == null) {
    return REFUSED;
  }

  const quaternion = unitAttitude([sample.qw, sample.qx, sample.qy, sample.qz]);
  return quaternion ? { kind: "usable", quaternion } : REFUSED;
}

/**
 * A caller's attitude as a unit quaternion, or undefined when it does not name
 * a rotation.
 */
export function unitAttitude(attitude: Quat | undefined): Quat | undefined {
  if (attitude == null) return undefined;
  let [w, x, y, z] = attitude;
  let n = Math.hypot(w, x, y, z);
  // `Math.hypot` scales before squaring, but its own result can still overflow:
  // the norm of `[MAX_VALUE, MAX_VALUE, 0, 0]` comes out Infinity — measured —
  // though it names a rotation as much as `[1e308, 1e308, 0, 0]`, whose norm is
  // finite. Retried against the largest component, which leaves every component
  // within [-1, 1] and the rotation unchanged, since scaling a quaternion
  // scales its norm and not the rotation it names.
  //
  // A retry rather than the first step, so the subnormal case keeps the answer
  // it has: scaling `[5e-324, 5e-324, 0, 0]` up would normalise it exactly,
  // where the policy below refuses it.
  if (n === Number.POSITIVE_INFINITY) {
    const largest = Math.max(Math.abs(w), Math.abs(x), Math.abs(y), Math.abs(z));
    if (!Number.isFinite(largest) || largest === 0) return undefined;
    [w, x, y, z] = [w / largest, x / largest, y / largest, z / largest];
    n = Math.hypot(w, x, y, z);
  }
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
