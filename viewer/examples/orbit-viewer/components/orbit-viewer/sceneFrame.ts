export const SCENE_UP: [number, number, number] = [0, 0, 1];

/**
 * Euler rotation [rx, ry, rz] that aligns a Three.js sphere (Y-pole)
 * with the display frame's polar axis.
 *
 * ECI: +π/2 around X maps local +Y → world +Z (north pole).
 */
export const POLE_ALIGNMENT_ROTATION: [number, number, number] = [Math.PI / 2, 0, 0];

/**
 * Default camera position for the display frame.
 * Placed on the +X side (vernal equinox direction), slightly above the equator.
 */
export const DEFAULT_CAMERA_POSITION: [number, number, number] = [5, 0, 2];

/**
 * Compute the camera "up" direction for satellite-centered view.
 *
 * When centered on a satellite, the up vector is the radial outward direction
 * (from central body through satellite), so the central body always appears
 * "below" in the viewport. Returns SCENE_UP for non-satellite-centered views, and
 * for a zero or non-finite position — a NaN `camera.up` blanks the whole canvas,
 * so a fallback that is merely wrong beats one that is not a number.
 */
export function computeCameraUp(
  originPosition: [number, number, number] | null,
): [number, number, number] {
  if (originPosition == null) return SCENE_UP;
  const [x, y, z] = originPosition;
  const len = Math.sqrt(x * x + y * y + z * z);
  if (!(Number.isFinite(len) && len > 1e-10)) return SCENE_UP;
  return [x / len, y / len, z / len];
}

/** LVLH (Local Vertical Local Horizontal) axes as unit vectors. */
export interface LvlhAxes {
  /** Radial outward (from central body through satellite). */
  radial: [number, number, number];
  /** In-track (roughly along velocity, in the orbit plane). */
  inTrack: [number, number, number];
  /** Cross-track (orbit normal, completes right-handed triad: C × R = I). */
  crossTrack: [number, number, number];
}

/**
 * Compute the LVLH frame axes from satellite position and velocity.
 *
 * - Radial (R) = normalize(r)
 * - Cross-track (C) = normalize(r × v)  (orbit normal)
 * - In-track (I) = C × R  (in orbit plane, roughly along velocity)
 *
 * Returns null when the axes cannot be built: a null, zero or non-finite
 * position or velocity, or a position parallel to the velocity. A comparison
 * against a threshold is false for NaN, so each length is checked for being
 * finite as well — otherwise a non-finite input passes the degeneracy test and
 * leaves NaN in every axis, and a caller that treats non-null axes as "the
 * local-orbital frame is available" then draws the whole scene at NaN.
 */
export function computeLvlhAxes(
  position: [number, number, number] | null,
  velocity: [number, number, number] | null,
): LvlhAxes | null {
  if (position == null || velocity == null) return null;

  const [rx, ry, rz] = position;
  const rLen = Math.sqrt(rx * rx + ry * ry + rz * rz);
  if (!(Number.isFinite(rLen) && rLen > 1e-10)) return null;

  const [vx, vy, vz] = velocity;
  const vLen = Math.sqrt(vx * vx + vy * vy + vz * vz);
  if (!(Number.isFinite(vLen) && vLen > 1e-10)) return null;

  // Radial = normalize(r)
  const radial: [number, number, number] = [rx / rLen, ry / rLen, rz / rLen];

  // Cross-track = normalize(r × v)
  const cx = ry * vz - rz * vy;
  const cy = rz * vx - rx * vz;
  const cz = rx * vy - ry * vx;
  const cLen = Math.sqrt(cx * cx + cy * cy + cz * cz);
  // Degenerate (r parallel to v), or non-finite from a large-magnitude input.
  if (!(Number.isFinite(cLen) && cLen > 1e-10)) return null;
  const crossTrack: [number, number, number] = [cx / cLen, cy / cLen, cz / cLen];

  // In-track = crossTrack × radial
  const inTrack: [number, number, number] = [
    crossTrack[1] * radial[2] - crossTrack[2] * radial[1],
    crossTrack[2] * radial[0] - crossTrack[0] * radial[2],
    crossTrack[0] * radial[1] - crossTrack[1] * radial[0],
  ];

  return { radial, inTrack, crossTrack };
}
