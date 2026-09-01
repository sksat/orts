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
type OrbitDerivedBatch = (states: Float64Array, mu: number, body_radius: number) => Float64Array;

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
let wasmOrbitDerived: OrbitDerivedBatch | undefined;

/** Options for {@link initArika}. */
export interface InitArikaOptions {
  /**
   * Where to load the arika `.wasm` from. Omit to let the bundler resolve it
   * from the `arika-wasm` package (the usual case). Supply a URL to fetch it
   * from elsewhere — e.g. a CDN, or when a bundler can't resolve the asset.
   * Only the first init call's options take effect.
   */
  wasmUrl?: string | URL;
}

/**
 * Initialize the arika WASM module. Safe to call multiple times (idempotent;
 * the first call wins). Rejects on failure.
 *
 * `OrbitViewer` calls this for you when given an `epochJd`; call it explicitly
 * only to pre-load, or to override where the wasm is fetched from via
 * {@link InitArikaOptions.wasmUrl}.
 */
export function initArika(options?: InitArikaOptions): Promise<void> {
  if (initialized) return Promise.resolve();
  if (initPromise) return initPromise;

  const p: Promise<void> = import("arika-wasm").then(async (mod) => {
    await mod.default(options?.wasmUrl);
    wasmBatch = mod.eci_to_ecef_batch;
    wasmSingle = mod.eci_to_ecef;
    wasmEra = mod.earth_rotation_angle;
    wasmSunDir = mod.sun_direction_eci;
    wasmSunDirFromBody = mod.sun_direction_from_body;
    wasmSunDistFromBody = mod.sun_distance_from_body;
    wasmJdToUtc = mod.jd_to_utc_string;
    wasmBodyOrientation = mod.body_orientation;
    wasmOrbitDerived = mod.orbit_derived_batch;
    initialized = true;
  });
  initPromise = p;
  return p;
}

/** Whether the WASM module is loaded and ready. */
export function isArikaReady(): boolean {
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

/**
 * Keplerian elements and chart scalars for a batch of state vectors, via WASM.
 *
 * `states` is `[x,y,z,vx,vy,vz, ...]`; the result is 10 values per state,
 * `[a, e, inc, raan, omega, nu, altitude, specific_energy, angular_momentum,
 * velocity]`, with angles in radians. A state with no orbital plane comes back
 * as ten `NaN`s.
 */
export function orbit_derived_batch(
  states: Float64Array,
  mu: number,
  body_radius: number,
): Float64Array {
  return wasmOrbitDerived!(states, mu, body_radius);
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
