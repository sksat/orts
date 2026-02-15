import { describe, it, expect } from "vitest";
import { computeReplayDrawStart } from "./trailDrawStart.js";
import type { OrbitPoint } from "../orbit.js";

function makePoint(t: number): OrbitPoint {
  return { t, x: 6778 + t, y: t * 0.1, z: 0, vx: 0, vy: 7.669, vz: 0, a: 0, e: 0, inc: 0, raan: 0, omega: 0, nu: 0 };
}

describe("computeReplayDrawStart", () => {
  it("returns 0 when timeRange is null", () => {
    const points = Array.from({ length: 11 }, (_, i) => makePoint(i * 10));
    expect(computeReplayDrawStart(points, 100, null)).toBe(0);
  });

  it("returns 0 for empty points array", () => {
    expect(computeReplayDrawStart([], 100, 30)).toBe(0);
  });

  it("returns 0 when timeRange covers all data", () => {
    const points = Array.from({ length: 11 }, (_, i) => makePoint(i * 10));
    // currentT=100, timeRange=200 → startT=-100, before all points
    expect(computeReplayDrawStart(points, 100, 200)).toBe(0);
  });

  it("clips start for timeRange shorter than data duration", () => {
    // points at t=0,10,20,...,100
    const points = Array.from({ length: 11 }, (_, i) => makePoint(i * 10));
    // currentT=100, timeRange=30 → startT=70 → last index with t<=70 is index 7
    expect(computeReplayDrawStart(points, 100, 30)).toBe(7);
  });

  it("clips start when playback is in the middle", () => {
    const points = Array.from({ length: 11 }, (_, i) => makePoint(i * 10));
    // currentT=50, timeRange=20 → startT=30 → last index with t<=30 is index 3
    expect(computeReplayDrawStart(points, 50, 20)).toBe(3);
  });

  it("returns 0 when startT equals first point time", () => {
    const points = Array.from({ length: 11 }, (_, i) => makePoint(i * 10));
    // currentT=100, timeRange=100 → startT=0 → t[0]=0 → returns 0
    expect(computeReplayDrawStart(points, 100, 100)).toBe(0);
  });

  it("returns 0 when startT is before first point", () => {
    const points = Array.from({ length: 6 }, (_, i) => makePoint(50 + i * 10));
    // currentT=60, timeRange=30 → startT=30, before first point t=50
    expect(computeReplayDrawStart(points, 60, 30)).toBe(0);
  });

  it("returns last index when startT is after last point", () => {
    const points = Array.from({ length: 11 }, (_, i) => makePoint(i * 10));
    // currentT=200, timeRange=30 → startT=170, after last point t=100
    expect(computeReplayDrawStart(points, 200, 30)).toBe(10);
  });

  it("handles single point", () => {
    const points = [makePoint(50)];
    // currentT=100, timeRange=30 → startT=70 > 50 → returns last index = 0
    expect(computeReplayDrawStart(points, 100, 30)).toBe(0);
  });

  it("handles startT between points (non-exact match)", () => {
    const points = Array.from({ length: 11 }, (_, i) => makePoint(i * 10));
    // currentT=100, timeRange=25 → startT=75 → last index with t<=75 is index 7 (t=70)
    expect(computeReplayDrawStart(points, 100, 25)).toBe(7);
  });
});
