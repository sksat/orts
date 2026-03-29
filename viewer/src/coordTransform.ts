import type { OrbitPoint } from "./orbit.js";
import type { LvlhAxes } from "./sceneFrame.js";
import { eci_to_ecef_batch as wasmBatch } from "./wasm/kanameInit.js";

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

/**
 * Batch-transform orbit points by subtracting an origin offset and scaling.
 *
 * Used for satellite-centered (and future Moon/Sun-centered) views where
 * the origin is shifted from the central body to another object.
 *
 * @param origin  Position of the new origin in ECI [km], or null for no offset.
 */
export function batchTransformWithOffset(
  points: OrbitPoint[],
  from: number,
  to: number,
  origin: [number, number, number] | null,
  outBuf: Float32Array,
  outOffset: number,
  scaleRadius: number,
): void {
  const invScale = 1 / scaleRadius;
  const ox = origin?.[0] ?? 0;
  const oy = origin?.[1] ?? 0;
  const oz = origin?.[2] ?? 0;

  for (let i = from; i < to; i++) {
    const p = points[i];
    const outOff = (outOffset + i - from) * 3;
    outBuf[outOff] = (p.x - ox) * invScale;
    outBuf[outOff + 1] = (p.y - oy) * invScale;
    outBuf[outOff + 2] = (p.z - oz) * invScale;
  }
}

/**
 * Transform a single ECI point into the satellite's LVLH body frame.
 *
 * LVLH scene axis mapping (Three.js Z-up):
 *   X = inTrack (along orbit), Y = crossTrack (orbit normal), Z = radial (outward)
 *
 * The subtraction and dot products are computed in f64 before the result is
 * returned, avoiding float32 precision loss from large ECI coordinates.
 */
export function transformToLvlh(
  px: number,
  py: number,
  pz: number,
  origin: [number, number, number],
  axes: LvlhAxes,
  scaleRadius: number,
): [number, number, number] {
  const dx = px - origin[0];
  const dy = py - origin[1];
  const dz = pz - origin[2];
  const invScale = 1 / scaleRadius;

  return [
    (axes.inTrack[0] * dx + axes.inTrack[1] * dy + axes.inTrack[2] * dz) * invScale,
    (axes.crossTrack[0] * dx + axes.crossTrack[1] * dy + axes.crossTrack[2] * dz) * invScale,
    (axes.radial[0] * dx + axes.radial[1] * dy + axes.radial[2] * dz) * invScale,
  ];
}

/**
 * Batch-transform orbit points from ECI into the satellite's LVLH body frame,
 * writing scaled results into `outBuf` starting at vertex index `outOffset`.
 *
 * All arithmetic (subtraction + dot product) is performed in f64 before
 * writing to the Float32Array, preserving precision for satellite-relative
 * coordinates.
 */
export function batchTransformToLvlh(
  points: OrbitPoint[],
  from: number,
  to: number,
  origin: [number, number, number],
  axes: LvlhAxes,
  outBuf: Float32Array,
  outOffset: number,
  scaleRadius: number,
): void {
  const invScale = 1 / scaleRadius;
  const [ox, oy, oz] = origin;
  const { radial, inTrack, crossTrack } = axes;

  for (let i = from; i < to; i++) {
    const p = points[i];
    const dx = p.x - ox;
    const dy = p.y - oy;
    const dz = p.z - oz;

    const outOff = (outOffset + i - from) * 3;
    outBuf[outOff] = (inTrack[0] * dx + inTrack[1] * dy + inTrack[2] * dz) * invScale;
    outBuf[outOff + 1] = (crossTrack[0] * dx + crossTrack[1] * dy + crossTrack[2] * dz) * invScale;
    outBuf[outOff + 2] = (radial[0] * dx + radial[1] * dy + radial[2] * dz) * invScale;
  }
}

// --- High/Low split encoding for GPU precision ---

const HIGH_LOW_SPLIT = 65536.0; // 2^16

/**
 * Split a float64 value into high and low float32 parts using 2^16 chunking.
 * Reconstruction: value ≈ high + low (in float64 arithmetic).
 *
 * In the GPU shader, `(posHigh - originHigh) + (posLow - originLow)`
 * cancels the large common parts, preserving sub-meter precision even
 * at interplanetary distances.
 */
export function encodeFloat64ToHighLow(value: number): [high: number, low: number] {
  if (value >= 0.0) {
    const high = Math.floor(value / HIGH_LOW_SPLIT) * HIGH_LOW_SPLIT;
    return [high, value - high];
  }
  const doubleHigh = Math.floor(-value / HIGH_LOW_SPLIT) * HIGH_LOW_SPLIT;
  return [-doubleHigh, value + doubleHigh];
}

/**
 * Batch-encode ECI orbit points into high/low Float32Array pair.
 * Positions are stored in km (not scaled by scaleRadius) so that
 * scaleRadius changes don't invalidate the buffer.
 */
export function batchEncodeEciHighLow(
  points: OrbitPoint[],
  from: number,
  to: number,
  highBuf: Float32Array,
  lowBuf: Float32Array,
  outOffset: number,
): void {
  for (let i = from; i < to; i++) {
    const p = points[i];
    const outOff = (outOffset + i - from) * 3;

    const [xh, xl] = encodeFloat64ToHighLow(p.x);
    const [yh, yl] = encodeFloat64ToHighLow(p.y);
    const [zh, zl] = encodeFloat64ToHighLow(p.z);

    highBuf[outOff] = xh;
    highBuf[outOff + 1] = yh;
    highBuf[outOff + 2] = zh;
    lowBuf[outOff] = xl;
    lowBuf[outOff + 1] = yl;
    lowBuf[outOff + 2] = zl;
  }
}

/**
 * Batch-encode orbit points after ECEF transform into high/low Float32Array pair.
 * Calls WASM ECI→ECEF first, then splits the resulting km values.
 */
export function batchEncodeEcefHighLow(
  points: OrbitPoint[],
  from: number,
  to: number,
  epochJd: number,
  highBuf: Float32Array,
  lowBuf: Float32Array,
  outOffset: number,
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

  for (let i = 0; i < count; i++) {
    const outOff = (outOffset + i) * 3;
    const [xh, xl] = encodeFloat64ToHighLow(ecef[i * 3]);
    const [yh, yl] = encodeFloat64ToHighLow(ecef[i * 3 + 1]);
    const [zh, zl] = encodeFloat64ToHighLow(ecef[i * 3 + 2]);

    highBuf[outOff] = xh;
    highBuf[outOff + 1] = yh;
    highBuf[outOff + 2] = zh;
    lowBuf[outOff] = xl;
    lowBuf[outOff + 1] = yl;
    lowBuf[outOff + 2] = zl;
  }
}
