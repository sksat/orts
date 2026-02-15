type EciToEcefBatch = (
  positions: Float32Array,
  times: Float32Array,
  epoch_jd: number,
) => Float32Array;

let initialized = false;
let initPromise: Promise<void> | undefined;
let wasmBatch: EciToEcefBatch | undefined;

/** Initialize the kaname WASM module. Safe to call multiple times. */
export function initKaname(): Promise<void> {
  if (initialized) return Promise.resolve();
  if (initPromise) return initPromise;

  const p: Promise<void> = import("./kaname/kaname.js")
    .then(async (mod) => {
      await mod.default();
      wasmBatch = mod.eci_to_ecef_batch;
      initialized = true;
    })
    .catch((err) => {
      console.warn("kaname WASM not available, using TypeScript fallback:", err);
    });
  initPromise = p;
  return p;
}

/** Whether the WASM module is loaded and ready. */
export function isKanameReady(): boolean {
  return initialized;
}

/** Batch ECI→ECEF transform via WASM. Only call when isKanameReady() is true. */
export function eci_to_ecef_batch(
  positions: Float32Array,
  times: Float32Array,
  epoch_jd: number,
): Float32Array {
  return wasmBatch!(positions, times, epoch_jd);
}
