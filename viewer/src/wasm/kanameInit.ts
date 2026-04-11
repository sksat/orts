type EciToEcefBatch = (
  positions: Float32Array,
  times: Float32Array,
  epoch_jd: number,
) => Float32Array;

type EciToEcef = (x: number, y: number, z: number, epoch_jd: number, t: number) => Float32Array;

type EarthRotationAngle = (epoch_jd: number, t: number) => number;
type SunDirectionEci = (epoch_jd: number, t: number) => Float32Array;
type SunDirectionFromBody = (body: string, epoch_jd: number, t: number) => Float32Array;
type SunDistanceFromBody = (body: string, epoch_jd: number, t: number) => number;
type JdToUtcString = (epoch_jd: number, t: number) => string;
type BodyOrientation = (body: string, epoch_jd: number, t: number) => Float64Array;
type BodyQuatToRsw = (
  pos_x: number,
  pos_y: number,
  pos_z: number,
  vel_x: number,
  vel_y: number,
  vel_z: number,
  qw: number,
  qx: number,
  qy: number,
  qz: number,
) => Float64Array;

let initialized = false;
let initPromise: Promise<void> | undefined;
let wasmBatch: EciToEcefBatch | undefined;
let wasmSingle: EciToEcef | undefined;
let wasmEra: EarthRotationAngle | undefined;
let wasmSunDir: SunDirectionEci | undefined;
let wasmSunDirFromBody: SunDirectionFromBody | undefined;
let wasmSunDistFromBody: SunDistanceFromBody | undefined;
let wasmJdToUtc: JdToUtcString | undefined;
let wasmBodyOrientation: BodyOrientation | undefined;
let wasmBodyQuatToRsw: BodyQuatToRsw | undefined;

/** Initialize the kaname WASM module. Safe to call multiple times. Rejects on failure. */
export function initKaname(): Promise<void> {
  if (initialized) return Promise.resolve();
  if (initPromise) return initPromise;

  const p: Promise<void> = import("./kaname/kaname.js").then(async (mod) => {
    await mod.default();
    wasmBatch = mod.eci_to_ecef_batch;
    wasmSingle = mod.eci_to_ecef;
    wasmEra = mod.earth_rotation_angle;
    wasmSunDir = mod.sun_direction_eci;
    wasmSunDirFromBody = mod.sun_direction_from_body;
    wasmSunDistFromBody = mod.sun_distance_from_body;
    wasmJdToUtc = mod.jd_to_utc_string;
    wasmBodyOrientation = mod.body_orientation;
    wasmBodyQuatToRsw = mod.body_quat_to_rsw;
    initialized = true;
  });
  initPromise = p;
  return p;
}

/** Whether the WASM module is loaded and ready. */
export function isKanameReady(): boolean {
  return initialized;
}

/** Batch ECI→ECEF transform via WASM. */
export function eci_to_ecef_batch(
  positions: Float32Array,
  times: Float32Array,
  epoch_jd: number,
): Float32Array {
  return wasmBatch!(positions, times, epoch_jd);
}

/** Single-point ECI→ECEF transform via WASM. Returns [ex, ey, ez]. */
export function eci_to_ecef(
  x: number,
  y: number,
  z: number,
  epoch_jd: number,
  t: number,
): Float32Array {
  return wasmSingle!(x, y, z, epoch_jd, t);
}

/** Compute Earth Rotation Angle (GMST) in radians via WASM. */
export function earth_rotation_angle(epoch_jd: number, t: number): number {
  return wasmEra!(epoch_jd, t);
}

/** Approximate sun direction (unit vector) in ECI frame via WASM. Returns [x, y, z]. */
export function sun_direction_eci(epoch_jd: number, t: number): Float32Array {
  return wasmSunDir!(epoch_jd, t);
}

/** Sun direction (unit vector) as seen from a given body, in J2000 equatorial frame via WASM. Returns [x, y, z]. */
export function sun_direction_from_body(body: string, epoch_jd: number, t: number): Float32Array {
  return wasmSunDirFromBody!(body, epoch_jd, t);
}

/** Sun distance [km] from a given body via WASM. */
export function sun_distance_from_body(body: string, epoch_jd: number, t: number): number {
  return wasmSunDistFromBody!(body, epoch_jd, t);
}

/** Convert Julian Date + elapsed sim time to ISO 8601 UTC string via WASM. */
export function jd_to_utc_string(epoch_jd: number, t: number): string {
  return wasmJdToUtc!(epoch_jd, t);
}

/**
 * Body-fixed → ECI orientation quaternion via IAU rotation model.
 *
 * Returns [w, x, y, z] (Hamilton scalar-first) or undefined for unknown bodies.
 */
export function body_orientation(
  body: string,
  epoch_jd: number,
  t: number,
): [number, number, number, number] | undefined {
  const result = wasmBodyOrientation!(body, epoch_jd, t);
  if (result.length === 0) return undefined;
  return [result[0], result[1], result[2], result[3]];
}

/**
 * Transform body-to-ECI quaternion to body-to-RSW frame via WASM.
 *
 * RSW axis order: [Radial, Along-track, Cross-track] (standard Vallado).
 *
 * Returns [w, x, y, z] (Hamilton scalar-first) or undefined if degenerate.
 */
export function body_quat_to_rsw(
  pos_x: number,
  pos_y: number,
  pos_z: number,
  vel_x: number,
  vel_y: number,
  vel_z: number,
  qw: number,
  qx: number,
  qy: number,
  qz: number,
): [number, number, number, number] | undefined {
  const result = wasmBodyQuatToRsw!(pos_x, pos_y, pos_z, vel_x, vel_y, vel_z, qw, qx, qy, qz);
  if (result.length === 0) return undefined;
  return [result[0], result[1], result[2], result[3]];
}
