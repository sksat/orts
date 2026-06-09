import type { OrbitPoint } from "../orbit.js";
import type { TrailBuffer } from "../utils/TrailBuffer.js";

/** A satellite to render: its trail buffer (if any) and current position (if any). */
export interface RenderEntry {
  satId: string;
  buf: TrailBuffer | undefined;
  pos: OrbitPoint | null | undefined;
}

/**
 * The satellites to render: the union of those with a non-empty trail and those
 * with a current position.
 *
 * Markers are driven by position and trails by the buffer, so a satellite given
 * only a position (no trail history) still shows a marker — the streaming app
 * always has both, but an embedder may just want to drop a point somewhere.
 *
 * Order is stable across renders (trail-having satellites first, in insertion
 * order, then position-only ones) so palette colour assignment doesn't shift.
 */
export function buildRenderEntries(
  trailBuffers: Map<string, TrailBuffer> | undefined,
  satellitePositions: Map<string, OrbitPoint | null> | undefined,
): RenderEntry[] {
  const ids: string[] = [];
  const seen = new Set<string>();

  if (trailBuffers) {
    for (const [id, buf] of trailBuffers) {
      if (buf.length > 0 && !seen.has(id)) {
        seen.add(id);
        ids.push(id);
      }
    }
  }
  if (satellitePositions) {
    for (const [id, pos] of satellitePositions) {
      if (pos && !seen.has(id)) {
        seen.add(id);
        ids.push(id);
      }
    }
  }

  return ids.map((satId) => ({
    satId,
    buf: trailBuffers?.get(satId),
    pos: satellitePositions?.get(satId),
  }));
}
