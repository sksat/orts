/**
 * Derive the orbital quantities a `.rrd` recording does not carry.
 *
 * The decoder recovers position and velocity; the Keplerian elements and the
 * scalar quantities the charts plot have to be computed from those. The
 * WebSocket source gets them from the server, so this is the file source's
 * half of the same job.
 *
 * The arithmetic lives in `arika-wasm` (`orbit_derived_batch`), which calls the
 * same `KeplerianElements::from_state_vector` the CLI writes into CSV.
 */

import type { OrbitPoint } from "../orbit.js";
import type { RrdPointOut } from "./rrdParseLogic.js";

/** Values `orbit_derived_batch` returns per state. */
export const ORBIT_DERIVED_STRIDE = 10;

/** Flatten points into the `[x,y,z,vx,vy,vz, ...]` layout the batch takes. */
export function packStates(points: readonly RrdPointOut[]): Float64Array {
  const out = new Float64Array(points.length * 6);
  for (let i = 0; i < points.length; i++) {
    const p = points[i];
    const o = i * 6;
    out[o] = p.x;
    out[o + 1] = p.y;
    out[o + 2] = p.z;
    out[o + 3] = p.vx;
    out[o + 4] = p.vy;
    out[o + 5] = p.vz;
  }
  return out;
}

/**
 * Build `OrbitPoint`s from decoded states and the batch's output.
 *
 * `derived` is the flat result of `orbit_derived_batch` over the same points in
 * the same order. A state with no orbital plane comes back as `NaN`s and is
 * carried through as such, because zero is a real reading — a circular
 * equatorial orbit at the body's surface — and cannot double as "no value".
 */
export function toOrbitPoints(
  points: readonly RrdPointOut[],
  derived: Float64Array | readonly number[],
): OrbitPoint[] {
  if (derived.length !== points.length * ORBIT_DERIVED_STRIDE) {
    throw new Error(
      `orbit-derived batch returned ${derived.length} values for ${points.length} points ` +
        `(expected ${points.length * ORBIT_DERIVED_STRIDE})`,
    );
  }
  return points.map((p, i) => {
    const o = i * ORBIT_DERIVED_STRIDE;
    return {
      t: p.t,
      x: p.x,
      y: p.y,
      z: p.z,
      vx: p.vx,
      vy: p.vy,
      vz: p.vz,
      entityPath: p.entityPath ?? undefined,
      a: derived[o],
      e: derived[o + 1],
      inc: derived[o + 2],
      raan: derived[o + 3],
      omega: derived[o + 4],
      nu: derived[o + 5],
      altitude: derived[o + 6],
      specific_energy: derived[o + 7],
      angular_momentum: derived[o + 8],
      velocity_mag: derived[o + 9],
      qw: p.qw,
      qx: p.qx,
      qy: p.qy,
      qz: p.qz,
      wx: p.wx,
      wy: p.wy,
      wz: p.wz,
    };
  });
}
