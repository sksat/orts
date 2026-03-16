import { describe, expect, it } from "vitest";
import type { OrbitPoint } from "../orbit.js";
import { TrailBuffer } from "./TrailBuffer.js";

function makePoint(t: number): OrbitPoint {
  return {
    t,
    x: 6778 + t,
    y: t * 0.1,
    z: 0,
    vx: 0,
    vy: 7.669,
    vz: 0,
    a: 0,
    e: 0,
    inc: 0,
    raan: 0,
    omega: 0,
    nu: 0,
  };
}

describe("TrailBuffer", () => {
  it("push / length / latest basic operation", () => {
    const buf = new TrailBuffer(100);
    expect(buf.length).toBe(0);
    expect(buf.latest).toBeNull();

    buf.push(makePoint(0));
    expect(buf.length).toBe(1);
    expect(buf.latest?.t).toBe(0);

    buf.push(makePoint(10));
    expect(buf.length).toBe(2);
    expect(buf.latest?.t).toBe(10);
  });

  it("trims when exceeding capacity * 1.5", () => {
    const buf = new TrailBuffer(10);
    // Push 16 points (> 10 * 1.5 = 15)
    for (let i = 0; i < 16; i++) {
      buf.push(makePoint(i));
    }

    // Should have trimmed to capacity (10)
    expect(buf.length).toBe(10);
    // Oldest remaining should be point 6 (kept last 10 of 0..15)
    expect(buf.getAll()[0].t).toBe(6);
    expect(buf.latest?.t).toBe(15);
  });

  it("increments generation on trim", () => {
    const buf = new TrailBuffer(10);
    expect(buf.generation).toBe(0);

    // Push under threshold — no trim
    for (let i = 0; i < 15; i++) {
      buf.push(makePoint(i));
    }
    expect(buf.generation).toBe(0);

    // One more push triggers trim (16 > 15)
    buf.push(makePoint(15));
    expect(buf.generation).toBe(1);
  });

  it("latest returns most recent point after trim", () => {
    const buf = new TrailBuffer(10);
    for (let i = 0; i < 20; i++) {
      buf.push(makePoint(i));
    }
    expect(buf.latest?.t).toBe(19);
  });

  it("pushMany adds multiple points and trims once", () => {
    const buf = new TrailBuffer(10);
    const points = Array.from({ length: 20 }, (_, i) => makePoint(i));
    buf.pushMany(points);

    expect(buf.length).toBe(10);
    expect(buf.getAll()[0].t).toBe(10);
    expect(buf.latest?.t).toBe(19);
    // Only one trim should have happened
    expect(buf.generation).toBe(1);
  });

  it("clear resets and increments generation", () => {
    const buf = new TrailBuffer(100);
    buf.push(makePoint(0));
    buf.push(makePoint(10));
    expect(buf.generation).toBe(0);

    buf.clear();
    expect(buf.length).toBe(0);
    expect(buf.latest).toBeNull();
    expect(buf.generation).toBe(1);
  });

  it("getAll returns reference to internal array", () => {
    const buf = new TrailBuffer(100);
    buf.push(makePoint(0));
    buf.push(makePoint(10));

    const all = buf.getAll();
    expect(all).toHaveLength(2);
    expect(all[0].t).toBe(0);
    expect(all[1].t).toBe(10);

    // Pushing more should be reflected (same reference)
    buf.push(makePoint(20));
    expect(all).toHaveLength(3);
  });

  it("capacity of 1 works correctly", () => {
    const buf = new TrailBuffer(1);
    buf.push(makePoint(0));
    expect(buf.length).toBe(1);

    // Push second point — triggers trim at > 1.5 (i.e., at 2)
    buf.push(makePoint(10));
    expect(buf.length).toBe(1);
    expect(buf.latest?.t).toBe(10);
  });
});

describe("TrailBuffer.interpolateAt", () => {
  it("returns null on empty buffer", () => {
    const buf = new TrailBuffer(100);
    expect(buf.interpolateAt(5)).toBeNull();
  });

  it("returns the single point for a one-element buffer", () => {
    const buf = new TrailBuffer(100);
    buf.push(makePoint(10));
    const result = buf.interpolateAt(10)!;
    expect(result.t).toBe(10);
    expect(result.x).toBe(makePoint(10).x);
  });

  it("returns first point when t is before first", () => {
    const buf = new TrailBuffer(100);
    buf.push(makePoint(10));
    buf.push(makePoint(20));
    const result = buf.interpolateAt(5)!;
    expect(result.t).toBe(10);
    expect(result.x).toBe(makePoint(10).x);
  });

  it("returns last point when t is after last", () => {
    const buf = new TrailBuffer(100);
    buf.push(makePoint(10));
    buf.push(makePoint(20));
    const result = buf.interpolateAt(25)!;
    expect(result.t).toBe(20);
    expect(result.x).toBe(makePoint(20).x);
  });

  it("returns exact point when t matches", () => {
    const buf = new TrailBuffer(100);
    buf.push(makePoint(0));
    buf.push(makePoint(10));
    buf.push(makePoint(20));
    const result = buf.interpolateAt(10)!;
    expect(result.t).toBe(10);
    expect(result.x).toBe(makePoint(10).x);
  });

  it("interpolates between two points", () => {
    const buf = new TrailBuffer(100);
    buf.push(makePoint(0));
    buf.push(makePoint(10));
    // makePoint: x = 6778 + t, y = t * 0.1
    // At t=5: x = (6778+0)*0.5 + (6778+10)*0.5 = 6783
    const result = buf.interpolateAt(5)!;
    expect(result.t).toBeCloseTo(5);
    expect(result.x).toBeCloseTo(6783);
    expect(result.y).toBeCloseTo(0.5);
  });

  it("interpolates at 25% between points", () => {
    const buf = new TrailBuffer(100);
    buf.push(makePoint(0));
    buf.push(makePoint(20));
    // At t=5 (25% of [0,20]): x = 6778 * 0.75 + 6798 * 0.25 = 6783
    const result = buf.interpolateAt(5)!;
    expect(result.t).toBeCloseTo(5);
    expect(result.x).toBeCloseTo(6783);
  });
});

