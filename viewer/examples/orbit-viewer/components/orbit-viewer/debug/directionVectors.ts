import type * as THREE from "three";
import { Vector3 } from "three";
import { IS_DEV } from "../env.js";

/** One drawn arrow, as an E2E test sees it. */
export interface DebugDirectionVector {
  kind: string;
  /** Unit vector from the arrow's origin to its head, in world (scene) axes. */
  direction: [number, number, number];
  /**
   * World distance from the arrow's origin to its head's centre [scene units].
   *
   * How far the arrow reaches, which moves with the radius its tail starts at —
   * the marker's, or the model's when one is drawn. Reported unnormalised so a
   * test can pin the proportions as well: it is `startOffset + length -
   * headLength / 2` for the spacecraft's apparent size. Measured from the scene
   * graph like the direction, so a test comparing two scenes reads the geometry
   * rather than the number that was passed in.
   */
  distance: number;
}

/** The scene objects an arrow's drawn direction is measured between. */
export interface DrawnArrow {
  kind: string;
  origin: THREE.Object3D | null;
  head: THREE.Object3D | null;
}

type ArrowsGetter = () => DrawnArrow[];

interface DebugWindow extends Record<string, unknown> {
  __debug_direction_vector_registry?: Map<string, ArrowsGetter>;
  __debug_get_direction_vectors?: (id: string) => DebugDirectionVector[] | null;
}

function measure(arrows: DrawnArrow[]): DebugDirectionVector[] {
  const out: DebugDirectionVector[] = [];
  const from = new Vector3();
  const to = new Vector3();
  for (const arrow of arrows) {
    if (arrow.origin == null || arrow.head == null) continue;
    arrow.origin.getWorldPosition(from);
    arrow.head.getWorldPosition(to);
    const d = to.sub(from);
    const len = d.length();
    if (!(len > 0)) continue;
    d.divideScalar(len);
    out.push({ kind: arrow.kind, direction: [d.x, d.y, d.z], distance: len });
  }
  return out;
}

function ensureRegistry(): Map<string, ArrowsGetter> {
  const w = window as unknown as DebugWindow;
  let reg = w.__debug_direction_vector_registry;
  if (!reg) {
    reg = new Map();
    w.__debug_direction_vector_registry = reg;
    w.__debug_get_direction_vectors = (id) => {
      const arrows = w.__debug_direction_vector_registry?.get(id)?.();
      return arrows == null ? null : measure(arrows);
    };
  }
  return reg;
}

/**
 * Register the arrows drawn at `id` so their rendered directions are queryable.
 * Returns a cleanup function; a no-op (returning a no-op) outside dev builds.
 */
export function registerDirectionVectors(id: string, getArrows: ArrowsGetter): () => void {
  if (!IS_DEV) return () => {};
  const reg = ensureRegistry();
  reg.set(id, getArrows);
  return () => {
    reg.delete(id);
  };
}
