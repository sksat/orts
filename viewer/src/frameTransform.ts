/**
 * Coordinate frame types and general rotation utilities.
 *
 * ECI→ECEF coordinate transforms are handled by the kaname WASM module.
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
