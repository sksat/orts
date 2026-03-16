import { describe, expect, it } from "vitest";
import { rotateZ } from "./frameTransform.js";

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
