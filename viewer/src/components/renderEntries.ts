import type { OrbitPoint } from "../orbit.js";
import type { TrailBufferLike } from "../utils/TrailBuffer.js";

/** A satellite to render: its trail buffer (if any) and current position (if any). */
export interface RenderEntry {
  satId: string;
  buf: TrailBufferLike | undefined;
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
 * Order: trail-having satellites first (in insertion order), then position-only
 * ones. This is stable while each satellite's trail-status is stable; a satellite
 * that gains or loses a trail moves between the two groups, which can shift the
 * default palette-colour index of later satellites. Colours are cosmetic and such
 * transitions are rare — pass an explicit `color` if you need guaranteed stability.
 */
export function buildRenderEntries(
  trailBuffers: Map<string, TrailBufferLike> | undefined,
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
