/**
 * Dev/E2E-only registry that publishes each rendered satellite's world quaternion
 * from the live Three.js scene graph, keyed by entity path.
 *
 * Mirrors `EarthBody`'s `window.__debug_get_earth_world_quat` (a single global) but
 * supports multiple satellites. E2E tests read `window.__debug_get_sat_world_quat(id)`
 * to assert the *rendered* attitude without reading pixels — see
 * viewer/tests/attitude-rendering.spec.ts. No-op outside dev builds.
 */
import type * as THREE from "three";
import { Quaternion } from "three";

type GroupGetter = () => THREE.Object3D | null;

interface DebugWindow extends Record<string, unknown> {
  __debug_sat_quat_registry?: Map<string, GroupGetter>;
  __debug_get_sat_world_quat?: (id: string) => [number, number, number, number] | null;
}

function ensureRegistry(): Map<string, GroupGetter> {
  const w = window as unknown as DebugWindow;
  let reg = w.__debug_sat_quat_registry;
  if (!reg) {
    reg = new Map();
    w.__debug_sat_quat_registry = reg;
    // Three.js (x, y, z, w) order, matching __debug_get_earth_world_quat.
    w.__debug_get_sat_world_quat = (id) => {
      const group = w.__debug_sat_quat_registry?.get(id)?.();
      if (!group) return null;
      const q = group.getWorldQuaternion(new Quaternion());
      return [q.x, q.y, q.z, q.w];
    };
  }
  return reg;
}

/**
 * Register a satellite group so its world quaternion is queryable by `id`.
 * Returns a cleanup function; a no-op (returning a no-op) outside dev builds.
 */
export function registerSatWorldQuat(id: string, getGroup: GroupGetter): () => void {
  if (!import.meta.env.DEV) return () => {};
  const reg = ensureRegistry();
  reg.set(id, getGroup);
  return () => {
    reg.delete(id);
  };
}
