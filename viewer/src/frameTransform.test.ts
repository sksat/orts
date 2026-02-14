import { describe, it, expect } from "vitest";
import { rotateZ, eciToEcef } from "./frameTransform.js";

const TAU = 2 * Math.PI;

describe("rotateZ", () => {
  it("identity when angle=0", () => {
    const [x, y, z] = rotateZ(1, 0, 0, 0);
    expect(x).toBeCloseTo(1);
    expect(y).toBeCloseTo(0);
    expect(z).toBeCloseTo(0);
  });

  it("rotates (1,0,0) by π/2 to (0,1,0)", () => {
    // Counter-clockwise rotation of +X by 90° → +Y
    const [x, y, z] = rotateZ(1, 0, 0, Math.PI / 2);
    expect(x).toBeCloseTo(0);
    expect(y).toBeCloseTo(1);
    expect(z).toBeCloseTo(0);
  });

  it("rotates (1,0,0) by π to (-1,0,0)", () => {
    const [x, y, z] = rotateZ(1, 0, 0, Math.PI);
    expect(x).toBeCloseTo(-1);
    expect(y).toBeCloseTo(0);
    expect(z).toBeCloseTo(0);
  });

  it("rotates (0,1,0) by π/2 to (-1,0,0)", () => {
    const [x, y, z] = rotateZ(0, 1, 0, Math.PI / 2);
    expect(x).toBeCloseTo(-1);
    expect(y).toBeCloseTo(0);
    expect(z).toBeCloseTo(0);
  });

  it("preserves Z component", () => {
    for (const angle of [0, Math.PI / 4, Math.PI / 2, Math.PI, 3.7]) {
      const [, , z] = rotateZ(3, 4, 5, angle);
      expect(z).toBeCloseTo(5);
    }
  });

  it("preserves vector magnitude", () => {
    const original = [3, 4, 5] as const;
    const origMag = Math.sqrt(3 * 3 + 4 * 4 + 5 * 5);

    for (const angle of [0, 0.1, Math.PI / 3, Math.PI, TAU * 0.7]) {
      const [x, y, z] = rotateZ(...original, angle);
      const mag = Math.sqrt(x * x + y * y + z * z);
      expect(mag).toBeCloseTo(origMag);
    }
  });

  it("roundtrip: rotateZ(θ) then rotateZ(-θ) returns original", () => {
    const original: [number, number, number] = [6778, 1234, -500];
    const angle = 1.23;

    const rotated = rotateZ(...original, angle);
    const [x, y, z] = rotateZ(...rotated, -angle);

    expect(x).toBeCloseTo(original[0], 6);
    expect(y).toBeCloseTo(original[1], 6);
    expect(z).toBeCloseTo(original[2], 6);
  });

  it("full rotation (2π) returns original", () => {
    const [x, y, z] = rotateZ(3, 4, 5, TAU);
    expect(x).toBeCloseTo(3);
    expect(y).toBeCloseTo(4);
    expect(z).toBeCloseTo(5);
  });
});

describe("eciToEcef", () => {
  it("identity when ERA=0", () => {
    const [x, y, z] = eciToEcef(6778, 1234, 500, 0);
    expect(x).toBeCloseTo(6778);
    expect(y).toBeCloseTo(1234);
    expect(z).toBeCloseTo(500);
  });

  it("transforms (1,0,0) to (-1,0,0) at ERA=π", () => {
    // ERA=π means Earth has rotated 180°.
    // ECI→ECEF = R_z(-ERA) = R_z(-π)
    // (1,0,0) rotated by -π → (-1,0,0)
    const [x, y, z] = eciToEcef(1, 0, 0, Math.PI);
    expect(x).toBeCloseTo(-1);
    expect(y).toBeCloseTo(0);
    expect(z).toBeCloseTo(0);
  });

  it("transforms (1,0,0) to (0,-1,0) at ERA=π/2", () => {
    // ERA=π/2: Earth rotated 90° eastward.
    // R_z(-π/2) * (1,0,0) = (cos(-π/2), sin(-π/2), 0) = (0,-1,0)
    const [x, y, z] = eciToEcef(1, 0, 0, Math.PI / 2);
    expect(x).toBeCloseTo(0);
    expect(y).toBeCloseTo(-1);
    expect(z).toBeCloseTo(0);
  });

  it("preserves altitude (distance from origin)", () => {
    const r = 6778;
    for (const era of [0, Math.PI / 4, Math.PI / 2, Math.PI, 5.0]) {
      const [x, y, z] = eciToEcef(r, 0, 0, era);
      const dist = Math.sqrt(x * x + y * y + z * z);
      expect(dist).toBeCloseTo(r);
    }
  });

  it("preserves Z component (latitude invariant)", () => {
    for (const era of [0, Math.PI / 3, Math.PI, 4.5]) {
      const [, , z] = eciToEcef(4000, 3000, 5000, era);
      expect(z).toBeCloseTo(5000);
    }
  });

  it("is the inverse of ECEF→ECI (rotateZ with +ERA)", () => {
    const eciPos: [number, number, number] = [6778, 1234, 500];
    const era = 2.5;

    // ECI → ECEF
    const ecef = eciToEcef(...eciPos, era);
    // ECEF → ECI: rotateZ with +ERA
    const [x, y, z] = rotateZ(...ecef, era);

    expect(x).toBeCloseTo(eciPos[0], 6);
    expect(y).toBeCloseTo(eciPos[1], 6);
    expect(z).toBeCloseTo(eciPos[2], 6);
  });

  it("consistency: eciToEcef is rotateZ with -ERA", () => {
    const pos: [number, number, number] = [1000, 2000, 3000];
    const era = 1.7;

    const fromEciToEcef = eciToEcef(...pos, era);
    const fromRotateZ = rotateZ(...pos, -era);

    expect(fromEciToEcef[0]).toBeCloseTo(fromRotateZ[0]);
    expect(fromEciToEcef[1]).toBeCloseTo(fromRotateZ[1]);
    expect(fromEciToEcef[2]).toBeCloseTo(fromRotateZ[2]);
  });
});
