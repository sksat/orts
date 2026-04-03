/**
 * RRD parse worker message types.
 *
 * Shared between the Worker and the main thread (RrdFileAdapter).
 */

import type { RrdMetadata } from "../wasm/rrdWasmInit.js";

/** Messages from main thread → Worker */
export type RrdWorkerInput = {
  type: "parse";
  buffer: ArrayBuffer;
};

/** Messages from Worker → main thread */
export type RrdWorkerMessage =
  | { type: "metadata"; metadata: RrdMetadata }
  | { type: "chunk"; points: RrdPointOut[]; done: boolean }
  | { type: "error"; message: string };

/** A single point output from the Worker (raw state vector, no Keplerian). */
export interface RrdPointOut {
  t: number;
  x: number;
  y: number;
  z: number;
  vx: number;
  vy: number;
  vz: number;
  entityPath: string | null;
  qw?: number;
  qx?: number;
  qy?: number;
  qz?: number;
  wx?: number;
  wy?: number;
  wz?: number;
}
