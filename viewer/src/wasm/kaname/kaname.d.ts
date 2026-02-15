/* tslint:disable */
/* eslint-disable */

/**
 * Compute the Earth Rotation Angle (GMST) in radians.
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
 * Z-axis rotation (ECI→ECEF).
 */
export function eci_to_ecef_batch(positions: Float32Array, times: Float32Array, epoch_jd: number): Float32Array;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly eci_to_ecef_batch: (a: number, b: number, c: number, d: number, e: number) => [number, number];
    readonly eci_to_ecef: (a: number, b: number, c: number, d: number, e: number) => [number, number];
    readonly earth_rotation_angle: (a: number, b: number) => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
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
