/* tslint:disable */
/* eslint-disable */

/**
 * Body-fixed → ECI orientation quaternion using the IAU rotation model.
 *
 * `body`: body identifier string (e.g., "moon", "mars", "sun")
 * `epoch_jd`: Julian Date of the simulation epoch
 * `t`: elapsed simulation time in seconds
 *
 * Returns `[w, x, y, z]` quaternion (4 f64 values, Hamilton scalar-first).
 * Returns an empty vec if the body has no IAU rotation model.
 */
export function body_orientation(body: string, epoch_jd: number, t: number): Float64Array;

/**
 * Transform a body-to-ECI quaternion into a body-to-RSW quaternion.
 *
 * `pos_x/y/z`: satellite position in ECI \[km\]
 * `vel_x/y/z`: satellite velocity in ECI \[km/s\]
 * `qw/qx/qy/qz`: body-to-ECI quaternion (Hamilton scalar-first: w,x,y,z)
 *
 * Returns `[w, x, y, z]` body-to-RSW quaternion (4 floats, f64).
 * Returns an empty vec if the RSW frame cannot be computed (degenerate orbit).
 */
export function body_quat_to_rsw(pos_x: number, pos_y: number, pos_z: number, vel_x: number, vel_y: number, vel_z: number, qw: number, qx: number, qy: number, qz: number): Float64Array;

/**
 * Compute the Earth Rotation Angle (ERA, historically called GMST) in radians.
 *
 * `epoch_jd`: Julian Date of the simulation epoch
 * `t`: elapsed simulation time in seconds
 */
export function earth_rotation_angle(epoch_jd: number, t: number): number;

/**
 * Single-point ECI→ECEF transform.
 *
 * Returns flat ECEF `[ex, ey, ez]` (3 floats, km).
 */
export function eci_to_ecef(x: number, y: number, z: number, epoch_jd: number, t: number): Float32Array;

/**
 * Batch ECI→ECEF transform with per-point time.
 *
 * `positions`: flat `[x0,y0,z0, x1,y1,z1, ...]` (length = N×3, km)
 * `times`: `[t0, t1, ...]` (length = N, simulation elapsed seconds)
 * `epoch_jd`: Julian Date of the simulation epoch
 *
 * Returns flat ECEF `[ex0,ey0,ez0, ...]` (length = N×3, km).
 *
 * For each point, computes ERA from `epoch_jd + t` and applies the
 * Z-axis rotation (SimpleEci → SimpleEcef).
 */
export function eci_to_ecef_batch(positions: Float32Array, times: Float32Array, epoch_jd: number): Float32Array;

/**
 * Geodetic (lat_deg, lon_deg, altitude_km) → SimpleEcef [km].
 *
 * Returns `[x, y, z]` (3 floats, km).
 */
export function geodetic_to_ecef(lat_deg: number, lon_deg: number, altitude_km: number): Float64Array;

/**
 * Geodetic (lat_deg, lon_deg, altitude_km) → SimpleEci [km] at given epoch.
 *
 * Returns `[x, y, z]` (3 floats, km).
 */
export function geodetic_to_eci(lat_deg: number, lon_deg: number, altitude_km: number, epoch_jd: number): Float64Array;

/**
 * Convert Julian Date + elapsed sim time to a UTC date/time string.
 *
 * Returns ISO 8601 string like "2024-03-20T12:00:00Z".
 */
export function jd_to_utc_string(epoch_jd: number, t: number): string;

/**
 * Approximate sun direction (unit vector) in Gcrs frame.
 *
 * Returns `[x, y, z]` (3 floats).
 */
export function sun_direction_eci(epoch_jd: number, t: number): Float32Array;

/**
 * Sun direction (unit vector) as seen from a given central body, in J2000 equatorial frame.
 *
 * Returns `[x, y, z]` (3 floats).
 * `body`: body identifier string (e.g., "earth", "mars")
 * `epoch_jd`: Julian Date of the simulation epoch
 * `t`: elapsed simulation time in seconds
 */
export function sun_direction_from_body(body: string, epoch_jd: number, t: number): Float32Array;

/**
 * Sun distance [km] from a given central body.
 *
 * `body`: body identifier string (e.g., "earth", "mars")
 * `epoch_jd`: Julian Date of the simulation epoch
 * `t`: elapsed simulation time in seconds
 */
export function sun_distance_from_body(body: string, epoch_jd: number, t: number): number;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly body_orientation: (a: number, b: number, c: number, d: number) => [number, number];
    readonly body_quat_to_rsw: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => [number, number];
    readonly earth_rotation_angle: (a: number, b: number) => number;
    readonly eci_to_ecef: (a: number, b: number, c: number, d: number, e: number) => [number, number];
    readonly eci_to_ecef_batch: (a: number, b: number, c: number, d: number, e: number) => [number, number];
    readonly geodetic_to_ecef: (a: number, b: number, c: number) => [number, number];
    readonly geodetic_to_eci: (a: number, b: number, c: number, d: number) => [number, number];
    readonly jd_to_utc_string: (a: number, b: number) => [number, number];
    readonly sun_direction_eci: (a: number, b: number) => [number, number];
    readonly sun_direction_from_body: (a: number, b: number, c: number, d: number) => [number, number];
    readonly sun_distance_from_body: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
