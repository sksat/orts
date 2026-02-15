import {
  isKanameReady,
  eci_to_ecef_batch as wasmBatch,
} from "./wasm/kanameInit.js";
import { earthRotationAngle } from "./astro.js";
import { eciToEcef } from "./frameTransform.js";
import type { OrbitPoint } from "./orbit.js";

/** Minimum point count to prefer the WASM batch path over plain TypeScript. */
const WASM_BATCH_THRESHOLD = 16;

/**
 * Batch-transform orbit points from ECI to ECEF, writing scaled results
 * into `outBuf` starting at vertex index `outOffset`.
 *
 * Uses WASM when loaded and `count > WASM_BATCH_THRESHOLD`; otherwise
 * falls back to the existing TypeScript implementation.
 */
export function batchEciToEcef(
  points: OrbitPoint[],
  from: number,
  to: number,
  epochJd: number,
  outBuf: Float32Array,
  outOffset: number,
  scaleRadius: number,
): void {
  const count = to - from;

  if (isKanameReady() && count > WASM_BATCH_THRESHOLD) {
    // --- WASM path ---
    const positions = new Float32Array(count * 3);
    const times = new Float32Array(count);
    for (let i = 0; i < count; i++) {
      const p = points[from + i];
      positions[i * 3] = p.x;
      positions[i * 3 + 1] = p.y;
      positions[i * 3 + 2] = p.z;
      times[i] = p.t;
    }

    const ecef = wasmBatch(positions, times, epochJd);

    const invScale = 1 / scaleRadius;
    for (let i = 0; i < count; i++) {
      const outOff = (outOffset + i) * 3;
      outBuf[outOff] = ecef[i * 3] * invScale;
      outBuf[outOff + 1] = ecef[i * 3 + 1] * invScale;
      outBuf[outOff + 2] = ecef[i * 3 + 2] * invScale;
    }
  } else {
    // --- TypeScript fallback ---
    const invScale = 1 / scaleRadius;
    for (let i = 0; i < count; i++) {
      const p = points[from + i];
      const era = earthRotationAngle(epochJd, p.t);
      const [ex, ey, ez] = eciToEcef(p.x, p.y, p.z, era);
      const outOff = (outOffset + i) * 3;
      outBuf[outOff] = ex * invScale;
      outBuf[outOff + 1] = ey * invScale;
      outBuf[outOff + 2] = ez * invScale;
    }
  }
}
