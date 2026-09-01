/**
 * Reference directions drawn at a spacecraft: where the Sun is, where the
 * central body is.
 *
 * Both arrive in the central-body inertial frame and leave in the display frame,
 * through the same {@link displayDirection} the position and the attitude go
 * through — an arrow that followed a different convention from the body it is
 * drawn on would be worse than no arrow.
 *
 * A direction is dropped rather than guessed when its input is missing: no
 * epoch means no Sun (a fixed vernal-equinox arrow would read as a measurement),
 * no position means no nadir.
 */

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
   * Sun direction in the central-body inertial frame. Null when no epoch is
   * known, which is what makes the Sun unknown rather than fixed.
   */
  sunEci?: Vec3 | null;
  /** Spacecraft position in the central-body inertial frame [km]. */
  positionEci?: Vec3 | null;
  /** Which arrows the view asks for. Both default to on. */
  options?: DirectionVectorOptions;
}

/** Unit vector, or null for a zero-length or non-finite input. */
function normalize(v: Vec3 | null | undefined): Vec3 | null {
  if (v == null) return null;
  const [x, y, z] = v;
  if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(z)) return null;
  const len = Math.sqrt(x * x + y * y + z * z);
  if (!(len > 0)) return null;
  return [x / len, y / len, z / len];
}

/**
 * Resolve the arrows to draw, in display-frame axes.
 *
 * Normalisation happens before the frame transform so the degenerate cases
 * (zero, NaN, Infinity) are rejected in one place and no arrow can carry a
 * non-finite direction into the geometry.
 */
export function resolveDirectionVectors({
  frame,
  sunEci = null,
  positionEci = null,
  options,
}: DirectionVectorInputs): DirectionVector[] {
  const vectors: DirectionVector[] = [];

  if (options?.sun !== false) {
    const sun = normalize(sunEci);
    if (sun != null) {
      vectors.push({
        kind: "sun",
        direction: displayDirection(frame, sun),
        color: DIRECTION_VECTOR_COLORS.sun,
      });
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
