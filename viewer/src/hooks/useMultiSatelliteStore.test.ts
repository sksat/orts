import { describe, it, expect } from "vitest";
import { buildMultiChartData } from "./buildMultiChartData.js";

/** Minimal ChartDataMap for testing. */
type ChartDataMap = { t: Float64Array; [key: string]: Float64Array };

function f64(arr: number[]): Float64Array {
  return Float64Array.from(arr);
}

describe("buildMultiChartData", () => {
  const metricNames = ["altitude", "energy"];

  it("returns null for empty satellite data", () => {
    const result = buildMultiChartData(
      new Map(),
      metricNames,
      [],
    );
    expect(result).toBeNull();
  });

  it("single satellite produces MultiSeriesData with one series", () => {
    const satData = new Map<string, ChartDataMap>([
      ["sat1", { t: f64([1, 2, 3]), altitude: f64([100, 200, 300]), energy: f64([10, 20, 30]) }],
    ]);
    const configs = [{ id: "sat1", label: "SSO", color: "#f00" }];

    const result = buildMultiChartData(satData, metricNames, configs);
    expect(result).not.toBeNull();

    const alt = result!.altitude;
    expect(alt).not.toBeNull();
    expect(alt!.series).toHaveLength(1);
    expect(alt!.series[0].label).toBe("SSO");
    expect(alt!.series[0].color).toBe("#f00");
    expect(Array.from(alt!.t)).toEqual([1, 2, 3]);
    expect(Array.from(alt!.values[0])).toEqual([100, 200, 300]);
  });

  it("two satellites with same time axes align without NaN", () => {
    const satData = new Map<string, ChartDataMap>([
      ["sat1", { t: f64([1, 2]), altitude: f64([100, 200]), energy: f64([10, 20]) }],
      ["sat2", { t: f64([1, 2]), altitude: f64([150, 250]), energy: f64([15, 25]) }],
    ]);
    const configs = [
      { id: "sat1", label: "SSO", color: "#f00" },
      { id: "sat2", label: "ISS", color: "#00f" },
    ];

    const result = buildMultiChartData(satData, metricNames, configs);
    expect(result).not.toBeNull();

    const alt = result!.altitude;
    expect(alt!.series).toHaveLength(2);
    expect(alt!.series[0].label).toBe("SSO");
    expect(alt!.series[1].label).toBe("ISS");
    expect(Array.from(alt!.t)).toEqual([1, 2]);
    expect(Array.from(alt!.values[0])).toEqual([100, 200]);
    expect(Array.from(alt!.values[1])).toEqual([150, 250]);
  });

  it("two satellites with different time axes fill NaN", () => {
    const satData = new Map<string, ChartDataMap>([
      ["sat1", { t: f64([1, 2]), altitude: f64([100, 200]), energy: f64([10, 20]) }],
      ["sat2", { t: f64([2, 3]), altitude: f64([250, 350]), energy: f64([25, 35]) }],
    ]);
    const configs = [
      { id: "sat1", label: "A", color: "#f00" },
      { id: "sat2", label: "B", color: "#00f" },
    ];

    const result = buildMultiChartData(satData, metricNames, configs);
    const alt = result!.altitude!;

    expect(Array.from(alt.t)).toEqual([1, 2, 3]);
    // A: [100, 200, NaN]
    expect(alt.values[0][0]).toBe(100);
    expect(alt.values[0][1]).toBe(200);
    expect(alt.values[0][2]).toBeNaN();
    // B: [NaN, 250, 350]
    expect(alt.values[1][0]).toBeNaN();
    expect(alt.values[1][1]).toBe(250);
    expect(alt.values[1][2]).toBe(350);
  });

  it("skips satellites not in config", () => {
    const satData = new Map<string, ChartDataMap>([
      ["sat1", { t: f64([1, 2]), altitude: f64([100, 200]), energy: f64([10, 20]) }],
      ["unknown", { t: f64([1, 2]), altitude: f64([0, 0]), energy: f64([0, 0]) }],
    ]);
    const configs = [{ id: "sat1", label: "SSO", color: "#f00" }];

    const result = buildMultiChartData(satData, metricNames, configs);
    const alt = result!.altitude!;
    expect(alt.series).toHaveLength(1);
    expect(alt.series[0].label).toBe("SSO");
  });
});
