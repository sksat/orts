import { eci_to_ecef_batch as wasmBatch } from "./wasm/kanameInit.js";
import type { OrbitPoint } from "./orbit.js";

/**
 * Batch-transform orbit points from ECI to ECEF via WASM, writing scaled
 * results into `outBuf` starting at vertex index `outOffset`.
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
}
