import { useMemo, useRef } from "react";
import { TrailBuffer } from "../utils/TrailBuffer.js";
import { trailPointToOrbitPoint } from "./adapt.js";
import { decideTrailUpdate, type TrailSyncState, trailSyncState } from "./trailSync.js";
import type { SatelliteState } from "./types.js";

/**
 * Floor for a trail buffer's capacity (in points). `TrailBuffer` is *bounded*:
 * once a trail exceeds ~1.5× its capacity it trims the oldest points and bumps
 * its generation (a one-off full re-upload). Capacity is fixed at creation
 * (max of 2× the initial trail length and this floor), so raise it if you need
 * very long histories without trimming.
 */
const INITIAL_CAPACITY = 4096;

interface Entry {
  buffer: TrailBuffer;
  sync: TrailSyncState | null;
}

/**
 * Keep one persistent {@link TrailBuffer} per satellite across renders, returning
 * a map of stable buffer instances.
 *
 * Stable identity is the whole point: `OrbitTrail` rebuilds its GPU geometry only
 * when the buffer object changes, so re-using the same instance lets it upload
 * just the appended tail (or do a full rewrite only on a real reset). The reconcile
 * is keyed on `satellites`, so advancing `time` alone never touches the buffers.
 */
export function useTrailBuffers(satellites: readonly SatelliteState[]): Map<string, TrailBuffer> {
  const store = useRef(new Map<string, Entry>());

  // Reconciled in useMemo (render phase) rather than an effect so children see
  // the latest points in the same commit — no one-frame trail lag. Idempotent
  // against StrictMode's double render: decideTrailUpdate compares against the
  // stored sync state, so a repeat with the same inputs is a "noop" (never
  // double-appends).
  // TODO(#91): render-phase mutation is NOT safe against a React 19 render that
  // is aborted and then committed with different props; the robust form computes
  // a pure plan here and applies it in a useLayoutEffect / useFrame pre-pass.
  return useMemo(() => {
    const entries = store.current;
    const live = new Map<string, TrailBuffer>();
    const seen = new Set<string>();

    for (const sat of satellites) {
      seen.add(sat.id);
      const points = sat.trail ?? [];
      let entry = entries.get(sat.id);
      if (!entry) {
        entry = {
          buffer: new TrailBuffer(Math.max(points.length * 2, INITIAL_CAPACITY)),
          sync: null,
        };
        entries.set(sat.id, entry);
      }

      const decision = decideTrailUpdate(entry.sync, points, sat.trailVersion);
      if (decision.kind === "rebuild") {
        entry.buffer.clear();
        entry.buffer.pushMany(points.map(trailPointToOrbitPoint));
        entry.sync = trailSyncState(points, sat.trailVersion);
      } else if (decision.kind === "append") {
        const from = decision.from;
        entry.buffer.pushMany(
          points.slice(from).map((p, i) => trailPointToOrbitPoint(p, from + i)),
        );
        entry.sync = trailSyncState(points, sat.trailVersion);
      }

      live.set(sat.id, entry.buffer);
    }

    // Forget buffers for satellites that are no longer present.
    for (const id of entries.keys()) {
      if (!seen.has(id)) entries.delete(id);
    }

    return live;
  }, [satellites]);
}
