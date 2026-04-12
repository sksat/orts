/* tslint:disable */
/* eslint-disable */
export const memory: WebAssembly.Memory;
export const atmosphere_altitude_profile: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => [number, number];
export const atmosphere_latlon_map: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => [number, number];
export const atmosphere_latlon_map_sw: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number];
export const atmosphere_volume: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => [number, number];
export const atmosphere_volume_sw: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => [number, number];
export const dipole_field_at: (a: number, b: number, c: number, d: number) => [number, number];
export const igrf_field_at: (a: number, b: number, c: number, d: number) => [number, number];
export const load_space_weather: (a: number, b: number) => number;
export const magnetic_field_latlon_map: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => [number, number];
export const magnetic_field_lines: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => [number, number];
export const magnetic_field_volume: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => [number, number];
export const space_weather_date_range: () => [number, number];
export const space_weather_lookup: (a: number) => [number, number];
export const space_weather_series: () => [number, number];
export const nrlmsise00_density: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
export const exponential_density: (a: number) => number;
export const harris_priester_density: (a: number, b: number, c: number, d: number) => number;
export const __wbindgen_externrefs: WebAssembly.Table;
export const __wbindgen_malloc: (a: number, b: number) => number;
export const __wbindgen_free: (a: number, b: number, c: number) => void;
export const __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
export const __wbindgen_start: () => void;
