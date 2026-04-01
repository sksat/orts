import { describe, expect, it } from "vitest";
import type { OrbitPoint } from "../orbit.js";
import { TrailBuffer } from "../utils/TrailBuffer.js";
import { computeLiveSyncTime, computeTrailDrawStarts } from "./useRealtimePlayback.js";

function makePoint(t: number, entityPath?: string): OrbitPoint {
  return {
    t,
    entityPath,
    x: 6778 + t,
    y: t * 0.1,
    z: 0,
    vx: 0,
    vy: 7.669,
    vz: 0,
    a: 6778,
    e: 0,
    inc: 0.9,
    raan: 0,
    omega: 0,
    nu: 0,
  };
}

describe("computeLiveSyncTime", () => {
  it("returns min of all satellites when none terminated", () => {
    const buffers = new Map<string, TrailBuffer>();
    const bufA = new TrailBuffer(1000);
    const bufB = new TrailBuffer(1000);

    for (let t = 0; t <= 100; t += 10) bufA.push(makePoint(t, "sat-a"));
    for (let t = 0; t <= 200; t += 10) bufB.push(makePoint(t, "sat-b"));

    buffers.set("sat-a", bufA);
    buffers.set("sat-b", bufB);

    const syncTime = computeLiveSyncTime(buffers, new Set());
    expect(syncTime).toBe(100);
  });

  it("excludes terminated satellite from sync time", () => {
    const buffers = new Map<string, TrailBuffer>();
    const bufA = new TrailBuffer(1000);
    const bufB = new TrailBuffer(1000);

    // sat-a stopped at t=100, sat-b continued to t=200
    for (let t = 0; t <= 100; t += 10) bufA.push(makePoint(t, "sat-a"));
    for (let t = 0; t <= 200; t += 10) bufB.push(makePoint(t, "sat-b"));

    buffers.set("sat-a", bufA);
    buffers.set("sat-b", bufB);

    const terminated = new Set(["sat-a"]);
    const syncTime = computeLiveSyncTime(buffers, terminated);
    expect(syncTime).toBe(200);
  });

  it("returns Infinity when all satellites are terminated", () => {
    const buffers = new Map<string, TrailBuffer>();
    const bufA = new TrailBuffer(1000);
    bufA.push(makePoint(100, "sat-a"));
    buffers.set("sat-a", bufA);

    const terminated = new Set(["sat-a"]);
    const syncTime = computeLiveSyncTime(buffers, terminated);
    expect(syncTime).toBe(Infinity);
  });

  it("returns Infinity for empty buffers", () => {
    const buffers = new Map<string, TrailBuffer>();
    const syncTime = computeLiveSyncTime(buffers, new Set());
    expect(syncTime).toBe(Infinity);
  });

  it("skips buffers with no data", () => {
    const buffers = new Map<string, TrailBuffer>();
    const bufA = new TrailBuffer(1000);
    const bufB = new TrailBuffer(1000);

    bufA.push(makePoint(50, "sat-a"));
    // bufB is empty

    buffers.set("sat-a", bufA);
    buffers.set("sat-b", bufB);

    const syncTime = computeLiveSyncTime(buffers, new Set());
    expect(syncTime).toBe(50);
  });
});

describe("computeTrailDrawStarts", () => {
  it("returns all zeros when timeRange is null", () => {
    const buffers = new Map<string, TrailBuffer>();
    const buf = new TrailBuffer(1000);
    for (let t = 0; t <= 100; t += 10) buf.push(makePoint(t));
    buffers.set("sat-a", buf);

    const starts = computeTrailDrawStarts(buffers, 100, null);
    expect(starts.get("sat-a")).toBe(0);
  });

  it("returns 0 when timeRange covers entire buffer", () => {
    const buffers = new Map<string, TrailBuffer>();
    const buf = new TrailBuffer(1000);
    for (let t = 0; t <= 100; t += 10) buf.push(makePoint(t));
    buffers.set("sat-a", buf);

    // timeRange=200 > total duration 100
    const starts = computeTrailDrawStarts(buffers, 100, 200);
    expect(starts.get("sat-a")).toBe(0);
  });

  it("clips start for timeRange shorter than buffer duration", () => {
    const buffers = new Map<string, TrailBuffer>();
    const buf = new TrailBuffer(1000);
    // Points at t=0,10,20,...,100
    for (let t = 0; t <= 100; t += 10) buf.push(makePoint(t));
    buffers.set("sat-a", buf);

    // currentTime=100, timeRange=30 → startT=70 → indexBefore(70)=7
    const starts = computeTrailDrawStarts(buffers, 100, 30);
    expect(starts.get("sat-a")).toBe(7); // point at t=70
  });

  it("clips start when paused in the middle", () => {
    const buffers = new Map<string, TrailBuffer>();
    const buf = new TrailBuffer(1000);
    for (let t = 0; t <= 100; t += 10) buf.push(makePoint(t));
    buffers.set("sat-a", buf);

    // Paused at currentTime=50, timeRange=20 → startT=30 → indexBefore(30)=3
    const starts = computeTrailDrawStarts(buffers, 50, 20);
    expect(starts.get("sat-a")).toBe(3); // point at t=30
  });

  it("handles multiple satellites independently", () => {
    const buffers = new Map<string, TrailBuffer>();
    const bufA = new TrailBuffer(1000);
    const bufB = new TrailBuffer(1000);

    // sat-a: t=0,10,...,100
    for (let t = 0; t <= 100; t += 10) bufA.push(makePoint(t, "sat-a"));
    // sat-b: t=50,60,...,100
    for (let t = 50; t <= 100; t += 10) bufB.push(makePoint(t, "sat-b"));

    buffers.set("sat-a", bufA);
    buffers.set("sat-b", bufB);

    // currentTime=100, timeRange=30 → startT=70
    const starts = computeTrailDrawStarts(buffers, 100, 30);
    // sat-a: indexBefore(70)=7 (t=70)
    expect(starts.get("sat-a")).toBe(7);
    // sat-b: has [50,60,70,80,90,100], indexBefore(70)=2 (t=70)
    expect(starts.get("sat-b")).toBe(2);
  });

  it("returns 0 for empty buffers", () => {
    const buffers = new Map<string, TrailBuffer>();
    const buf = new TrailBuffer(1000);
    buffers.set("sat-a", buf);

    const starts = computeTrailDrawStarts(buffers, 100, 30);
    expect(starts.get("sat-a")).toBe(0);
  });

  it("returns 0 when startT is before all points", () => {
    const buffers = new Map<string, TrailBuffer>();
    const buf = new TrailBuffer(1000);
    for (let t = 50; t <= 100; t += 10) buf.push(makePoint(t));
    buffers.set("sat-a", buf);

    // currentTime=60, timeRange=30 → startT=30, which is before t=50
    const starts = computeTrailDrawStarts(buffers, 60, 30);
    expect(starts.get("sat-a")).toBe(0);
  });
});
