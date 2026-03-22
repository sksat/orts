/**
 * Web Worker for tobari/kaname WASM computation.
 *
 * Keeps heavy WASM calls off the main thread so UI (OrbitControls, etc.)
 * stays responsive during animation.
 */

import initTobari, {
  atmosphere_altitude_profile,
  atmosphere_latlon_map,
  atmosphere_latlon_map_sw,
  atmosphere_volume,
  atmosphere_volume_sw,
  dipole_field_at,
  igrf_field_at,
  load_space_weather,
  magnetic_field_latlon_map,
  magnetic_field_lines,
  magnetic_field_volume,
  space_weather_date_range,
  space_weather_lookup,
  space_weather_series,
} from "./tobari/tobari.js";

let ready = false;
let swLoaded = false;

async function init() {
  try {
    await initTobari();
    ready = true;
    self.postMessage({ type: "ready" });

    // Try to load bundled space weather data
    // Derive base URL from worker location (handles both dev and production)
    try {
      let baseUrl: string;
      const loc = self.location?.href ?? "";
      if (loc.includes("/assets/")) {
        // Production: worker is at .../assets/worker-xxx.js
        baseUrl = loc.substring(0, loc.lastIndexOf("/assets/") + 1);
      } else {
        // Dev: worker runs inline, use origin root
        baseUrl = loc.substring(0, loc.indexOf("/", loc.indexOf("//") + 2) + 1);
      }
      const res = await fetch(`${baseUrl}space-weather.txt`);
      if (res.ok) {
        const text = await res.text();
        swLoaded = load_space_weather(text);
        if (swLoaded) {
          const range = space_weather_date_range();
          self.postMessage({
            type: "sw_ready",
            jdFirst: range[0],
            jdLast: range[1],
          });
        }
      }
    } catch {
      // Space weather data not available — constant mode only
    }
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

// biome-ignore lint/suspicious/noExplicitAny: WASM function signatures vary
const FN_MAP: Record<string, (...args: any[]) => unknown> = {
  atmosphere_latlon_map,
  atmosphere_latlon_map_sw,
  atmosphere_volume,
  atmosphere_volume_sw,
  atmosphere_altitude_profile,
  magnetic_field_latlon_map,
  magnetic_field_lines,
  magnetic_field_volume,
  igrf_field_at,
  dipole_field_at,
  space_weather_lookup,
  space_weather_date_range,
  space_weather_series,
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
