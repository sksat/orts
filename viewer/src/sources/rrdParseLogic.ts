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
  quaternion?: ArrayLike<number> | null;
  angular_velocity?: ArrayLike<number> | null;
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

  // Attitude is optional, and arrives whole or not at all. A `quaternion` column
  // that is present but not four long would otherwise leave the missing
  // components undefined, and a sample carrying only some of them reads
  // downstream as no attitude — which lets a registered model stand at its own
  // orientation, with the scene scaled to that model. `NaN` is what those
  // components are; the display frame refuses them, and the spacecraft gets the
  // marker that shows no orientation.
  if (row.quaternion) {
    const complete = row.quaternion.length === 4;
    point.qw = complete ? row.quaternion[0] : Number.NaN;
    point.qx = complete ? row.quaternion[1] : Number.NaN;
    point.qy = complete ? row.quaternion[2] : Number.NaN;
    point.qz = complete ? row.quaternion[3] : Number.NaN;
  }
  if (row.angular_velocity) {
    point.wx = row.angular_velocity[0];
    point.wy = row.angular_velocity[1];
    point.wz = row.angular_velocity[2];
  }
  return point;
}
