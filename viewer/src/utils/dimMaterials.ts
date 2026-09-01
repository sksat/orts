import type * as THREE from "three";

/**
 * Make an object's material(s) translucent, in place.
 *
 * `Object3D.material` is either one material or an array of them, and which one
 * a helper hands back is the helper's business rather than the caller's — three's
 * `AxesHelper` builds its three axes from a single vertex-coloured material
 * today, and a caller that assumed so would break silently if that changed.
 * Handles both shapes and reports how many materials it touched, so the effect is
 * testable without a renderer.
 */
export function dimMaterials(object: THREE.Object3D | null, opacity: number): number {
  const material = (object as { material?: THREE.Material | THREE.Material[] } | null)?.material;
  if (material == null) return 0;
  const materials = Array.isArray(material) ? material : [material];
  for (const m of materials) {
    m.transparent = true;
    m.opacity = opacity;
  }
  return materials.length;
}
