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
export const POLE_ALIGNMENT_ROTATION: [number, number, number] = [
  Math.PI / 2,
  0,
  0,
];

/**
 * Default camera position for the display frame.
 * Placed on the +X side (vernal equinox direction), slightly above the equator.
 */
export const DEFAULT_CAMERA_POSITION: [number, number, number] = [5, 0, 2];
