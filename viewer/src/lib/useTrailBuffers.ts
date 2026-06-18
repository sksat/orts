import { useLayoutEffect, useMemo, useRef } from "react";
import { TrailBuffer, type TrailBufferLike } from "../utils/TrailBuffer.js";
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
 * Two trail modes per satellite (see {@link SatelliteState}):
 * - **value** (`sat.trail`): this hook owns a buffer and reconciles it from the
 *   point array. `sat.trail` must be treated immutably (new array reference on
 *   change) — the reconcile is keyed on the array's identity.
 * - **streaming** (`sat.trailBuffer`): the caller owns the buffer and mutates it
 *   outside React; it's used as-is (no reconcile, no second buffer). Keep its
 *   identity stable across renders.
 */
export function useTrailBuffers(
  satellites: readonly SatelliteState[],
): Map<string, TrailBufferLike> {
  const store = useRef(new Map<string, TrailEntry>());

  // Render phase: assemble the buffer map with stable identities. A caller-owned
  // buffer (streaming mode) is used directly; a value-mode satellite gets a
  // stable owned buffer whose *contents* are filled in the commit phase below.
  // Marker-only satellites (neither trail nor buffer) get no buffer, so no
  // OrbitTrail is mounted for them.
  const live = useMemo(() => {
    const map = new Map<string, TrailBufferLike>();
    for (const sat of satellites) {
      if (sat.trailBuffer != null) {
        map.set(sat.id, sat.trailBuffer);
        continue;
      }
      if (sat.trail === undefined) continue;
      let entry = store.current.get(sat.id);
      if (!entry) {
        entry = {
          buffer: new TrailBuffer(Math.max(sat.trail.length * 2, INITIAL_CAPACITY)),
          sync: null,
        };
        store.current.set(sat.id, entry);
      }
      map.set(sat.id, entry.buffer);
    }
    return map;
  }, [satellites]);

  // Commit phase: fill value-mode buffers (idempotent per input set, covering
  // StrictMode's double-invoked effects). Forget owned buffers for satellites
  // that vanished or switched to a caller-owned buffer.
  useLayoutEffect(() => {
    const owned = new Set<string>();
    for (const sat of satellites) {
      if (sat.trailBuffer != null || sat.trail === undefined) continue;
      owned.add(sat.id);
      const entry = store.current.get(sat.id);
      if (entry) reconcileTrailEntry(entry, sat.trail, sat.trailVersion);
    }
    for (const id of store.current.keys()) {
      if (!owned.has(id)) store.current.delete(id);
    }
  }, [satellites]);

  return live;
}
