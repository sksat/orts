import { describe, it, expect } from "vitest";
import { TrailBuffer } from "./TrailBuffer.js";
import type { OrbitPoint } from "../orbit.js";

function makePoint(t: number): OrbitPoint {
  return { t, x: 6778 + t, y: t * 0.1, z: 0, vx: 0, vy: 7.669, vz: 0 };
}

describe("TrailBuffer", () => {
  it("push / length / latest basic operation", () => {
    const buf = new TrailBuffer(100);
    expect(buf.length).toBe(0);
    expect(buf.latest).toBeNull();

    buf.push(makePoint(0));
    expect(buf.length).toBe(1);
    expect(buf.latest!.t).toBe(0);

    buf.push(makePoint(10));
    expect(buf.length).toBe(2);
    expect(buf.latest!.t).toBe(10);
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
    expect(buf.latest!.t).toBe(15);
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
    expect(buf.latest!.t).toBe(19);
  });

  it("pushMany adds multiple points and trims once", () => {
    const buf = new TrailBuffer(10);
    const points = Array.from({ length: 20 }, (_, i) => makePoint(i));
    buf.pushMany(points);

    expect(buf.length).toBe(10);
    expect(buf.getAll()[0].t).toBe(10);
    expect(buf.latest!.t).toBe(19);
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
    expect(buf.latest!.t).toBe(10);
  });
});
