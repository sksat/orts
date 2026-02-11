import { describe, it, expect } from "vitest";
import { IngestBuffer } from "./IngestBuffer.js";
import type { TimePoint } from "../types.js";

/** Multi-field test point mimicking orbital data. */
interface TestPoint extends TimePoint {
  t: number;
  x: number;
  y: number;
  z: number;
  vx: number;
  vy: number;
  vz: number;
}

function makePoint(t: number): TestPoint {
  return { t, x: 6778 + t, y: t * 0.1, z: 0, vx: 0, vy: 7.669, vz: 0 };
}

describe("IngestBuffer", () => {
  it("push then drain returns point and empties buffer", () => {
    const buf = new IngestBuffer<TestPoint>();
    buf.push(makePoint(0));
    buf.push(makePoint(10));

    const drained = buf.drain();
    expect(drained).toHaveLength(2);
    expect(drained[0].t).toBe(0);
    expect(drained[1].t).toBe(10);
    expect(buf.pendingCount).toBe(0);
  });

  it("second drain returns empty array", () => {
    const buf = new IngestBuffer<TestPoint>();
    buf.push(makePoint(0));
    buf.drain();

    const second = buf.drain();
    expect(second).toHaveLength(0);
  });

  it("pushMany preserves insertion order", () => {
    const buf = new IngestBuffer<TestPoint>();
    const points = [makePoint(0), makePoint(10), makePoint(20)];
    buf.pushMany(points);

    const drained = buf.drain();
    expect(drained).toHaveLength(3);
    expect(drained[0].t).toBe(0);
    expect(drained[1].t).toBe(10);
    expect(drained[2].t).toBe(20);
  });

  it("latestT tracks the maximum t value", () => {
    const buf = new IngestBuffer<TestPoint>();
    expect(buf.latestT).toBe(-Infinity);

    buf.push(makePoint(10));
    expect(buf.latestT).toBe(10);

    buf.push(makePoint(5));
    expect(buf.latestT).toBe(10);

    buf.push(makePoint(20));
    expect(buf.latestT).toBe(20);
  });

  it("latestT persists after drain", () => {
    const buf = new IngestBuffer<TestPoint>();
    buf.push(makePoint(100));
    buf.drain();

    // latestT should still reflect the last seen t
    expect(buf.latestT).toBe(100);
  });

  it("pendingCount reflects accumulated points", () => {
    const buf = new IngestBuffer<TestPoint>();
    expect(buf.pendingCount).toBe(0);

    buf.push(makePoint(0));
    expect(buf.pendingCount).toBe(1);

    buf.pushMany([makePoint(10), makePoint(20)]);
    expect(buf.pendingCount).toBe(3);

    buf.drain();
    expect(buf.pendingCount).toBe(0);
  });

  it("works with minimal TimePoint type (just {t: number})", () => {
    const buf = new IngestBuffer<TimePoint>();
    buf.push({ t: 42 });
    buf.push({ t: 99 });

    const drained = buf.drain();
    expect(drained).toHaveLength(2);
    expect(drained[0].t).toBe(42);
    expect(drained[1].t).toBe(99);
    expect(buf.latestT).toBe(99);
  });

  it("works with complex multi-field types", () => {
    interface SensorPoint extends TimePoint {
      t: number;
      temperature: number;
      pressure: number;
      humidity: number;
      altitude: number;
    }

    const buf = new IngestBuffer<SensorPoint>();
    buf.push({ t: 0, temperature: 20.5, pressure: 1013.25, humidity: 65, altitude: 100 });
    buf.push({ t: 1, temperature: 21.0, pressure: 1013.00, humidity: 64, altitude: 105 });
    buf.pushMany([
      { t: 2, temperature: 21.5, pressure: 1012.75, humidity: 63, altitude: 110 },
      { t: 3, temperature: 22.0, pressure: 1012.50, humidity: 62, altitude: 115 },
    ]);

    expect(buf.pendingCount).toBe(4);
    expect(buf.latestT).toBe(3);

    const drained = buf.drain();
    expect(drained).toHaveLength(4);
    expect(drained[0].temperature).toBe(20.5);
    expect(drained[3].altitude).toBe(115);
    expect(buf.pendingCount).toBe(0);
    expect(buf.latestT).toBe(3);
  });
});
