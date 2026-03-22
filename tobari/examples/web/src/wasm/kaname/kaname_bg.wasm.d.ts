/* tslint:disable */
/* eslint-disable */
export const memory: WebAssembly.Memory;
export const eci_to_ecef_batch: (a: number, b: number, c: number, d: number, e: number) => [number, number];
export const eci_to_ecef: (a: number, b: number, c: number, d: number, e: number) => [number, number];
export const earth_rotation_angle: (a: number, b: number) => number;
export const sun_direction_eci: (a: number, b: number) => [number, number];
export const sun_direction_from_body: (a: number, b: number, c: number, d: number) => [number, number];
export const sun_distance_from_body: (a: number, b: number, c: number, d: number) => number;
export const jd_to_utc_string: (a: number, b: number) => [number, number];
export const geodetic_to_ecef: (a: number, b: number, c: number) => [number, number];
export const geodetic_to_eci: (a: number, b: number, c: number, d: number) => [number, number];
export const __wbindgen_externrefs: WebAssembly.Table;
export const __wbindgen_free: (a: number, b: number, c: number) => void;
export const __wbindgen_malloc: (a: number, b: number) => number;
export const __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
export const __wbindgen_start: () => void;
