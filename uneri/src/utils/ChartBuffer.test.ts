import { describe, expect, it } from "vitest";
import { ChartBuffer } from "./ChartBuffer.js";

describe("ChartBuffer", () => {
  it("requires t column", () => {
    expect(() => new ChartBuffer(["x", "y"], 100)).toThrow('columns must include "t"');
  });

  it("starts empty", () => {
    const buf = new ChartBuffer(["t", "altitude"], 100);
    expect(buf.length).toBe(0);
    expect(buf.latestT).toBe(-Infinity);
    expect(buf.earliestT).toBe(Infinity);
  });

  it("push adds a row", () => {
    const buf = new ChartBuffer(["t", "altitude"], 100);
    buf.push({ t: 10, altitude: 800 });
    expect(buf.length).toBe(1);
    expect(buf.latestT).toBe(10);
    expect(buf.earliestT).toBe(10);
  });

  it("pushMany adds multiple rows", () => {
    const buf = new ChartBuffer(["t", "altitude", "velocity"], 100);
    buf.pushMany([
      { t: 0, altitude: 800, velocity: 7.5 },
      { t: 10, altitude: 801, velocity: 7.4 },
      { t: 20, altitude: 799, velocity: 7.6 },
    ]);
    expect(buf.length).toBe(3);
    expect(buf.earliestT).toBe(0);
    expect(buf.latestT).toBe(20);
  });

  it("toChartData returns subarrays of correct length", () => {
    const buf = new ChartBuffer(["t", "alt"], 100);
    buf.pushMany([
      { t: 0, alt: 800 },
      { t: 10, alt: 810 },
    ]);
    const data = buf.toChartData();
    expect(data.t.length).toBe(2);
    expect(data.alt.length).toBe(2);
    expect(data.t[0]).toBe(0);
    expect(data.t[1]).toBe(10);
    expect(data.alt[0]).toBe(800);
    expect(data.alt[1]).toBe(810);
  });

  it("missing column values default to 0", () => {
    const buf = new ChartBuffer(["t", "a", "b"], 100);
    buf.push({ t: 5, a: 1 }); // b missing
    const data = buf.toChartData();
    expect(data.b[0]).toBe(0);
  });

  it("trims oldest half when capacity reached", () => {
    const buf = new ChartBuffer(["t", "v"], 4);
    buf.pushMany([
      { t: 0, v: 10 },
      { t: 1, v: 11 },
      { t: 2, v: 12 },
      { t: 3, v: 13 },
    ]);
    expect(buf.length).toBe(4);

    // Push one more → triggers trim (drop oldest half=2, keep 2, then add)
    buf.push({ t: 4, v: 14 });
    expect(buf.length).toBe(3); // 2 kept + 1 new
    expect(buf.earliestT).toBe(2);
    expect(buf.latestT).toBe(4);

    const data = buf.toChartData();
    expect(Array.from(data.t)).toEqual([2, 3, 4]);
    expect(Array.from(data.v)).toEqual([12, 13, 14]);
  });

  it("getWindow returns correct range", () => {
    const buf = new ChartBuffer(["t", "alt"], 100);
    buf.pushMany([
      { t: 0, alt: 800 },
      { t: 10, alt: 810 },
      { t: 20, alt: 820 },
      { t: 30, alt: 830 },
      { t: 40, alt: 840 },
    ]);

    const win = buf.getWindow(10, 30);
    expect(win.t.length).toBe(3);
    expect(Array.from(win.t)).toEqual([10, 20, 30]);
    expect(Array.from(win.alt)).toEqual([810, 820, 830]);
  });

  it("getWindow with no matching points returns empty", () => {
    const buf = new ChartBuffer(["t", "x"], 100);
    buf.push({ t: 10, x: 1 });
    const win = buf.getWindow(20, 30);
    expect(win.t.length).toBe(0);
  });

  it("clear resets length", () => {
    const buf = new ChartBuffer(["t"], 100);
    buf.push({ t: 1 });
    buf.push({ t: 2 });
    buf.clear();
    expect(buf.length).toBe(0);
    expect(buf.latestT).toBe(-Infinity);
  });
});
