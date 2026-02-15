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

let initialized = false;
let initPromise: Promise<void> | undefined;
let wasmBatch: EciToEcefBatch | undefined;
let wasmSingle: EciToEcef | undefined;
let wasmEra: EarthRotationAngle | undefined;

/** Initialize the kaname WASM module. Safe to call multiple times. Rejects on failure. */
export function initKaname(): Promise<void> {
  if (initialized) return Promise.resolve();
  if (initPromise) return initPromise;

  const p: Promise<void> = import("./kaname/kaname.js").then(async (mod) => {
    await mod.default();
    wasmBatch = mod.eci_to_ecef_batch;
    wasmSingle = mod.eci_to_ecef;
    wasmEra = mod.earth_rotation_angle;
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
