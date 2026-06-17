import type { TrailBuffer } from "../utils/TrailBuffer.js";
import { trailPointToOrbitPoint } from "./adapt.js";
import { decideTrailUpdate, type TrailSyncState, trailSyncState } from "./trailSync.js";
import type { TrailPoint } from "./types.js";

/** A persistent per-satellite trail buffer plus the sync state of its contents. */
export interface TrailEntry {
  buffer: TrailBuffer;
  sync: TrailSyncState | null;
}

/** Reconcile `entry.buffer` to hold exactly `points` (append when possible). */
export function reconcileTrailEntry(
  entry: TrailEntry,
  points: readonly TrailPoint[],
  version: string | number | undefined,
): void {
  const decision = decideTrailUpdate(entry.sync, points, version);
  if (decision.kind === "rebuild") {
    entry.buffer.clear();
    entry.buffer.pushMany(points.map(trailPointToOrbitPoint));
    entry.sync = trailSyncState(points, version);
  } else if (decision.kind === "append") {
    const from = decision.from;
    entry.buffer.pushMany(points.slice(from).map((p, i) => trailPointToOrbitPoint(p, from + i)));
    entry.sync = trailSyncState(points, version);
  }
}
