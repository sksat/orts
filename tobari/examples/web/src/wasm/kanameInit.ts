/**
 * Lazy WASM loader for kaname coordinate/time utilities.
 */

// biome-ignore lint: dynamic import types
let wasmModule: any = undefined;
let initialized = false;
let initPromise: Promise<void> | undefined;

export function initKaname(): Promise<void> {
  if (initialized) return Promise.resolve();
  if (initPromise) return initPromise;

  const p: Promise<void> = import("./kaname/kaname.js").then(async (mod) => {
    await mod.default();
    wasmModule = mod;
    initialized = true;
  });
  initPromise = p;
  return p;
}

export function isKanameReady(): boolean {
  return initialized;
}

/** Compute GMST (Earth Rotation Angle) in radians via kaname WASM. */
export function earthRotationAngle(epochJd: number): number {
  return wasmModule!.earth_rotation_angle(epochJd, 0);
}
