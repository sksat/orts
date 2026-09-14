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
