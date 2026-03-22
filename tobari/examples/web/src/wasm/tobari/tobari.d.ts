/* tslint:disable */
/* eslint-disable */

/**
 * Compute altitude profile for all 3 atmosphere models.
 *
 * Returns flat `[exp_0, hp_0, msis_0, exp_1, hp_1, msis_1, ...]` (length = N×3).
 */
export function atmosphere_altitude_profile(altitudes: Float64Array, lat_deg: number, lon_deg: number, epoch_jd: number, f107: number, ap: number): Float64Array;

/**
 * Compute lat/lon density map for a chosen atmosphere model.
 *
 * `model`: `"exponential"`, `"harris-priester"`, or `"nrlmsise00"`.
 * Returns flat row-major `[rho_0, rho_1, ...]` (length = n_lat × n_lon).
 * Latitude ranges from -90 to +90, longitude from -180 to +180.
 */
export function atmosphere_latlon_map(model: string, altitude_km: number, epoch_jd: number, n_lat: number, n_lon: number, f107: number, ap: number): Float64Array;

/**
 * Compute lat/lon density map using loaded space weather data.
 *
 * Like `atmosphere_latlon_map` but uses the loaded CSSI/GFZ data
 * instead of constant F10.7/Ap values.
 * Returns empty vec if no space weather data is loaded.
 */
export function atmosphere_latlon_map_sw(model: string, altitude_km: number, epoch_jd: number, n_lat: number, n_lon: number): Float64Array;

/**
 * Compute 3D atmospheric density volume as Float32.
 *
 * Layout: alt-major `index = iAlt * nLat * nLon + iLat * nLon + iLon`
 * Returns `[rho_0, rho_1, ...]` (length = n_alt × n_lat × n_lon).
 * Also returns `[min, max]` appended at the end (total length = n_alt*n_lat*n_lon + 2).
 */
export function atmosphere_volume(model: string, alt_min_km: number, alt_max_km: number, n_alt: number, epoch_jd: number, n_lat: number, n_lon: number, f107: number, ap: number): Float32Array;

/**
 * Compute 3D atmosphere volume using loaded space weather data.
 */
export function atmosphere_volume_sw(model: string, alt_min_km: number, alt_max_km: number, n_alt: number, epoch_jd: number, n_lat: number, n_lon: number): Float32Array;

/**
 * Tilted dipole field at a geodetic point.
 *
 * Returns `[B_north, B_east, B_down, |B|, inclination_deg, declination_deg]` in nT.
 */
export function dipole_field_at(lat_deg: number, lon_deg: number, altitude_km: number, epoch_jd: number): Float64Array;

/**
 * Exponential atmosphere density [kg/m³] at the given altitude.
 */
export function exponential_density(altitude_km: number): number;

/**
 * Harris-Priester density [kg/m³] at a geodetic point and epoch.
 *
 * `epoch_jd`: Julian Date of the epoch.
 */
export function harris_priester_density(lat_deg: number, lon_deg: number, altitude_km: number, epoch_jd: number): number;

/**
 * IGRF-14 field at a geodetic point.
 *
 * Returns `[B_north, B_east, B_down, |B|, inclination_deg, declination_deg]` in nT.
 */
export function igrf_field_at(lat_deg: number, lon_deg: number, altitude_km: number, epoch_jd: number): Float64Array;

/**
 * Load space weather data from text (CSSI or GFZ format, auto-detected).
 *
 * Returns `true` on success. Can only be called once; subsequent calls
 * return `false` without replacing the existing data.
 */
export function load_space_weather(text: string): boolean;

/**
 * Compute lat/lon magnetic field map.
 *
 * `model`: `"igrf"` or `"dipole"`.
 * `component`: `"total"`, `"inclination"`, `"declination"`, `"north"`, `"east"`, `"down"`.
 * Returns flat row-major values (length = n_lat × n_lon).
 * Values in nT for field components, degrees for angles.
 */
export function magnetic_field_latlon_map(model: string, component: string, altitude_km: number, epoch_jd: number, n_lat: number, n_lon: number): Float64Array;

/**
 * Integrate magnetic field lines from seed points using RK4.
 *
 * `seed_lats`, `seed_lons`: geodetic seed points (degrees).
 * `seed_alt_km`: starting altitude for all seeds.
 * `model`: `"igrf"` or `"dipole"`.
 * `max_steps`: max integration steps per line.
 * `step_km`: step size in km.
 *
 * Returns flat `[n_lines, n_pts_0, x0,y0,z0, x1,y1,z1, ..., n_pts_1, ...]`
 * where coordinates are in Earth radii (6371 km).
 */
export function magnetic_field_lines(seed_lats: Float64Array, seed_lons: Float64Array, seed_alt_km: number, epoch_jd: number, model: string, max_steps: number, step_km: number): Float32Array;

/**
 * NRLMSISE-00 density [kg/m³] at a geodetic point with constant space weather.
 *
 * `f107`: F10.7 solar radio flux [SFU].
 * `ap`: daily Ap geomagnetic index.
 */
export function nrlmsise00_density(lat_deg: number, lon_deg: number, altitude_km: number, epoch_jd: number, f107: number, ap: number): number;

/**
 * Get date range of the loaded space weather data.
 *
 * Returns `[jd_first, jd_last]` or empty vec if no data loaded.
 */
export function space_weather_date_range(): Float64Array;

/**
 * Look up space weather for an epoch from the loaded dataset.
 *
 * Returns `[f107_daily, f107_avg, ap_daily, ap_3h_0..6]` (length = 10).
 * Returns empty vec if no data is loaded.
 */
export function space_weather_lookup(epoch_jd: number): Float64Array;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly atmosphere_altitude_profile: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => [number, number];
    readonly atmosphere_latlon_map: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => [number, number];
    readonly igrf_field_at: (a: number, b: number, c: number, d: number) => [number, number];
    readonly dipole_field_at: (a: number, b: number, c: number, d: number) => [number, number];
    readonly magnetic_field_latlon_map: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => [number, number];
    readonly atmosphere_volume: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => [number, number];
    readonly magnetic_field_lines: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => [number, number];
    readonly load_space_weather: (a: number, b: number) => number;
    readonly space_weather_lookup: (a: number) => [number, number];
    readonly space_weather_date_range: () => [number, number];
    readonly atmosphere_latlon_map_sw: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number];
    readonly atmosphere_volume_sw: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => [number, number];
    readonly nrlmsise00_density: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly exponential_density: (a: number) => number;
    readonly harris_priester_density: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
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
