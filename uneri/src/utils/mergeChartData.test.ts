import { describe, expect, it } from "vitest";
import type { ChartDataMap } from "../types.js";
import { mergeChartData, trimChartDataLeft } from "./mergeChartData.js";

function f64(...values: number[]): Float64Array {
  return new Float64Array(values);
}

describe("mergeChartData", () => {
  it("returns cold directly when hot is null", () => {
    const cold: ChartDataMap = { t: f64(1, 2, 3), value: f64(10, 20, 30) };
    const result = mergeChartData(cold, null, ["value"]);
    expect(result).toBe(cold); // same reference, no allocation
  });

  it("returns cold directly when hot is empty", () => {
    const cold: ChartDataMap = { t: f64(1, 2, 3), value: f64(10, 20, 30) };
    const hot: ChartDataMap = { t: f64(), value: f64() };
    const result = mergeChartData(cold, hot, ["value"]);
    expect(result).toBe(cold);
  });

  it("concatenates cold + hot correctly", () => {
    const cold: ChartDataMap = { t: f64(1, 2, 3), value: f64(10, 20, 30) };
    const hot: ChartDataMap = { t: f64(4, 5), value: f64(40, 50) };
    const result = mergeChartData(cold, hot, ["value"]);

    expect(Array.from(result.t)).toEqual([1, 2, 3, 4, 5]);
    expect(Array.from(result.value)).toEqual([10, 20, 30, 40, 50]);
  });

  it("handles multiple derived columns", () => {
    const cold: ChartDataMap = {
      t: f64(1, 2),
      alt: f64(100, 200),
      vel: f64(7.5, 7.6),
    };
    const hot: ChartDataMap = {
      t: f64(3),
      alt: f64(300),
      vel: f64(7.7),
    };
    const result = mergeChartData(cold, hot, ["alt", "vel"]);

    expect(Array.from(result.t)).toEqual([1, 2, 3]);
    expect(Array.from(result.alt)).toEqual([100, 200, 300]);
    expect(Array.from(result.vel)).toEqual([7.5, 7.6, 7.7]);
  });

  it("handles cold with no derived (t only)", () => {
    const cold: ChartDataMap = { t: f64(1, 2) };
    const hot: ChartDataMap = { t: f64(3) };
    const result = mergeChartData(cold, hot, []);

    expect(Array.from(result.t)).toEqual([1, 2, 3]);
  });
});

describe("trimChartDataLeft", () => {
  it("returns original when no trimming needed", () => {
    const data: ChartDataMap = { t: f64(5, 10, 15), value: f64(1, 2, 3) };
    const result = trimChartDataLeft(data, 3, ["value"]);
    expect(result).toBe(data); // same reference
  });

  it("trims points where t < tMin", () => {
    const data: ChartDataMap = { t: f64(1, 2, 3, 4, 5), value: f64(10, 20, 30, 40, 50) };
    const result = trimChartDataLeft(data, 3, ["value"]);

    expect(Array.from(result.t)).toEqual([3, 4, 5]);
    expect(Array.from(result.value)).toEqual([30, 40, 50]);
  });

  it("trims all points when all < tMin", () => {
    const data: ChartDataMap = { t: f64(1, 2, 3), value: f64(10, 20, 30) };
    const result = trimChartDataLeft(data, 100, ["value"]);

    expect(result.t.length).toBe(0);
    expect(result.value.length).toBe(0);
  });

  it("handles empty data", () => {
    const data: ChartDataMap = { t: f64(), value: f64() };
    const result = trimChartDataLeft(data, 5, ["value"]);
    expect(result).toBe(data);
  });

  it("uses subarray (zero-copy)", () => {
    const data: ChartDataMap = { t: f64(1, 2, 3, 4, 5), value: f64(10, 20, 30, 40, 50) };
    const result = trimChartDataLeft(data, 3, ["value"]);

    // subarray shares the same underlying ArrayBuffer
    expect(result.t.buffer).toBe(data.t.buffer);
    expect(result.value.buffer).toBe(data.value.buffer);
  });
});