describe("TrailBuffer.indexBefore", () => {
  it("returns -1 on empty buffer", () => {
    const buf = new TrailBuffer(100);
    expect(buf.indexBefore(5)).toBe(-1);
  });

  it("returns -1 when t is before all points", () => {
    const buf = new TrailBuffer(100);
    buf.push(makePoint(10));
    buf.push(makePoint(20));
    expect(buf.indexBefore(5)).toBe(-1);
  });

  it("returns last index when t is after all points", () => {
    const buf = new TrailBuffer(100);
    buf.push(makePoint(10));
    buf.push(makePoint(20));
    expect(buf.indexBefore(25)).toBe(1);
  });

  it("returns correct index for exact match", () => {
    const buf = new TrailBuffer(100);
    buf.push(makePoint(0));
    buf.push(makePoint(10));
    buf.push(makePoint(20));
    expect(buf.indexBefore(10)).toBe(1);
  });

  it("returns index of point just before t", () => {
    const buf = new TrailBuffer(100);
    buf.push(makePoint(0));
    buf.push(makePoint(10));
    buf.push(makePoint(20));
    expect(buf.indexBefore(15)).toBe(1);
  });

  it("returns 0 for t equal to first point", () => {
    const buf = new TrailBuffer(100);
    buf.push(makePoint(0));
    buf.push(makePoint(10));
    expect(buf.indexBefore(0)).toBe(0);
  });
});

describe("TrailBuffer history replay sequence", () => {
  // Simulates the full handleHistory → handleHistoryDetailComplete flow
  // to verify TrailBuffer correctly holds historical data after replay.

  it("retains history overview data after push", () => {
    const buf = new TrailBuffer(50000);
    // Simulate handleHistory: push downsampled overview (100 points)
    const overview = Array.from({ length: 100 }, (_, i) => makePoint(i * 10));
    for (const p of overview) buf.push(p);

    expect(buf.length).toBe(100);
    expect(buf.getAll()[0].t).toBe(0);
    expect(buf.latest?.t).toBe(990);
  });

  it("rebuilds with detail + streaming after handleHistoryDetailComplete", () => {
    const buf = new TrailBuffer(50000);

    // Step 1: Simulate handleHistory (overview: 100 points)
    const overview = Array.from({ length: 100 }, (_, i) => makePoint(i * 10));
    for (const p of overview) buf.push(p);
    const genAfterOverview = buf.generation;

    // Step 2: Simulate streaming points arriving (20 points after overview)
    const streaming = Array.from({ length: 20 }, (_, i) => makePoint(1000 + i * 10));
    for (const p of streaming) buf.push(p);
    expect(buf.length).toBe(120);

    // Step 3: Simulate handleHistoryDetailComplete
    //   detail = full resolution of history (500 points)
    //   streaming = last 20 points from buffer
    const detail = Array.from({ length: 500 }, (_, i) => makePoint(i * 2));
    const streamingSlice = buf.getAll().slice(-20);
    const combined = [...detail, ...streamingSlice].sort((a, b) => a.t - b.t);

    buf.clear();
    buf.pushMany(combined);

    // After rebuild: should contain detail (0..998) + streaming (1000..1190)
    expect(buf.length).toBe(combined.length);
    expect(buf.getAll()[0].t).toBe(0); // Starts from the beginning of history
    expect(buf.latest?.t).toBe(1190); // Includes streaming data
    expect(buf.generation).toBeGreaterThan(genAfterOverview); // Generation incremented by clear
  });

  it("generation increments on clear during replay rebuild", () => {
    const buf = new TrailBuffer(50000);
    buf.push(makePoint(0));
    const gen0 = buf.generation;

    buf.clear();
    expect(buf.generation).toBe(gen0 + 1);

    buf.pushMany([makePoint(0), makePoint(10), makePoint(20)]);
    // pushMany doesn't trim (3 << 50000), so generation stays
    expect(buf.generation).toBe(gen0 + 1);
  });
});
