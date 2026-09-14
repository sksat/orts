/**
 * Dev/E2E-only record of which of the two things each spacecraft was drawn as:
 * its registered model, or the marker that stands in when there is none.
 *
 * E2E tests read `window.__debug_spacecraft_drawn.get(id)`. The scene's
 * amplification is derived separately from the same facts, so reading it cannot
 * tell a drawn model from a marker standing in for one — a test written against
 * the amplification passed with the forwarding it was meant to cover reverted
 * (measured). No-op outside dev builds.
 */
import { IS_DEV } from "../env.js";

/** What a spacecraft was drawn as. */
export type DrawnAs = "model" | "marker";

interface DebugWindow extends Record<string, unknown> {
  __debug_spacecraft_drawn?: Map<string, DrawnAs>;
}

function ensureRegistry(): Map<string, DrawnAs> {
  const w = window as unknown as DebugWindow;
  let drawn = w.__debug_spacecraft_drawn;
  if (!drawn) {
    drawn = new Map();
    w.__debug_spacecraft_drawn = drawn;
  }
  return drawn;
}

/**
 * Record that `id` was drawn as `as`.
 * Returns a cleanup function; a no-op (returning a no-op) outside dev builds.
 */
export function recordSpacecraftDrawn(id: string, as: DrawnAs): () => void {
  if (!IS_DEV) return () => {};
  const drawn = ensureRegistry();
  drawn.set(id, as);
  return () => {
    drawn.delete(id);
  };
}
