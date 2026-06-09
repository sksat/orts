/**
 * Decide how to reconcile a persistent trail GPU buffer with a freshly-supplied
 * trail array, so the common "append a few points" case re-uses the buffer
 * (cheap incremental upload) instead of rebuilding it (full re-upload).
 *
 * Pure so the append/rebuild policy is unit tested independently of React and
 * Three.js. The owning hook keeps one {@link TrailSyncState} per satellite.
 */

import type { TrailPoint, Vec3 } from "./types.js";

/** Snapshot of what a trail buffer currently holds. */
export interface TrailSyncState {
  length: number;
  version: string | number | undefined;
  /** Position of the last point (index `length - 1`), or undefined when empty. */
  lastPosition: Vec3 | undefined;
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
  return {
    length: points.length,
    version,
    lastPosition: points.length > 0 ? points[points.length - 1].position : undefined,
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
  // earlier history was rewritten and an append would corrupt the line.
  if (prev.length > 0) {
    const tail = points[prev.length - 1]?.position;
    if (!vec3Equal(tail, prev.lastPosition)) return { kind: "rebuild" };
  }

  if (points.length > prev.length) return { kind: "append", from: prev.length };
  return { kind: "noop" };
}
