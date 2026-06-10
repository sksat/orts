import { useLayoutEffect, useMemo, useRef } from "react";
import { TrailBuffer } from "../utils/TrailBuffer.js";
import { reconcileTrailEntry, type TrailEntry } from "./trailReconcile.js";
import type { SatelliteState } from "./types.js";

/**
 * Floor for a trail buffer's capacity (in points). `TrailBuffer` is *bounded*:
 * once a trail exceeds ~1.5× its capacity it trims the oldest points and bumps
 * its generation (a one-off full re-upload). Capacity is fixed at creation
 * (max of 2× the initial trail length and this floor), so raise it if you need
 * very long histories without trimming.
 */
const INITIAL_CAPACITY = 4096;

/**
 * Keep one persistent {@link TrailBuffer} per satellite across renders, returning
 * a map of stable buffer instances.
 *
 * Stable identity is the whole point: `OrbitTrail` rebuilds its GPU geometry only
 * when the buffer object changes, so re-using the same instance lets it upload
 * just the appended tail (or do a full rewrite only on a real reset).
 *
 * Phasing (#91): the render phase only guarantees *identities* — a stable empty
 * buffer is created the first time an id is seen (idempotent and content-free,
 * so a render that React later aborts at worst leaves an empty buffer to be
 * pruned). All *content* mutations — fill, append, rebuild, and pruning of
 * vanished ids — happen in `useLayoutEffect`, i.e. only for committed renders,
 * so an aborted concurrent render can never leak points into the buffers.
 * `OrbitTrail` reads contents in `useFrame` (after layout effects run, before
 * the next animation frame), so the commit-phase fill adds no frame lag.
 *
 * `satellites` (and each `sat.trail`) must be treated immutably — supply new
 * array references when points change, as with any React prop. The reconcile is
 * keyed on the array's identity, so in-place mutation won't be picked up.
 */
export function useTrailBuffers(satellites: readonly SatelliteState[]): Map<string, TrailBuffer> {
  const store = useRef(new Map<string, TrailEntry>());

  // Render phase: ensure a stable buffer identity exists for each current id.
  const live = useMemo(() => {
    const map = new Map<string, TrailBuffer>();
    for (const sat of satellites) {
      let entry = store.current.get(sat.id);
      if (!entry) {
        entry = {
          buffer: new TrailBuffer(Math.max((sat.trail?.length ?? 0) * 2, INITIAL_CAPACITY)),
          sync: null,
        };
        store.current.set(sat.id, entry);
      }
      map.set(sat.id, entry.buffer);
    }
    return map;
  }, [satellites]);

  // Commit phase: apply content mutations only for renders that actually
  // committed. reconcileTrailEntry is idempotent per input set, which also
  // covers StrictMode's double-invoked effects.
  useLayoutEffect(() => {
    const seen = new Set<string>();
    for (const sat of satellites) {
      seen.add(sat.id);
      const entry = store.current.get(sat.id);
      if (entry) reconcileTrailEntry(entry, sat.trail ?? [], sat.trailVersion);
    }
    // Forget buffers for satellites that are no longer present.
    for (const id of store.current.keys()) {
      if (!seen.has(id)) store.current.delete(id);
    }
  }, [satellites]);

  return live;
}
