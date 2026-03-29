import { describe, expect, it } from "vitest";
import {
  batchEncodeEciHighLow,
  batchTransformToLvlh,
  batchTransformWithOffset,
  encodeFloat64ToHighLow,
  transformToLvlh,
} from "./coordTransform.js";
import type { OrbitPoint } from "./orbit.js";
import type { LvlhAxes } from "./sceneFrame.js";

function makePoint(t: number, x: number, y: number, z: number): OrbitPoint {
  return { t, x, y, z, vx: 0, vy: 0, vz: 0, a: 0, e: 0, inc: 0, raan: 0, omega: 0, nu: 0 };
}

describe("batchTransformWithOffset", () => {
  it("scales positions by 1/scaleRadius when no origin offset", () => {
    const points = [makePoint(0, 6378.137, 0, 0), makePoint(10, 0, 6378.137, 0)];
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
    const points = [makePoint(0, 7000, 1000, 500), makePoint(10, 7100, 1100, 600)];
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
    const points = [makePoint(0, 1000, 0, 0), makePoint(10, 2000, 0, 0), makePoint(20, 3000, 0, 0)];
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

// LVLH axes for equatorial circular orbit: satellite at +X, velocity +Y
// radial = +X, inTrack = +Y, crossTrack = +Z
const equatorialAxes: LvlhAxes = {
  radial: [1, 0, 0],
  inTrack: [0, 1, 0],
  crossTrack: [0, 0, 1],
};

// LVLH axes for polar orbit: satellite at +X, velocity +Z
// radial = +X, crossTrack = -Y (r×v = X×Z = -Y), inTrack = C×R = (-Y)×X = +Z
const polarAxes: LvlhAxes = {
  radial: [1, 0, 0],
  inTrack: [0, 0, 1],
  crossTrack: [0, -1, 0],
};

type Vec3 = [number, number, number];
function dot(a: Vec3, b: Vec3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
}
function mag(v: Vec3): number {
  return Math.sqrt(dot(v, v));
}

describe("transformToLvlh", () => {
  it("returns [0, 0, 0] for a point at the satellite position", () => {
    const origin: Vec3 = [7000, 0, 0];
    const result = transformToLvlh(7000, 0, 0, origin, equatorialAxes, 6378.137);
    expect(result[0]).toBeCloseTo(0, 10);
    expect(result[1]).toBeCloseTo(0, 10);
    expect(result[2]).toBeCloseTo(0, 10);
  });

  it("maps point radially outward to +Z (radial axis)", () => {
    const origin: Vec3 = [7000, 0, 0];
    // Point 100 km further from center in +X (radial direction)
    const result = transformToLvlh(7100, 0, 0, origin, equatorialAxes, 6378.137);
    // dp = [100, 0, 0], dot(inTrack=[0,1,0], dp) = 0, dot(cross=[0,0,1], dp) = 0, dot(radial=[1,0,0], dp) = 100
    expect(result[0]).toBeCloseTo(0, 10); // X = inTrack
    expect(result[1]).toBeCloseTo(0, 10); // Y = crossTrack
    expect(result[2]).toBeCloseTo(100 / 6378.137, 5); // Z = radial
  });

  it("maps point in velocity direction to +X (inTrack axis)", () => {
    const origin: Vec3 = [7000, 0, 0];
    // Point 100 km ahead in +Y (inTrack direction for equatorial orbit at +X)
    const result = transformToLvlh(7000, 100, 0, origin, equatorialAxes, 6378.137);
    expect(result[0]).toBeCloseTo(100 / 6378.137, 5); // X = inTrack
    expect(result[1]).toBeCloseTo(0, 10); // Y = crossTrack
    expect(result[2]).toBeCloseTo(0, 10); // Z = radial
  });

  it("maps point in orbit-normal direction to +Y (crossTrack axis)", () => {
    const origin: Vec3 = [7000, 0, 0];
    // Point 100 km in +Z (crossTrack for equatorial orbit)
    const result = transformToLvlh(7000, 0, 100, origin, equatorialAxes, 6378.137);
    expect(result[0]).toBeCloseTo(0, 10); // X = inTrack
    expect(result[1]).toBeCloseTo(100 / 6378.137, 5); // Y = crossTrack
    expect(result[2]).toBeCloseTo(0, 10); // Z = radial
  });

  it("maps central body (origin of ECI) to -Z direction", () => {
    const origin: Vec3 = [7000, 0, 0];
    // Central body at [0, 0, 0]
    const result = transformToLvlh(0, 0, 0, origin, equatorialAxes, 6378.137);
    // dp = [-7000, 0, 0], radial = [1,0,0] → dot = -7000
    expect(result[0]).toBeCloseTo(0, 10);
    expect(result[1]).toBeCloseTo(0, 10);
    expect(result[2]).toBeCloseTo(-7000 / 6378.137, 5);
  });

  it("preserves distance (isometric transformation)", () => {
    const origin: Vec3 = [7000, 0, 0];
    const px = 7050,
      py = 30,
      pz = -20;
    const result = transformToLvlh(px, py, pz, origin, equatorialAxes, 1.0);

    const dp: Vec3 = [px - origin[0], py - origin[1], pz - origin[2]];
    const expectedMag = mag(dp);
    const resultMag = mag(result);
    expect(resultMag).toBeCloseTo(expectedMag, 8);
  });

  it("works correctly for polar orbit axes", () => {
    const origin: Vec3 = [7000, 0, 0];
    // Point 100 km in +Z (velocity direction for polar orbit at +X)
    const result = transformToLvlh(7000, 0, 100, origin, polarAxes, 1.0);
    // dp = [0, 0, 100]
    // inTrack = [0, 0, 1] → dot = 100
    // crossTrack = [0, -1, 0] → dot = 0
    // radial = [1, 0, 0] → dot = 0
    expect(result[0]).toBeCloseTo(100, 10); // X = inTrack
    expect(result[1]).toBeCloseTo(0, 10); // Y = crossTrack
    expect(result[2]).toBeCloseTo(0, 10); // Z = radial
  });

  it("handles arbitrary LVLH axes with off-diagonal components", () => {
    // Satellite at 45° in equatorial plane: r = [R/√2, R/√2, 0]
    const s = Math.SQRT2 / 2;
    const axes: LvlhAxes = {
      radial: [s, s, 0],
      inTrack: [-s, s, 0],
      crossTrack: [0, 0, 1],
    };
    const origin: Vec3 = [7000 * s, 7000 * s, 0];
    // Point 100 km radially outward
    const px = (7000 + 100) * s,
      py = (7000 + 100) * s,
      pz = 0;
    const result = transformToLvlh(px, py, pz, origin, axes, 1.0);

    expect(result[0]).toBeCloseTo(0, 5); // X = inTrack (no along-track component)
    expect(result[1]).toBeCloseTo(0, 5); // Y = crossTrack
    expect(result[2]).toBeCloseTo(100, 5); // Z = radial (100 km outward)
  });
});

describe("batchTransformToLvlh", () => {
  it("transforms multiple points correctly", () => {
    const origin: Vec3 = [7000, 0, 0];
    const points = [
      makePoint(0, 7100, 0, 0), // 100 km radially outward
      makePoint(10, 7000, 100, 0), // 100 km in inTrack
      makePoint(20, 7000, 0, 100), // 100 km in crossTrack
    ];
    const outBuf = new Float32Array(9);
    batchTransformToLvlh(points, 0, 3, origin, equatorialAxes, outBuf, 0, 6378.137);

    const s = 100 / 6378.137;
    // Point 0: radial +100 → [0, 0, s]
    expect(outBuf[0]).toBeCloseTo(0, 5);
    expect(outBuf[1]).toBeCloseTo(0, 5);
    expect(outBuf[2]).toBeCloseTo(s, 5);
    // Point 1: inTrack +100 → [s, 0, 0]
    expect(outBuf[3]).toBeCloseTo(s, 5);
    expect(outBuf[4]).toBeCloseTo(0, 5);
    expect(outBuf[5]).toBeCloseTo(0, 5);
    // Point 2: crossTrack +100 → [0, s, 0]
    expect(outBuf[6]).toBeCloseTo(0, 5);
    expect(outBuf[7]).toBeCloseTo(s, 5);
    expect(outBuf[8]).toBeCloseTo(0, 5);
  });

  it("respects from/to range", () => {
    const origin: Vec3 = [7000, 0, 0];
    const points = [
      makePoint(0, 7000, 0, 0), // at origin
      makePoint(10, 7100, 0, 0), // 100 km radial
      makePoint(20, 7000, 200, 0), // 200 km inTrack
    ];
    const outBuf = new Float32Array(9);
    batchTransformToLvlh(points, 1, 3, origin, equatorialAxes, outBuf, 0, 1.0);

    // Only points[1] and points[2] written at outOffset 0
    // Point 1: [0, 0, 100]
    expect(outBuf[0]).toBeCloseTo(0, 5);
    expect(outBuf[1]).toBeCloseTo(0, 5);
    expect(outBuf[2]).toBeCloseTo(100, 5);
    // Point 2: [200, 0, 0]
    expect(outBuf[3]).toBeCloseTo(200, 5);
    expect(outBuf[4]).toBeCloseTo(0, 5);
    expect(outBuf[5]).toBeCloseTo(0, 5);
  });

  it("respects outOffset", () => {
    const origin: Vec3 = [7000, 0, 0];
    const points = [makePoint(0, 7100, 0, 0)];
    const outBuf = new Float32Array(12);
    batchTransformToLvlh(points, 0, 1, origin, equatorialAxes, outBuf, 2, 1.0);

    // Written at vertex index 2 → buffer offset 6
    expect(outBuf[6]).toBeCloseTo(0, 5);
    expect(outBuf[7]).toBeCloseTo(0, 5);
    expect(outBuf[8]).toBeCloseTo(100, 5);
    // Original slots untouched
    expect(outBuf[0]).toBe(0.0);
  });

  it("matches transformToLvlh results", () => {
    const origin: Vec3 = [7000, 0, 0];
    const point = makePoint(0, 7050, 30, -20);
    const single = transformToLvlh(point.x, point.y, point.z, origin, equatorialAxes, 6378.137);

    const outBuf = new Float32Array(3);
    batchTransformToLvlh([point], 0, 1, origin, equatorialAxes, outBuf, 0, 6378.137);

    expect(outBuf[0]).toBeCloseTo(single[0], 5);
    expect(outBuf[1]).toBeCloseTo(single[1], 5);
    expect(outBuf[2]).toBeCloseTo(single[2], 5);
  });
});

// --- High/Low split encoding tests ---

describe("encodeFloat64ToHighLow", () => {
  it("round-trips a small value", () => {
    const [h, l] = encodeFloat64ToHighLow(1.5);
    expect(h + l).toBeCloseTo(1.5, 14);
  });

  it("round-trips a large orbital position (LEO ~7000 km)", () => {
    const value = 7000.123456789;
    const [h, l] = encodeFloat64ToHighLow(value);
    expect(h + l).toBeCloseTo(value, 10);
    expect(h % 65536).toBe(0);
    expect(Math.abs(l)).toBeLessThan(65536);
  });

  it("round-trips a GEO distance (~42164 km)", () => {
    const value = 42164.5;
    const [h, l] = encodeFloat64ToHighLow(value);
    expect(h + l).toBeCloseTo(value, 10);
  });

  it("round-trips negative values", () => {
    const value = -4231.987654321;
    const [h, l] = encodeFloat64ToHighLow(value);
    expect(h + l).toBeCloseTo(value, 10);
  });

  it("handles zero", () => {
    const [h, l] = encodeFloat64ToHighLow(0);
    expect(h).toBe(0);
    expect(l).toBe(0);
  });

  it("preserves relative precision for nearby GEO points", () => {
    const a = 42164.5;
    const b = 42165.5;
    const [ah, al] = encodeFloat64ToHighLow(a);
    const [bh, bl] = encodeFloat64ToHighLow(b);
    // Simulate shader subtraction: (bh - ah) + (bl - al)
    const diff = (bh - ah) + (bl - al);
    expect(diff).toBeCloseTo(1.0, 6);
  });

  it("preserves relative precision at lunar distance (~384400 km)", () => {
    const a = 384400.0;
    const b = 384401.0;
    const [ah, al] = encodeFloat64ToHighLow(a);
    const [bh, bl] = encodeFloat64ToHighLow(b);
    const diff = (bh - ah) + (bl - al);
    expect(diff).toBeCloseTo(1.0, 4);
  });
});

describe("batchEncodeEciHighLow", () => {
  it("encodes points into dual buffers that reconstruct original values", () => {
    const points = [
      makePoint(0, 7000.123, -4231.456, 1500.789),
      makePoint(10, -2000.321, 6500.654, -800.987),
    ];
    const highBuf = new Float32Array(6);
    const lowBuf = new Float32Array(6);
    batchEncodeEciHighLow(points, 0, 2, highBuf, lowBuf, 0);

    for (let i = 0; i < 2; i++) {
      const p = points[i];
      const off = i * 3;
      // f64 reconstruction: high + low ≈ original (sub-meter precision at ~7000 km)
      expect(highBuf[off] + lowBuf[off]).toBeCloseTo(p.x, 3);
      expect(highBuf[off + 1] + lowBuf[off + 1]).toBeCloseTo(p.y, 3);
      expect(highBuf[off + 2] + lowBuf[off + 2]).toBeCloseTo(p.z, 3);
    }
  });

  it("respects from/to range and outOffset", () => {
    const points = [
      makePoint(0, 1000, 2000, 3000),
      makePoint(10, 4000, 5000, 6000),
      makePoint(20, 7000, 8000, 9000),
    ];
    const highBuf = new Float32Array(12);
    const lowBuf = new Float32Array(12);
    batchEncodeEciHighLow(points, 1, 3, highBuf, lowBuf, 1);

    // points[1] written at vertex 1 (offset 3)
    expect(highBuf[3] + lowBuf[3]).toBeCloseTo(4000, 6);
    // vertex 0 should be untouched
    expect(highBuf[0]).toBe(0);
    expect(lowBuf[0]).toBe(0);
  });

  it("simulated shader subtraction gives correct relative position", () => {
    const sat = makePoint(0, 7000, 0, 0);
    const trail = makePoint(10, 7100, 50, -30);
    const points = [sat, trail];
    const highBuf = new Float32Array(6);
    const lowBuf = new Float32Array(6);
    batchEncodeEciHighLow(points, 0, 2, highBuf, lowBuf, 0);

    // Simulate shader: (trailHigh - satHigh) + (trailLow - satLow)
    const dx = (highBuf[3] - highBuf[0]) + (lowBuf[3] - lowBuf[0]);
    const dy = (highBuf[4] - highBuf[1]) + (lowBuf[4] - lowBuf[1]);
    const dz = (highBuf[5] - highBuf[2]) + (lowBuf[5] - lowBuf[2]);

    expect(dx).toBeCloseTo(100, 4);
    expect(dy).toBeCloseTo(50, 4);
    expect(dz).toBeCloseTo(-30, 4);
  });
});
