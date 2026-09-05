/**
 * Reference directions drawn at a spacecraft: where the Sun is, where the
 * central body is.
 *
 * Both leave in the display frame, the one the body they are drawn on is placed
 * in — an arrow following a different convention from that body would be worse
 * than no arrow. They reach here differently: the Sun already transformed, by the
 * lighting that computed it, and the position in the central-body inertial frame,
 * turned here by {@link displayDirection} — the same frame rotation the attitude
 * is turned by, which reaches it as a quaternion instead.
 *
 * A direction is dropped rather than guessed when its input is missing: no
 * epoch means no Sun (a fixed vernal-equinox arrow would read as a measurement),
 * no position means no nadir.
 */

import { centrePositionIsUsable } from "./frameResolve.js";
import {
  type DisplayFrame,
  type DisplayRotationFrame,
  displayDirection,
  type Vec3,
} from "./displayFrame.js";

/** Which reference direction an arrow shows. */
export type DirectionVectorKind = "sun" | "nadir";

/** Which reference directions to draw. */
export interface DirectionVectorOptions {
  sun?: boolean;
  nadir?: boolean;
}

/** Arrow colours, shared by the scene and by the legend that names them. */
export const DIRECTION_VECTOR_COLORS: Record<DirectionVectorKind, number> = {
  sun: 0xffd84d,
  nadir: 0x4dd2ff,
};

/** Arrow labels for the legend. */
export const DIRECTION_VECTOR_LABELS: Record<DirectionVectorKind, string> = {
  sun: "Sun",
  nadir: "Nadir",
};

/** One resolved arrow. */
export interface DirectionVector {
  kind: DirectionVectorKind;
  /** Unit vector in the display frame. */
  direction: Vec3;
  color: number;
}

export interface DirectionVectorInputs {
  /** The display frame the spacecraft itself is drawn in. */
  frame: DisplayRotationFrame | DisplayFrame;
  /**
   * Direction to the Sun in the display frame — the direction the lighting places
   * its light along. Null when the Sun is unknown rather than fixed: no epoch, or
   * a central body no ephemeris places.
   *
   * It arrives transformed while the position does not, so that the arrow and the
   * light cannot point differently: the vector is turned into the display frame
   * once, where it is computed, and both read that result (DESIGN.md). Turning it
   * again here would make their agreement a coincidence of two call sites
   * choosing the same frame.
   */
  sunDisplay?: Vec3 | null;
  /** Spacecraft position in the central-body inertial frame [km]. */
  positionEci?: Vec3 | null;
  /** Which arrows the view asks for. Both default to on. */
  options?: DirectionVectorOptions;
}

/**
 * Unit vector, or null for a zero-length or non-finite input.
 *
 * The length is checked for being finite as well as positive: components can each
 * be finite while their squares overflow, and dividing by an infinite length
 * yields a zero vector — which a rotation onto it turns into an invalid
 * quaternion rather than a dropped arrow.
 */
function normalize(v: Vec3 | null | undefined): Vec3 | null {
  if (v == null) return null;
  const [x, y, z] = v;
  const len = Math.sqrt(x * x + y * y + z * z);
  if (!(Number.isFinite(len) && len > 0)) return null;
  return [x / len, y / len, z / len];
}

/**
 * Resolve the arrows to draw, in display-frame axes.
 *
 * Every input is normalised, and the degenerate cases (zero, NaN, Infinity) are
 * rejected there, in one place, so no arrow can carry a non-finite direction into
 * the geometry. Only the position is then transformed; the Sun arrives in the
 * display frame already.
 */
export function resolveDirectionVectors({
  frame,
  sunDisplay = null,
  positionEci = null,
  options,
}: DirectionVectorInputs): DirectionVector[] {
  const vectors: DirectionVector[] = [];

  if (options?.sun !== false) {
    // Normalised, not transformed: the direction is already in the display frame.
    // The length is still checked, so a degenerate value cannot reach the
    // geometry as a rotation onto a zero vector.
    const sun = normalize(sunDisplay);
    if (sun != null) {
      vectors.push({ kind: "sun", direction: sun, color: DIRECTION_VECTOR_COLORS.sun });
    }
  }

  if (options?.nadir !== false) {
    // Nadir points from the spacecraft at the central body, so it is the
    // spacecraft's own radial direction reversed.
    const radial = normalize(positionEci);
    if (radial != null) {
      vectors.push({
        kind: "nadir",
        direction: displayDirection(frame, [-radial[0], -radial[1], -radial[2]]),
        color: DIRECTION_VECTOR_COLORS.nadir,
      });
    }
  }

  return vectors;
}

/**
 * Which arrows the orbit view would draw at a spacecraft it is centred on.
 *
 * Two conditions in order, because they fail differently. The scene draws nothing
 * at all at a centre it cannot place, and the Sun is why that has to be said
 * first: it needs no position of its own, so it would otherwise stay on offer
 * beside a spacecraft that is not on screen. Placing a centre asks
 * {@link centrePositionIsUsable} — the renderer's own condition, so this cannot
 * answer more strictly than the scene draws. A centre at the coordinate origin is
 * placeable: the spacecraft is drawn there and the Sun with it, and only nadir
 * drops out, having no bearing to take.
 *
 * Then the arrows themselves, from the resolver both scenes use, so a control
 * cannot offer one the scene goes on to drop. The frame decides where a direction
 * points rather than whether it resolves, so the inertial one stands in.
 */
export function drawableAtCentre(inputs: {
  positionEci: Vec3 | null | undefined;
  /** Whether the scene can compute a Sun direction at all — an epoch, and a body arika can place. */
  sunIsComputable: boolean;
}): readonly DirectionVectorKind[] {
  if (!centrePositionIsUsable(inputs.positionEci)) return [];
  return resolveDirectionVectors({
    frame: { kind: "inertial", origin: null },
    // A stand-in: only whether a Sun direction exists changes the answer, never
    // which way it points.
    sunDisplay: inputs.sunIsComputable ? [1, 0, 0] : null,
    positionEci: inputs.positionEci,
    options: { sun: true, nadir: true },
  }).map((v) => v.kind);
}
