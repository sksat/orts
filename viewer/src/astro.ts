/**
 * Low-precision astronomical computations for visualization.
 *
 * Earth Rotation Angle (ERA) and ECI→ECEF transforms are handled by the
 * kaname WASM module. This file retains time formatting and sun direction.
 */

const J2000_JD = 2451545.0;
const UNIX_EPOCH_JD = 2440587.5;
const DEG_TO_RAD = Math.PI / 180;

/**
 * Convert Julian Date + elapsed sim time to a UTC date/time string.
 *
 * @returns  ISO 8601 string like "2024-03-20T12:34:56Z"
 */
export function jdToUTCString(epochJd: number, simTimeSec: number): string {
  const jd = epochJd + simTimeSec / 86400;
  const unixMs = Math.round((jd - UNIX_EPOCH_JD) * 86400 * 1000);
  const d = new Date(unixMs);
  return d.toISOString().replace(/\.\d{3}Z$/, "Z");
}

/**
 * Compute the approximate sun direction (unit vector) in ECI (J2000) frame.
 *
 * Uses a low-precision analytical model (~1 arcminute accuracy).
 *
 * @param epochJd   Julian Date of the simulation start epoch
 * @param simTimeSec  Elapsed simulation time in seconds
 * @returns  Normalized [x, y, z] sun direction in ECI
 */
export function sunDirectionECI(
  epochJd: number,
  simTimeSec: number,
): [number, number, number] {
  const jd = epochJd + simTimeSec / 86400;
  const t = (jd - J2000_JD) / 36525; // Julian centuries since J2000

  // Mean longitude (degrees)
  const l0 = 280.46646 + 36000.76983 * t;
  // Mean anomaly (degrees → radians)
  const mDeg = 357.52911 + 35999.05029 * t;
  const m = mDeg * DEG_TO_RAD;

  // Equation of center (degrees)
  const c =
    (1.9146 - 0.004817 * t) * Math.sin(m) +
    0.019993 * Math.sin(2 * m);

  // Sun's ecliptic longitude (radians)
  const lambda = (l0 + c) * DEG_TO_RAD;

  // Obliquity of the ecliptic (radians)
  const epsilon = (23.439291 - 0.0130042 * t) * DEG_TO_RAD;

  // Sun direction in ECI (equatorial coordinates)
  const x = Math.cos(lambda);
  const y = Math.cos(epsilon) * Math.sin(lambda);
  const z = Math.sin(epsilon) * Math.sin(lambda);

  // Normalize (should already be ~unit, but ensure)
  const norm = Math.sqrt(x * x + y * y + z * z);
  return [x / norm, y / norm, z / norm];
}
