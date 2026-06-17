import type { TrailPoint, Vec3 } from "./types.js";

/** Snapshot of what a trail buffer currently holds. */
export interface TrailSyncState {
  length: number;
  version: string | number | undefined;
  /**
   * Position of the last point (index `length - 1`), or undefined when empty.
   * Stored as a copy so an in-place Vec3 mutation by the caller can't silently
   * mutate the captured state and defeat change detection.
   */
  lastPosition: Vec3 | undefined;
  /**
   * Time of the last point. Affects body-fixed/ECEF vertices (each point is
   * de-rotated by the Earth-rotation angle at its own time), so a time change on
   * an otherwise-identical point must still trigger a rebuild.
   */
  lastTime: number | undefined;
}

/** Reconciliation decision for a trail buffer. */
export type TrailUpdate = { kind: "noop" } | { kind: "rebuild" } | { kind: "append"; from: number };

function vec3Equal(a: Vec3 | undefined, b: Vec3 | undefined): boolean {
  if (!a || !b) return a === b;
  return a[0] === b[0] && a[1] === b[1] && a[2] === b[2];
}

/** Capture the sync state for a trail array + version. */
export function trailSyncState(
  points: readonly TrailPoint[],
  version: string | number | undefined,
): TrailSyncState {
  const last = points.length > 0 ? points[points.length - 1] : undefined;
  return {
    length: points.length,
    version,
    // Copy the Vec3 so a later in-place mutation by the caller can't change it.
    lastPosition: last ? [last.position[0], last.position[1], last.position[2]] : undefined,
    lastTime: last?.time,
  };
}

/**
 * Decide whether to leave the buffer alone, append the new tail, or rebuild.
 *
 * Order matters: an explicit version bump or a shrink always rebuilds; a grow is
 * only treated as an append when the previously-last point still matches (i.e.
 * the history wasn't rewritten).
 */
export function decideTrailUpdate(
  prev: TrailSyncState | null,
  points: readonly TrailPoint[],
  version: string | number | undefined,
): TrailUpdate {
  if (prev === null) {
    return points.length === 0 ? { kind: "noop" } : { kind: "rebuild" };
  }
  if (version !== prev.version) return { kind: "rebuild" };
  if (points.length < prev.length) return { kind: "rebuild" };

  // The point that used to be the tail must still be there unchanged, otherwise
  // earlier history was rewritten and an append would corrupt the line. Compare
  // both position and time (time affects body-fixed/ECEF vertices).
  if (prev.length > 0) {
    const tail = points[prev.length - 1];
    if (!vec3Equal(tail?.position, prev.lastPosition) || tail?.time !== prev.lastTime) {
      return { kind: "rebuild" };
    }
  }

  if (points.length > prev.length) return { kind: "append", from: prev.length };
  return { kind: "noop" };
}
