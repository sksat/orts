/**
 * Web Worker for tobari/kaname WASM computation.
 *
 * Keeps heavy WASM calls off the main thread so UI (OrbitControls, etc.)
 * stays responsive during animation.
 */

import initTobari, {
  atmosphere_latlon_map,
  atmosphere_volume,
  atmosphere_altitude_profile,
  magnetic_field_latlon_map,
  magnetic_field_lines,
  igrf_field_at,
  dipole_field_at,
} from "./tobari/tobari.js";

let ready = false;

async function init() {
  try {
    await initTobari();
    ready = true;
    self.postMessage({ type: "ready" });
  } catch (e) {
    self.postMessage({ type: "error", message: String(e) });
  }
}

init();

export interface WorkerRequest {
  id: number;
  fn: string;
  args: unknown[];
}

export interface WorkerResponse {
  type: "result";
  id: number;
  result: unknown;
}

const FN_MAP: Record<string, (...args: any[]) => unknown> = {
  atmosphere_latlon_map,
  atmosphere_volume,
  atmosphere_altitude_profile,
  magnetic_field_latlon_map,
  magnetic_field_lines,
  igrf_field_at,
  dipole_field_at,
};

self.onmessage = (e: MessageEvent<WorkerRequest>) => {
  if (!ready) {
    // Return null for requests that arrive before init completes
    self.postMessage({ type: "result", id: e.data.id, result: null });
    return;
  }
  const { id, fn, args } = e.data;
  const func = FN_MAP[fn];
  if (!func) {
    self.postMessage({ type: "result", id, result: null });
    return;
  }
  const result = func(...args);
  // Transfer typed arrays for zero-copy
  const transfer: Transferable[] = [];
  if (result instanceof Float32Array || result instanceof Float64Array) {
    transfer.push(result.buffer);
  }
  self.postMessage({ type: "result", id, result } as WorkerResponse, { transfer });
};
