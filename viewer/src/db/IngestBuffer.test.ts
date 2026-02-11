import { describe, it, expect } from "vitest";
import { IngestBuffer } from "./IngestBuffer.js";
import type { OrbitPoint } from "../orbit.js";

function makePoint(t: number): OrbitPoint {
  return { t, x: 6778 + t, y: t * 0.1, z: 0, vx: 0, vy: 7.669, vz: 0 };
}

describe("IngestBuffer", () => {
  it("push then drain returns point and empties buffer", () => {
    const buf = new IngestBuffer();
    buf.push(makePoint(0));
    buf.push(makePoint(10));

    const drained = buf.drain();
    expect(drained).toHaveLength(2);
    expect(drained[0].t).toBe(0);
    expect(drained[1].t).toBe(10);
    expect(buf.pendingCount).toBe(0);
  });

  it("second drain returns empty array", () => {
    const buf = new IngestBuffer();
    buf.push(makePoint(0));
    buf.drain();

    const second = buf.drain();
    expect(second).toHaveLength(0);
  });

  it("pushMany preserves insertion order", () => {
    const buf = new IngestBuffer();
    const points = [makePoint(0), makePoint(10), makePoint(20)];
    buf.pushMany(points);

    const drained = buf.drain();
    expect(drained).toHaveLength(3);
    expect(drained[0].t).toBe(0);
    expect(drained[1].t).toBe(10);
    expect(drained[2].t).toBe(20);
  });

  it("latestT tracks the maximum t value", () => {
    const buf = new IngestBuffer();
    expect(buf.latestT).toBe(-Infinity);

    buf.push(makePoint(10));
    expect(buf.latestT).toBe(10);

    buf.push(makePoint(5));
    expect(buf.latestT).toBe(10);

    buf.push(makePoint(20));
    expect(buf.latestT).toBe(20);
  });

  it("latestT persists after drain", () => {
    const buf = new IngestBuffer();
    buf.push(makePoint(100));
    buf.drain();

    // latestT should still reflect the last seen t
    expect(buf.latestT).toBe(100);
  });

  it("pendingCount reflects accumulated points", () => {
    const buf = new IngestBuffer();
    expect(buf.pendingCount).toBe(0);

    buf.push(makePoint(0));
    expect(buf.pendingCount).toBe(1);

    buf.pushMany([makePoint(10), makePoint(20)]);
    expect(buf.pendingCount).toBe(3);

    buf.drain();
    expect(buf.pendingCount).toBe(0);
  });

});
