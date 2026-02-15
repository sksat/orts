import { describe, it, expect } from "vitest";
import { batchTransformWithOffset } from "./coordTransform.js";
import type { OrbitPoint } from "./orbit.js";

function makePoint(t: number, x: number, y: number, z: number): OrbitPoint {
  return { t, x, y, z, vx: 0, vy: 0, vz: 0, a: 0, e: 0, inc: 0, raan: 0, omega: 0, nu: 0 };
}

describe("batchTransformWithOffset", () => {
  it("scales positions by 1/scaleRadius when no origin offset", () => {
    const points = [
      makePoint(0, 6378.137, 0, 0),
      makePoint(10, 0, 6378.137, 0),
    ];
    const outBuf = new Float32Array(6);
    batchTransformWithOffset(points, 0, 2, null, outBuf, 0, 6378.137);

    expect(outBuf[0]).toBeCloseTo(1.0);
    expect(outBuf[1]).toBeCloseTo(0.0);
    expect(outBuf[2]).toBeCloseTo(0.0);
    expect(outBuf[3]).toBeCloseTo(0.0);
    expect(outBuf[4]).toBeCloseTo(1.0);
    expect(outBuf[5]).toBeCloseTo(0.0);
  });

  it("subtracts origin position before scaling", () => {
    const points = [
      makePoint(0, 7000, 1000, 500),
      makePoint(10, 7100, 1100, 600),
    ];
    const origin: [number, number, number] = [6500, 800, 400];
    const outBuf = new Float32Array(6);
    const scale = 1000;
    batchTransformWithOffset(points, 0, 2, origin, outBuf, 0, scale);

    // Point 0: (7000-6500, 1000-800, 500-400) / 1000 = (0.5, 0.2, 0.1)
    expect(outBuf[0]).toBeCloseTo(0.5);
    expect(outBuf[1]).toBeCloseTo(0.2);
    expect(outBuf[2]).toBeCloseTo(0.1);
    // Point 1: (7100-6500, 1100-800, 600-400) / 1000 = (0.6, 0.3, 0.2)
    expect(outBuf[3]).toBeCloseTo(0.6);
    expect(outBuf[4]).toBeCloseTo(0.3);
    expect(outBuf[5]).toBeCloseTo(0.2);
  });

  it("respects from/to range", () => {
    const points = [
      makePoint(0, 1000, 0, 0),
      makePoint(10, 2000, 0, 0),
      makePoint(20, 3000, 0, 0),
    ];
    const outBuf = new Float32Array(9);
    batchTransformWithOffset(points, 1, 3, null, outBuf, 0, 1000);

    // Only points[1] and points[2] should be written, starting at outOffset 0
    expect(outBuf[0]).toBeCloseTo(2.0);
    expect(outBuf[3]).toBeCloseTo(3.0);
  });

  it("respects outOffset", () => {
    const points = [makePoint(0, 5000, 0, 0)];
    const outBuf = new Float32Array(12);
    batchTransformWithOffset(points, 0, 1, null, outBuf, 2, 1000);

    // Written at vertex index 2 → buffer offset 6
    expect(outBuf[6]).toBeCloseTo(5.0);
    expect(outBuf[7]).toBeCloseTo(0.0);
    expect(outBuf[8]).toBeCloseTo(0.0);
    // Original slots should be untouched
    expect(outBuf[0]).toBe(0.0);
  });

  it("handles null origin the same as no offset", () => {
    const points = [makePoint(0, 1000, 2000, 3000)];
    const outBuf = new Float32Array(3);
    batchTransformWithOffset(points, 0, 1, null, outBuf, 0, 1000);

    expect(outBuf[0]).toBeCloseTo(1.0);
    expect(outBuf[1]).toBeCloseTo(2.0);
    expect(outBuf[2]).toBeCloseTo(3.0);
  });
});
