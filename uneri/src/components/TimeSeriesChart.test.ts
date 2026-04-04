import { describe, expect, it } from "vitest";
import {
  buildMultiSeriesConfig,
  computeLegendIsolation,
  float64NanToNull,
  safeYRange,
} from "./TimeSeriesChart.js";

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
    const result = buildMultiSeriesConfig([{ label: "SSO", color: "#f00" }]);
    expect(result).toHaveLength(2);
    expect(result[0]).toEqual({});
    expect(result[1]).toEqual({
      label: "SSO",
      stroke: "#f00",
      width: 1.5,
      spanGaps: true,
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
    expect(result[1].spanGaps).toBe(true);
    expect(result[2].label).toBe("B");
    expect(result[2].stroke).toBe("#0f0");
    expect(result[2].spanGaps).toBe(true);
    expect(result[3].label).toBe("C");
    expect(result[3].stroke).toBe("#00f");
    expect(result[3].spanGaps).toBe(true);
  });

  it("preserves color and label exactly", () => {
    const result = buildMultiSeriesConfig([{ label: "ISS (ZARYA)", color: "rgba(255,0,0,0.8)" }]);
    expect(result[1].label).toBe("ISS (ZARYA)");
    expect(result[1].stroke).toBe("rgba(255,0,0,0.8)");
  });
});

describe("float64NanToNull", () => {
  it("converts NaN to null for uPlot compatibility", () => {
    const input = new Float64Array([0.1, NaN, 0.3, NaN, 0.5]);
    const result = float64NanToNull(input);
    expect(result).toEqual([0.1, null, 0.3, null, 0.5]);
  });

  it("preserves all values when no NaN present", () => {
    const input = new Float64Array([1.0, 2.0, 3.0]);
    const result = float64NanToNull(input);
    expect(result).toEqual([1.0, 2.0, 3.0]);
  });

  it("handles empty array", () => {
    const input = new Float64Array([]);
    const result = float64NanToNull(input);
    expect(result).toEqual([]);
  });

  it("handles all-NaN array", () => {
    const input = new Float64Array([NaN, NaN, NaN]);
    const result = float64NanToNull(input);
    expect(result).toEqual([null, null, null]);
  });

  it("passes Infinity through unchanged", () => {
    const input = new Float64Array([1.0, Infinity, -Infinity, NaN]);
    const result = float64NanToNull(input);
    expect(result).toEqual([1.0, Infinity, -Infinity, null]);
  });
});

describe("computeLegendIsolation", () => {
  // currentShow is indexed like uPlot series: [0]=x-axis (always true), [1..N]=y-series

  it("isolates clicked series when all are visible", () => {
    // 3 y-series, all visible, click series 2
    const result = computeLegendIsolation(2, [true, true, true, true]);
    expect(result).toEqual([true, false, true, false]);
  });

  it("shows all when clicking the already-isolated series", () => {
    // Only series 2 visible → click series 2 → un-isolate (show all)
    const result = computeLegendIsolation(2, [true, false, true, false]);
    expect(result).toEqual([true, true, true, true]);
  });

  it("isolates clicked series when some others are hidden", () => {
    // Series 3 hidden, click series 1 → isolate series 1
    const result = computeLegendIsolation(1, [true, true, true, false]);
    expect(result).toEqual([true, true, false, false]);
  });

  it("isolates a currently-hidden series when clicked", () => {
    // Series 2 hidden, click series 2 → isolate it (show only series 2)
    const result = computeLegendIsolation(2, [true, true, false, true]);
    expect(result).toEqual([true, false, true, false]);
  });

  it("toggles back to all visible for single y-series", () => {
    // 1 y-series, it's already the only one → click → show all (no change)
    const result = computeLegendIsolation(1, [true, true]);
    expect(result).toEqual([true, true]);
  });

  it("isolates with two y-series", () => {
    const result = computeLegendIsolation(1, [true, true, true]);
    expect(result).toEqual([true, true, false]);
  });

  it("un-isolates with two y-series", () => {
    const result = computeLegendIsolation(1, [true, true, false]);
    expect(result).toEqual([true, true, true]);
  });

  it("returns unchanged for out-of-range index", () => {
    const input = [true, true, true];
    expect(computeLegendIsolation(0, input)).toEqual(input);
    expect(computeLegendIsolation(3, input)).toEqual(input);
    expect(computeLegendIsolation(-1, input)).toEqual(input);
  });
});
