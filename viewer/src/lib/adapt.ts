/**
 * Adapters from the small public {@link SatelliteState} / {@link TrailPoint}
 * shapes to the viewer's internal `OrbitPoint` / `TrailBuffer` representation.
 *
 * Kept as pure functions so the units/frame/attitude/time contract can be unit
 * tested without rendering anything.
 */

import type { OrbitPoint } from "../orbit.js";
import { TrailBuffer } from "../utils/TrailBuffer.js";
import type { SatelliteState, TrailPoint } from "./types.js";

/**
 * Convert a {@link SatelliteState} to an `OrbitPoint` at the given time.
 *
 * Orbital-element and chart fields the public type doesn't carry are filled
 * with zeros — they're unused by the renderer for marker/trail display.
 */
export function toOrbitPoint(sat: SatelliteState, time: number): OrbitPoint {
  const [x, y, z] = sat.position;
  const [vx, vy, vz] = sat.velocity ?? [0, 0, 0];
  const point: OrbitPoint = {
    entityPath: sat.id,
    t: time,
    x,
    y,
    z,
    vx,
    vy,
    vz,
    a: 0,
    e: 0,
    inc: 0,
    raan: 0,
    omega: 0,
    nu: 0,
  };
  if (sat.attitude) {
    [point.qw, point.qx, point.qy, point.qz] = sat.attitude;
  }
  return point;
}

/**
 * Convert a {@link TrailPoint} to an `OrbitPoint`.
 *
 * Uses the point's own `time` when present (required for body-fixed/ECEF trails,
 * where each point is de-rotated by the Earth-rotation angle at its own time);
 * otherwise falls back to the supplied index so the time axis stays monotonic.
 */
export function trailPointToOrbitPoint(point: TrailPoint, index: number): OrbitPoint {
  const [x, y, z] = point.position;
  return {
    t: point.time ?? index,
    x,
    y,
    z,
    vx: 0,
    vy: 0,
    vz: 0,
    a: 0,
    e: 0,
    inc: 0,
    raan: 0,
    omega: 0,
    nu: 0,
  };
}

/**
 * Build a fresh {@link TrailBuffer} from a list of trail points (oldest first).
 *
 * Convenience for one-shot construction (tests, the advanced "bring your own
 * buffer" path). The live viewer keeps a persistent buffer per satellite and
 * appends incrementally instead — see the trail-sync hook.
 */
export function toTrailBuffer(points: readonly TrailPoint[]): TrailBuffer {
  const buf = new TrailBuffer(Math.max(points.length, 1));
  buf.pushMany(points.map(trailPointToOrbitPoint));
  return buf;
}
