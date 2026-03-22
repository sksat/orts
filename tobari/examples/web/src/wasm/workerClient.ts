/**
 * Main-thread client for the WASM Web Worker.
 *
 * Provides async wrappers around WASM functions that run in the worker.
 * Automatically cancels stale requests — only the latest call per function
 * resolves, so rapid parameter changes during animation don't queue up.
 */

import type { WorkerRequest, WorkerResponse } from "./worker.js";

let worker: Worker | null = null;
let readyPromise: Promise<void> | null = null;
let nextId = 0;
const pending = new Map<number, (result: unknown) => void>();
/** Tracks the latest request ID per function for cancellation. */
const latestId = new Map<string, number>();

/** Callback for when space weather data is loaded in the worker. */
let swReadyCallback: ((range: { jdFirst: number; jdLast: number }) => void) | null = null;

/** Register a callback for when space weather data becomes available. */
export function onSpaceWeatherReady(
  cb: (range: { jdFirst: number; jdLast: number }) => void,
): void {
  swReadyCallback = cb;
}

export function initWorker(): Promise<void> {
  if (readyPromise) return readyPromise;

  readyPromise = new Promise<void>((resolve, reject) => {
    worker = new Worker(new URL("./worker.ts", import.meta.url), {
      type: "module",
    });
    worker.onmessage = (e: MessageEvent) => {
      if (e.data.type === "ready") {
        resolve();
        return;
      }
      if (e.data.type === "error") {
        reject(new Error(e.data.message ?? "Worker WASM init failed"));
        return;
      }
      if (e.data.type === "sw_ready") {
        swReadyCallback?.({
          jdFirst: e.data.jdFirst,
          jdLast: e.data.jdLast,
        });
        return;
      }
      if (e.data.type === "result") {
        const { id, result } = e.data as WorkerResponse;
        const cb = pending.get(id);
        if (cb) {
          pending.delete(id);
          cb(result);
        }
      }
    };
    worker.onerror = (e) => {
      reject(new Error(`Worker error: ${e.message}`));
    };
  });
  return readyPromise;
}

/**
 * Call a WASM function in the worker.
 * Waits for the worker to be ready before sending.
 * If a newer call with the same `fn` arrives before this one completes,
 * this promise resolves with `null` (cancelled).
 */
async function call(fn: string, args: unknown[]): Promise<unknown> {
  // Ensure worker is initialized before sending any message
  await readyPromise;

  const id = nextId++;
  latestId.set(fn, id);

  return new Promise((resolve) => {
    pending.set(id, (result) => {
      // Only deliver if this is still the latest request for this fn
      if (latestId.get(fn) === id) {
        resolve(result);
      } else {
        resolve(null); // cancelled — newer request superseded
      }
    });
    worker!.postMessage({ id, fn, args } satisfies WorkerRequest);
  });
}

// ---------------------------------------------------------------------------
// Typed async wrappers
// ---------------------------------------------------------------------------

export async function atmosphereLatlonMapAsync(
  model: string,
  altitudeKm: number,
  epochJd: number,
  nLat: number,
  nLon: number,
  f107: number,
  ap: number,
): Promise<Float64Array | null> {
  return (await call("atmosphere_latlon_map", [
    model,
    altitudeKm,
    epochJd,
    nLat,
    nLon,
    f107,
    ap,
  ])) as Float64Array | null;
}

export interface VolumeResult {
  data: Float32Array;
  min: number;
  max: number;
}

export async function atmosphereVolumeAsync(
  model: string,
  altMinKm: number,
  altMaxKm: number,
  nAlt: number,
  epochJd: number,
  nLat: number,
  nLon: number,
  f107: number,
  ap: number,
): Promise<VolumeResult | null> {
  const raw = (await call("atmosphere_volume", [
    model,
    altMinKm,
    altMaxKm,
    nAlt,
    epochJd,
    nLat,
    nLon,
    f107,
    ap,
  ])) as Float32Array | null;
  if (!raw) return null;
  const total = nAlt * nLat * nLon;
  return {
    data: raw.slice(0, total),
    min: raw[total],
    max: raw[total + 1],
  };
}

export async function atmosphereAltitudeProfileAsync(
  altitudes: Float64Array,
  latDeg: number,
  lonDeg: number,
  epochJd: number,
  f107: number,
  ap: number,
): Promise<Float64Array | null> {
  return (await call("atmosphere_altitude_profile", [
    altitudes,
    latDeg,
    lonDeg,
    epochJd,
    f107,
    ap,
  ])) as Float64Array | null;
}

export async function magneticFieldLatlonMapAsync(
  model: string,
  component: string,
  altitudeKm: number,
  epochJd: number,
  nLat: number,
  nLon: number,
): Promise<Float64Array | null> {
  return (await call("magnetic_field_latlon_map", [
    model,
    component,
    altitudeKm,
    epochJd,
    nLat,
    nLon,
  ])) as Float64Array | null;
}

export async function magneticFieldVolumeAsync(
  model: string,
  component: string,
  altMinKm: number,
  altMaxKm: number,
  nAlt: number,
  epochJd: number,
  nLat: number,
  nLon: number,
): Promise<VolumeResult | null> {
  const raw = (await call("magnetic_field_volume", [
    model,
    component,
    altMinKm,
    altMaxKm,
    nAlt,
    epochJd,
    nLat,
    nLon,
  ])) as Float32Array | null;
  if (!raw) return null;
  const total = nAlt * nLat * nLon;
  return {
    data: raw.slice(0, total),
    min: raw[total],
    max: raw[total + 1],
  };
}

export async function magneticFieldLinesAsync(
  seedLats: Float64Array,
  seedLons: Float64Array,
  seedAltKm: number,
  epochJd: number,
  model: string,
  maxSteps: number,
  stepKm: number,
): Promise<Float32Array | null> {
  return (await call("magnetic_field_lines", [
    seedLats,
    seedLons,
    seedAltKm,
    epochJd,
    model,
    maxSteps,
    stepKm,
  ])) as Float32Array | null;
}

// ---------------------------------------------------------------------------
// Space weather APIs (using loaded CSSI/GFZ data)
// ---------------------------------------------------------------------------

export async function atmosphereLatlonMapSwAsync(
  model: string,
  altitudeKm: number,
  epochJd: number,
  nLat: number,
  nLon: number,
): Promise<Float64Array | null> {
  return (await call("atmosphere_latlon_map_sw", [
    model,
    altitudeKm,
    epochJd,
    nLat,
    nLon,
  ])) as Float64Array | null;
}

export async function atmosphereVolumeSwAsync(
  model: string,
  altMinKm: number,
  altMaxKm: number,
  nAlt: number,
  epochJd: number,
  nLat: number,
  nLon: number,
): Promise<VolumeResult | null> {
  const raw = (await call("atmosphere_volume_sw", [
    model,
    altMinKm,
    altMaxKm,
    nAlt,
    epochJd,
    nLat,
    nLon,
  ])) as Float32Array | null;
  if (!raw) return null;
  const total = nAlt * nLat * nLon;
  return {
    data: raw.slice(0, total),
    min: raw[total],
    max: raw[total + 1],
  };
}

export async function spaceWeatherLookupAsync(epochJd: number): Promise<Float64Array | null> {
  return (await call("space_weather_lookup", [epochJd])) as Float64Array | null;
}

/** Get all space weather records for charting. Returns flat [jd, f107, ap, ...]. */
export async function spaceWeatherSeriesAsync(): Promise<Float64Array | null> {
  return (await call("space_weather_series", [])) as Float64Array | null;
}
