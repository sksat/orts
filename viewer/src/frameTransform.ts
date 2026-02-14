/**
 * Coordinate frame types and transform functions.
 *
 * The simulation runs in ECI (Earth-Centered Inertial, J2000).
 * The viewer can display in ECI or ECEF (Earth-Centered Earth-Fixed).
 *
 * ECI → ECEF is a Z-axis rotation by -ERA (Earth Rotation Angle).
 */

/** Display coordinate frame for the 3D scene. */
export type DisplayFrame = "eci" | "ecef";

/**
 * Rotate a 3D vector around the Z axis.
 *
 * @param x  X component
 * @param y  Y component
 * @param z  Z component
 * @param angle  Rotation angle in radians (positive = counter-clockwise when viewed from +Z)
 * @returns  Rotated [x, y, z]
 */
export function rotateZ(
  x: number,
  y: number,
  z: number,
  angle: number,
): [number, number, number] {
  const c = Math.cos(angle);
  const s = Math.sin(angle);
  return [x * c - y * s, x * s + y * c, z];
}

/**
 * Transform a position from ECI to ECEF.
 *
 * Applies R_z(-ERA) to rotate from the inertial frame to the Earth-fixed frame.
 *
 * @param x    ECI X position
 * @param y    ECI Y position
 * @param z    ECI Z position
 * @param era  Earth Rotation Angle in radians
 * @returns    ECEF [x, y, z]
 */
export function eciToEcef(
  x: number,
  y: number,
  z: number,
  era: number,
): [number, number, number] {
  return rotateZ(x, y, z, -era);
}
