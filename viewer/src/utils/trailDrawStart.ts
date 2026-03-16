import type { TimeRange } from "uneri";
import type { OrbitPoint } from "../orbit.js";

/**
 * Compute the draw-start index for a replay-mode points array.
 *
 * Returns the index of the last point whose time is at or before
 * `currentT - timeRange`, so the trail starts just outside the
 * visible window for visual continuity.
 *
 * Returns 0 when timeRange is null (show all) or when the window
 * covers all available data.
 */
export function computeReplayDrawStart(
  points: OrbitPoint[],
  currentT: number,
  timeRange: TimeRange,
): number {
  if (timeRange == null || points.length === 0) return 0;

  const startT = currentT - timeRange;
  if (startT <= points[0].t) return 0;

  // Binary search: find last index with t <= startT
  let lo = 0;
  let hi = points.length - 1;
  if (startT >= points[hi].t) return hi;

  while (hi - lo > 1) {
    const mid = (lo + hi) >>> 1;
    if (points[mid].t <= startT) {
      lo = mid;
    } else {
      hi = mid;
    }
  }
  return lo;
}
