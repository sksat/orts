import { describe, it, expect } from "vitest";
import { safeYRange, buildMultiSeriesConfig } from "./TimeSeriesChart.js";

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

describe("buildMultiSeriesConfig", () => {
  it("returns just x-axis entry for zero series", () => {
    const result = buildMultiSeriesConfig([]);
    expect(result).toHaveLength(1);
    expect(result[0]).toEqual({});
  });

  it("returns x-axis + one y-series for single series", () => {
    const result = buildMultiSeriesConfig([
      { label: "SSO", color: "#f00" },
    ]);
    expect(result).toHaveLength(2);
    expect(result[0]).toEqual({});
    expect(result[1]).toEqual({
      label: "SSO",
      stroke: "#f00",
      width: 1.5,
    });
  });

  it("returns correct config for three series", () => {
    const result = buildMultiSeriesConfig([
      { label: "A", color: "#f00" },
      { label: "B", color: "#0f0" },
      { label: "C", color: "#00f" },
    ]);
    expect(result).toHaveLength(4);
    expect(result[0]).toEqual({});
    expect(result[1].label).toBe("A");
    expect(result[1].stroke).toBe("#f00");
    expect(result[2].label).toBe("B");
    expect(result[2].stroke).toBe("#0f0");
    expect(result[3].label).toBe("C");
    expect(result[3].stroke).toBe("#00f");
  });

  it("preserves color and label exactly", () => {
    const result = buildMultiSeriesConfig([
      { label: "ISS (ZARYA)", color: "rgba(255,0,0,0.8)" },
    ]);
    expect(result[1].label).toBe("ISS (ZARYA)");
    expect(result[1].stroke).toBe("rgba(255,0,0,0.8)");
  });
});
