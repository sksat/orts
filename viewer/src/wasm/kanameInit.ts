type EciToEcefBatch = (
  positions: Float32Array,
  times: Float32Array,
  epoch_jd: number,
) => Float32Array;

type EciToEcef = (
  x: number,
  y: number,
  z: number,
  epoch_jd: number,
  t: number,
) => Float32Array;

type EarthRotationAngle = (epoch_jd: number, t: number) => number;
type SunDirectionEci = (epoch_jd: number, t: number) => Float32Array;
type SunDirectionFromBody = (body: string, epoch_jd: number, t: number) => Float32Array;
type SunDistanceFromBody = (body: string, epoch_jd: number, t: number) => number;
type JdToUtcString = (epoch_jd: number, t: number) => string;

let initialized = false;
let initPromise: Promise<void> | undefined;
let wasmBatch: EciToEcefBatch | undefined;
let wasmSingle: EciToEcef | undefined;
let wasmEra: EarthRotationAngle | undefined;
let wasmSunDir: SunDirectionEci | undefined;
let wasmSunDirFromBody: SunDirectionFromBody | undefined;
let wasmSunDistFromBody: SunDistanceFromBody | undefined;
let wasmJdToUtc: JdToUtcString | undefined;

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
