import * as THREE from "three";
import { describe, expect, it } from "vitest";
import { dimMaterials } from "./dimMaterials.js";

describe("dimMaterials", () => {
  it("dims a real AxesHelper's material", () => {
    // The reference-frame triad in the attitude scene: proving the flags land on
    // the helper's own material needs no renderer, and this is what a cast to a
    // single `Material` would get wrong if three ever built the axes from three.
    const helper = new THREE.AxesHelper(1);
    expect(dimMaterials(helper, 0.4)).toBe(1);
    const material = helper.material as THREE.Material;
    expect(material.transparent).toBe(true);
    expect(material.opacity).toBeCloseTo(0.4, 12);
  });

  it("dims every material when the object has an array of them", () => {
    const mesh = new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), [
      new THREE.MeshBasicMaterial(),
      new THREE.MeshBasicMaterial(),
      new THREE.MeshBasicMaterial(),
    ]);
    expect(dimMaterials(mesh, 0.25)).toBe(3);
    for (const m of mesh.material as THREE.Material[]) {
      expect(m.transparent).toBe(true);
      expect(m.opacity).toBeCloseTo(0.25, 12);
    }
  });

  it("does nothing for a null object or one without a material", () => {
    expect(dimMaterials(null, 0.5)).toBe(0);
    expect(dimMaterials(new THREE.Group(), 0.5)).toBe(0);
  });
});
