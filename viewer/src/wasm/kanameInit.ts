import init, { eci_to_ecef_batch } from "./kaname/kaname.js";

let initialized = false;
let initPromise: Promise<void> | null = null;

/** Initialize the kaname WASM module. Safe to call multiple times. */
export function initKaname(): Promise<void> {
  if (initialized) return Promise.resolve();
  if (initPromise) return initPromise;

  initPromise = init().then(() => {
    initialized = true;
  });
  return initPromise;
}

/** Whether the WASM module is loaded and ready. */
export function isKanameReady(): boolean {
  return initialized;
}

export { eci_to_ecef_batch };
