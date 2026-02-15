import { describe, it, expect } from "vitest";
import { TrailBuffer } from "../utils/TrailBuffer.js";
import { computeLiveSyncTime } from "./useRealtimePlayback.js";
import type { OrbitPoint } from "../orbit.js";

function makePoint(t: number, satelliteId?: string): OrbitPoint {
  return {
    t, satelliteId,
    x: 6778 + t, y: t * 0.1, z: 0,
    vx: 0, vy: 7.669, vz: 0,
    a: 6778, e: 0, inc: 0.9, raan: 0, omega: 0, nu: 0,
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
