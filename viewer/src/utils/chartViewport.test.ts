import { describe, it, expect } from "vitest";
import { sliceChartData, quantizeChartTime } from "./chartViewport.js";
import type { ChartData } from "../db/orbitStore.js";

function makeChartData(times: number[]): ChartData {
  const t = new Float64Array(times);
  const alt = new Float64Array(times.map((v) => v * 10));
  const energy = new Float64Array(times.map((v) => v * -1));
  const angMom = new Float64Array(times.map((v) => v * 100));
  const vel = new Float64Array(times.map((v) => v * 0.1));
  return [t, alt, energy, angMom, vel];
}

describe("sliceChartData", () => {
  it("returns null for null input", () => {
    expect(sliceChartData(null, undefined, null)).toBeNull();
  });

  it("returns data unchanged when no currentTime and no timeRange", () => {
    const data = makeChartData([0, 10, 20, 30]);
    const result = sliceChartData(data, undefined, null)!;
    expect(result[0].length).toBe(4);
    expect(result[0][0]).toBe(0);
    expect(result[0][3]).toBe(30);
  });

  it("slices right edge to currentTime", () => {
    const data = makeChartData([0, 10, 20, 30, 40, 50]);
    const result = sliceChartData(data, 25, null)!;
    // Should include points up to t=20 (last point with t <= 25)
    // Binary search finds index 3 (first with t > 25), so subarray(0, 3)
    expect(result[0].length).toBe(3);
    expect(result[0][0]).toBe(0);
    expect(result[0][2]).toBe(20);
    // Verify all columns are sliced consistently
    expect(result[1].length).toBe(3);
    expect(result[1][2]).toBe(200); // 20 * 10
  });

  it("slices right edge to exact match", () => {
    const data = makeChartData([0, 10, 20, 30]);
    const result = sliceChartData(data, 20, null)!;
    // t=20 is included (t <= 20)
    expect(result[0].length).toBe(3);
    expect(result[0][2]).toBe(20);
  });

  it("applies timeRange as left-edge window relative to currentTime", () => {
    const data = makeChartData([0, 10, 20, 30, 40, 50]);
    // currentTime=40, timeRange=15 → window [25, 40] → points at t=30, t=40
    const result = sliceChartData(data, 40, 15)!;
    expect(result[0].length).toBe(2);
    expect(result[0][0]).toBe(30);
    expect(result[0][1]).toBe(40);
  });

  it("applies timeRange without currentTime (uses last point as right edge)", () => {
    const data = makeChartData([0, 10, 20, 30, 40, 50]);
    // timeRange=25 → window [25, 50] → points at t=30, t=40, t=50
    const result = sliceChartData(data, undefined, 25)!;
    expect(result[0].length).toBe(3);
    expect(result[0][0]).toBe(30);
    expect(result[0][2]).toBe(50);
  });

  it("returns empty-like data when currentTime is before all points", () => {
    const data = makeChartData([10, 20, 30]);
    const result = sliceChartData(data, 5, null)!;
    expect(result[0].length).toBe(0);
  });

  it("handles empty chart data", () => {
    const data = makeChartData([]);
    const result = sliceChartData(data, 10, null)!;
    expect(result[0].length).toBe(0);
  });
});

function makeLargeChartData(n: number): ChartData {
  const t = new Float64Array(n);
  const alt = new Float64Array(n);
  const energy = new Float64Array(n);
  const angMom = new Float64Array(n);
  const vel = new Float64Array(n);
  for (let i = 0; i < n; i++) {
    const v = i * 0.1;
    t[i] = v;
    alt[i] = v * 10;
    energy[i] = v * -1;
    angMom[i] = v * 100;
    vel[i] = v * 0.1;
  }
  return [t, alt, energy, angMom, vel];
}

describe("sliceChartData performance", () => {
  it("slices 100k points in under 1ms", () => {
    const data = makeLargeChartData(100_000);
    const start = performance.now();
    for (let i = 0; i < 100; i++) {
      sliceChartData(data, 5000, 300);
    }
    const elapsed = (performance.now() - start) / 100;
    expect(elapsed).toBeLessThan(1);
  });

  it("handles 1000 consecutive scrub positions on 100k points in under 100ms total", () => {
    const data = makeLargeChartData(100_000);
    const start = performance.now();
    for (let i = 0; i < 1000; i++) {
      const t = (i / 1000) * 10000;
      sliceChartData(data, t, 300);
    }
    const elapsed = performance.now() - start;
    expect(elapsed).toBeLessThan(100);
  });

  it("returns subarrays sharing the original buffer (zero-copy)", () => {
    const data = makeChartData([0, 10, 20, 30, 40, 50]);
    const result = sliceChartData(data, 30, null)!;
    expect(result[0].buffer).toBe(data[0].buffer);
    expect(result[1].buffer).toBe(data[1].buffer);
  });
});

describe("quantizeChartTime", () => {
  it("returns undefined when input is undefined", () => {
    expect(quantizeChartTime(undefined)).toBeUndefined();
  });

  it("quantizes to 0.5s steps", () => {
    expect(quantizeChartTime(0.0)).toBe(0);
    expect(quantizeChartTime(0.24)).toBe(0);
    expect(quantizeChartTime(0.25)).toBe(0.5);
    expect(quantizeChartTime(0.49)).toBe(0.5);
    expect(quantizeChartTime(0.74)).toBe(0.5);
    expect(quantizeChartTime(0.75)).toBe(1.0);
    expect(quantizeChartTime(1.0)).toBe(1.0);
  });

  it("produces identical output for inputs within the same quantum", () => {
    const a = quantizeChartTime(5.1);
    const b = quantizeChartTime(5.2);
    const c = quantizeChartTime(5.24);
    expect(a).toBe(b);
    expect(b).toBe(c);
  });

  it("limits chart update frequency: 60fps over 10s produces <= 20 unique values", () => {
    // Simulate 60fps for 10s of sim time at 1x speed
    const uniqueValues = new Set<number>();
    for (let frame = 0; frame < 600; frame++) {
      const elapsed = frame / 60; // 60fps, 1x speed
      uniqueValues.add(quantizeChartTime(elapsed)!);
    }
    // With 0.5s quantization over 10s, expect ~20 unique values
    expect(uniqueValues.size).toBeLessThanOrEqual(21);
    expect(uniqueValues.size).toBeGreaterThan(0);
  });

  it("limits chart updates at 10x speed: 60fps for 100s produces <= 200 unique values", () => {
    const uniqueValues = new Set<number>();
    for (let frame = 0; frame < 600; frame++) {
      const elapsed = (frame / 60) * 10; // 60fps, 10x speed
      uniqueValues.add(quantizeChartTime(elapsed)!);
    }
    // 100s at 0.5s quanta = 200 unique values
    expect(uniqueValues.size).toBeLessThanOrEqual(201);
  });
});
