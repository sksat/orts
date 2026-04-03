/**
 * RRD WASM module loader.
 *
 * Lazily loads the rrd-wasm WASM module and exposes a typed parse function.
 * Follows the same pattern as kanameInit.ts.
 */

export interface RrdMetadata {
  epoch_jd: number | null;
  mu: number | null;
  body_radius: number | null;
  body_name: string | null;
  altitude: number | null;
  period: number | null;
}

export interface RrdRow {
  t: number;
  x: number;
  y: number;
  z: number;
  vx: number;
  vy: number;
  vz: number;
  entity_path: string | null;
  quaternion: [number, number, number, number] | null;
  angular_velocity: [number, number, number] | null;
}

export interface ParsedRrd {
  metadata: RrdMetadata;
  rows: RrdRow[];
}

type ParseRrdFn = (bytes: Uint8Array) => ParsedRrd;

let initialized = false;
let initPromise: Promise<void> | undefined;
let wasmParseRrd: ParseRrdFn | undefined;

/** Initialize the rrd-wasm WASM module. Safe to call multiple times. */
export function initRrdWasm(): Promise<void> {
  if (initialized) return Promise.resolve();
  if (initPromise) return initPromise;

  const p: Promise<void> = import("./rrd-wasm/rrd_wasm.js").then(async (mod) => {
    await mod.default();
    wasmParseRrd = mod.parse_rrd as ParseRrdFn;
    initialized = true;
  });
  initPromise = p;
  return p;
}

/** Whether the WASM module is loaded and ready. */
export function isRrdWasmReady(): boolean {
  return initialized;
}

/** Parse an RRD file from bytes. Must call initRrdWasm() first. */
export function parseRrd(bytes: Uint8Array): ParsedRrd {
  return wasmParseRrd!(bytes);
}
