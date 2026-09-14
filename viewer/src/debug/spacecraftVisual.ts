/**
 * Dev/E2E-only record of which visual each spacecraft's component selected: its
 * registered model, or the marker that stands in when there is none.
 *
 * The selection, not the mesh on screen — the model mounts inside a `Suspense`
 * whose fallback is the same marker, so a "model" entry can be a frame ahead of
 * the geometry. What it answers is which branch the component took, which is
 * where a supplied attitude state either arrives or does not. A test that needs
 * the drawn mesh reads the scene graph instead.
 *
 * E2E tests read `window.__debug_spacecraft_visual.get(id)`. The scene's
 * amplification is derived separately from the same facts, so reading that
 * cannot tell a selected model from a marker standing in for one — a test
 * written against the amplification passed with the forwarding it was meant to
 * cover reverted (measured). No-op outside dev builds.
 */
import { IS_DEV } from "../env.js";

/** Which visual a spacecraft's component selected. */
export type SpacecraftVisualChoice = "model" | "marker";

interface DebugWindow extends Record<string, unknown> {
  __debug_spacecraft_visual?: Map<string, SpacecraftVisualChoice>;
}

function ensureRegistry(): Map<string, SpacecraftVisualChoice> {
  const w = window as unknown as DebugWindow;
  let selected = w.__debug_spacecraft_visual;
  if (!selected) {
    selected = new Map();
    w.__debug_spacecraft_visual = selected;
  }
  return selected;
}

/**
 * Record that `id`'s component selected `choice`.
 * Returns a cleanup function; a no-op (returning a no-op) outside dev builds.
 */
export function recordSpacecraftVisual(id: string, choice: SpacecraftVisualChoice): () => void {
  if (!IS_DEV) return () => {};
  const selected = ensureRegistry();
  selected.set(id, choice);
  return () => {
    selected.delete(id);
  };
}
