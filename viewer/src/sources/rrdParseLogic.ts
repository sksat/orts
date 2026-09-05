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

/** One decoded row as rrd-wasm hands it over. */
export interface RrdRowIn {
  t: number;
  x: number;
  y: number;
  z: number;
  vx: number;
  vy: number;
  vz: number;
  entity_path: string | null;
  /** Four components or nothing — see {@link rowToPoint}. */
  quaternion?: readonly [number, number, number, number] | null;
  angular_velocity?: readonly [number, number, number] | null;
}

/**
 * One decoded row as a point the viewer can draw.
 *
 * Lives here rather than in the worker so the boundary can be tested: what a
 * malformed row turns into is the whole question, and a worker's message handler
 * is not reachable from a unit test.
 */
export function rowToPoint(row: RrdRowIn): RrdPointOut {
  const point: RrdPointOut = {
    t: row.t,
    x: row.x,
    y: row.y,
    z: row.z,
    vx: row.vx,
    vy: row.vy,
    vz: row.vz,
    entityPath: row.entity_path,
  };

  // Attitude is optional, and arrives whole or not at all: the decoder builds
  // `Some([qw?, qx?, qy?, qz?])`, so a row missing any one component yields
  // `None` rather than a short list (`rrd-wasm/src/lib.rs`). A complete tuple can
  // still carry `NaN` — a diverged simulation writes one — and that reaches the
  // display frame as an attitude to refuse, which is the behaviour wanted.
  if (row.quaternion) {
    point.qw = row.quaternion[0];
    point.qx = row.quaternion[1];
    point.qy = row.quaternion[2];
    point.qz = row.quaternion[3];
  }
  if (row.angular_velocity) {
    point.wx = row.angular_velocity[0];
    point.wy = row.angular_velocity[1];
    point.wz = row.angular_velocity[2];
  }
  return point;
}
