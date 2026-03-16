/**
 * Scene coordinate frame configuration.
 *
 * Centralizes the mapping between the physical coordinate system (e.g. ECI)
 * and the Three.js scene. To support a different display frame in the future,
 * change these values (or make them selectable at runtime).
 *
 * Current frame: ECI (Earth-Centered Inertial, J2000)
 *   - X: vernal equinox
 *   - Y: 90° in equatorial plane
 *   - Z: north pole (up)
 */

/**
 * The "up" direction in the display frame, used as the camera and scene up vector.
 * ECI: Z = north pole.
 */
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
 * "below" in the viewport. Returns SCENE_UP for non-satellite-centered views.
 */
export function computeCameraUp(
  originPosition: [number, number, number] | null,
): [number, number, number] {
  if (originPosition == null) return SCENE_UP;
  const [x, y, z] = originPosition;
  const len = Math.sqrt(x * x + y * y + z * z);
  if (len < 1e-10) return SCENE_UP;
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
 * Returns null if position or velocity is null/zero.
 */
export function computeLvlhAxes(
  position: [number, number, number] | null,
  velocity: [number, number, number] | null,
): LvlhAxes | null {
  if (position == null || velocity == null) return null;

  const [rx, ry, rz] = position;
  const rLen = Math.sqrt(rx * rx + ry * ry + rz * rz);
  if (rLen < 1e-10) return null;

  const [vx, vy, vz] = velocity;
  const vLen = Math.sqrt(vx * vx + vy * vy + vz * vz);
  if (vLen < 1e-10) return null;

  // Radial = normalize(r)
  const radial: [number, number, number] = [rx / rLen, ry / rLen, rz / rLen];

  // Cross-track = normalize(r × v)
  const cx = ry * vz - rz * vy;
  const cy = rz * vx - rx * vz;
  const cz = rx * vy - ry * vx;
  const cLen = Math.sqrt(cx * cx + cy * cy + cz * cz);
  if (cLen < 1e-10) return null; // degenerate: r parallel to v
  const crossTrack: [number, number, number] = [cx / cLen, cy / cLen, cz / cLen];

  // In-track = crossTrack × radial
  const inTrack: [number, number, number] = [
    crossTrack[1] * radial[2] - crossTrack[2] * radial[1],
    crossTrack[2] * radial[0] - crossTrack[0] * radial[2],
    crossTrack[0] * radial[1] - crossTrack[1] * radial[0],
  ];

  return { radial, inTrack, crossTrack };
}
