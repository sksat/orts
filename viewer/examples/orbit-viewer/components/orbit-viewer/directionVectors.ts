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
 * Whether a scene centred on this spacecraft can place it at all.
 *
 * The orbit view drops every arrow at a centre whose position its frame cannot
 * use, so a control that offers one there offers an arrow the scene then drops.
 * The Sun is the case that needs saying: it has no position of its own, and would
 * otherwise stay on offer beside a spacecraft that is not on screen.
 *
 * Asked of the resolver rather than derived from "a position is present": a
 * position can be there and still yield no direction — zero, or non-finite from a
 * file source. Nadir is the arrow that needs the position, so whether it resolves
 * is the whole question, and this takes no Sun input so the answer cannot come
 * from one. The frame decides where a direction points, not whether it resolves,
 * so the inertial one stands in.
 */
export function centreIsPlaceable(positionEci: Vec3 | null | undefined): boolean {
  return (
    resolveDirectionVectors({
      frame: { kind: "inertial", origin: null },
      positionEci,
      options: { nadir: true },
    }).length > 0
  );
}
