import { describe, it, expect } from "vitest";
import { safeYRange } from "./TimeSeriesChart.js";

const mockU = {} as any;

describe("safeYRange", () => {
  it("pads then delegates for constant data", () => {
    const [min, max] = safeYRange(mockU, 400, 400, "y");
    expect(min).toBeLessThan(400);
    expect(max).toBeGreaterThan(400);
    expect(max - min).toBeGreaterThan(0);
  });

  it("pads then delegates for near-constant data", () => {
    const [min, max] = safeYRange(mockU, 400.0, 400.001, "y");
    expect(min).toBeLessThan(400);
    expect(max).toBeGreaterThan(400.001);
  });

  it("handles zero constant values", () => {
    const [min, max] = safeYRange(mockU, 0, 0, "y");
    expect(min).toBeLessThan(0);
    expect(max).toBeGreaterThan(0);
  });

  it("handles negative near-constant values", () => {
    const [min, max] = safeYRange(mockU, -30.456, -30.456, "y");
    expect(min).toBeLessThan(-30.456);
    expect(max).toBeGreaterThan(-30.456);
  });

  it("delegates to uPlot.rangeNum for normal data ranges", () => {
    const [min, max] = safeYRange(mockU, 0, 100, "y");
    expect(min).toBeLessThanOrEqual(0);
    expect(max).toBeGreaterThanOrEqual(100);
  });

  it("delegates to uPlot.rangeNum for moderate variation", () => {
    const [min, max] = safeYRange(mockU, 400, 450, "y");
    expect(min).toBeLessThan(400);
    expect(max).toBeGreaterThan(450);
  });

  it("no soft-zero: axis doesn't pull to 0 for positive data", () => {
    const [min, max] = safeYRange(mockU, 395, 405, "y");
    // Without soft-zero, min should stay near 395, not go to 0
    expect(min).toBeGreaterThan(390);
    expect(max).toBeLessThan(410);
  });
});
