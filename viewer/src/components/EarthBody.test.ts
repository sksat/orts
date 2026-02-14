import { describe, it, expect } from "vitest";
import * as THREE from "three";
import { POLE_ALIGNMENT_ROTATION } from "./EarthBody.js";

describe("Earth pole alignment (Y-pole → ECI Z-pole)", () => {
  const euler = new THREE.Euler(...POLE_ALIGNMENT_ROTATION);

  it("maps north pole (local +Y) to ECI +Z", () => {
    const northPole = new THREE.Vector3(0, 1, 0).applyEuler(euler);
    expect(northPole.x).toBeCloseTo(0);
    expect(northPole.y).toBeCloseTo(0);
    expect(northPole.z).toBeCloseTo(1);
  });

  it("maps south pole (local -Y) to ECI -Z", () => {
    const southPole = new THREE.Vector3(0, -1, 0).applyEuler(euler);
    expect(southPole.x).toBeCloseTo(0);
    expect(southPole.y).toBeCloseTo(0);
    expect(southPole.z).toBeCloseTo(-1);
  });

  it("preserves equator in the XY plane (local +X stays +X)", () => {
    const equatorX = new THREE.Vector3(1, 0, 0).applyEuler(euler);
    expect(equatorX.x).toBeCloseTo(1);
    expect(equatorX.y).toBeCloseTo(0);
    expect(equatorX.z).toBeCloseTo(0);
  });
});

describe("ERA rotation axis (Z = ECI north pole)", () => {
  it("rotates equatorial points around Z, keeping poles fixed", () => {
    const era = Math.PI / 4; // 45 degrees
    const eraEuler = new THREE.Euler(0, 0, era);

    // North pole (+Z) should be unchanged by Z-rotation
    const pole = new THREE.Vector3(0, 0, 1).applyEuler(eraEuler);
    expect(pole.x).toBeCloseTo(0);
    expect(pole.y).toBeCloseTo(0);
    expect(pole.z).toBeCloseTo(1);

    // Equatorial point +X should rotate in the XY plane
    const eq = new THREE.Vector3(1, 0, 0).applyEuler(eraEuler);
    expect(eq.x).toBeCloseTo(Math.cos(era));
    expect(eq.y).toBeCloseTo(Math.sin(era));
    expect(eq.z).toBeCloseTo(0);
  });
});

describe("Combined transform (pole alignment + ERA)", () => {
  const poleEuler = new THREE.Euler(...POLE_ALIGNMENT_ROTATION);

  it("north pole remains on +Z regardless of ERA", () => {
    for (const era of [0, Math.PI / 3, Math.PI, 5.0]) {
      // Simulate nested group transforms: inner (pole alignment) then outer (ERA)
      const northPole = new THREE.Vector3(0, 1, 0);
      northPole.applyEuler(poleEuler); // inner group
      northPole.applyEuler(new THREE.Euler(0, 0, era)); // outer group
      expect(northPole.x).toBeCloseTo(0, 5);
      expect(northPole.y).toBeCloseTo(0, 5);
      expect(northPole.z).toBeCloseTo(1, 5);
    }
  });

  it("equatorial point rotates in XY plane with ERA", () => {
    const era = Math.PI / 6; // 30 degrees
    // Local +X (equator) after pole alignment stays at +X
    const eq = new THREE.Vector3(1, 0, 0);
    eq.applyEuler(poleEuler);
    eq.applyEuler(new THREE.Euler(0, 0, era));
    expect(eq.x).toBeCloseTo(Math.cos(era));
    expect(eq.y).toBeCloseTo(Math.sin(era));
    expect(eq.z).toBeCloseTo(0);
  });
});
