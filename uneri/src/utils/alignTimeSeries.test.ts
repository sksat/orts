import { describe, expect, it } from "vitest";
import { alignTimeSeries, type NamedTimeSeries } from "./alignTimeSeries.js";

function f64(arr: number[]): Float64Array {
  return Float64Array.from(arr);
}

describe("alignTimeSeries", () => {
  it("returns empty result for empty input", () => {
    const result = alignTimeSeries([]);
    expect(result.t.length).toBe(0);
    expect(result.values).toHaveLength(0);
    expect(result.labels).toHaveLength(0);
  });

  it("single series returns identity (no NaN)", () => {
    const input: NamedTimeSeries[] = [{ label: "A", t: f64([1, 2, 3]), values: f64([10, 20, 30]) }];
    const result = alignTimeSeries(input);
    expect(Array.from(result.t)).toEqual([1, 2, 3]);
    expect(result.values).toHaveLength(1);
    expect(Array.from(result.values[0])).toEqual([10, 20, 30]);
    expect(result.labels).toEqual(["A"]);
  });

  it("two series with identical time axes have no gaps", () => {
    const input: NamedTimeSeries[] = [
      { label: "A", t: f64([1, 2, 3]), values: f64([10, 20, 30]) },
      { label: "B", t: f64([1, 2, 3]), values: f64([100, 200, 300]) },
    ];
    const result = alignTimeSeries(input);
    expect(Array.from(result.t)).toEqual([1, 2, 3]);
    expect(Array.from(result.values[0])).toEqual([10, 20, 30]);
    expect(Array.from(result.values[1])).toEqual([100, 200, 300]);
    expect(result.labels).toEqual(["A", "B"]);
  });

  it("two series with completely disjoint time axes fill NaN", () => {
    const input: NamedTimeSeries[] = [
      { label: "A", t: f64([1, 2]), values: f64([10, 20]) },
      { label: "B", t: f64([3, 4]), values: f64([30, 40]) },
    ];
    const result = alignTimeSeries(input);
    expect(Array.from(result.t)).toEqual([1, 2, 3, 4]);
    expect(result.values[0][0]).toBe(10);
    expect(result.values[0][1]).toBe(20);
    expect(result.values[0][2]).toBeNaN();
    expect(result.values[0][3]).toBeNaN();
    expect(result.values[1][0]).toBeNaN();
    expect(result.values[1][1]).toBeNaN();
    expect(result.values[1][2]).toBe(30);
    expect(result.values[1][3]).toBe(40);
  });

  it("two series with partial overlap place NaN correctly", () => {
    const input: NamedTimeSeries[] = [
      { label: "A", t: f64([1, 2, 3]), values: f64([10, 20, 30]) },
      { label: "B", t: f64([2, 3, 4]), values: f64([200, 300, 400]) },
    ];
    const result = alignTimeSeries(input);
    expect(Array.from(result.t)).toEqual([1, 2, 3, 4]);
    // A: [10, 20, 30, NaN]
    expect(result.values[0][0]).toBe(10);
    expect(result.values[0][1]).toBe(20);
    expect(result.values[0][2]).toBe(30);
    expect(result.values[0][3]).toBeNaN();
    // B: [NaN, 200, 300, 400]
    expect(result.values[1][0]).toBeNaN();
    expect(result.values[1][1]).toBe(200);
    expect(result.values[1][2]).toBe(300);
    expect(result.values[1][3]).toBe(400);
  });

  it("three series generalizes correctly", () => {
    const input: NamedTimeSeries[] = [
      { label: "A", t: f64([1, 3]), values: f64([10, 30]) },
      { label: "B", t: f64([2, 3]), values: f64([20, 30]) },
      { label: "C", t: f64([1, 2, 3]), values: f64([100, 200, 300]) },
    ];
    const result = alignTimeSeries(input);
    expect(Array.from(result.t)).toEqual([1, 2, 3]);
    expect(result.values).toHaveLength(3);
    expect(result.labels).toEqual(["A", "B", "C"]);
    // A: [10, NaN, 30]
    expect(result.values[0][0]).toBe(10);
    expect(result.values[0][1]).toBeNaN();
    expect(result.values[0][2]).toBe(30);
    // B: [NaN, 20, 30]
    expect(result.values[1][0]).toBeNaN();
    expect(result.values[1][1]).toBe(20);
    expect(result.values[1][2]).toBe(30);
    // C: [100, 200, 300]
    expect(Array.from(result.values[2])).toEqual([100, 200, 300]);
  });

  it("handles empty series mixed with populated series", () => {
    const input: NamedTimeSeries[] = [
      { label: "A", t: f64([1, 2]), values: f64([10, 20]) },
      { label: "empty", t: f64([]), values: f64([]) },
    ];
    const result = alignTimeSeries(input);
    expect(Array.from(result.t)).toEqual([1, 2]);
    expect(Array.from(result.values[0])).toEqual([10, 20]);
    // Empty series should be all NaN
    expect(result.values[1][0]).toBeNaN();
    expect(result.values[1][1]).toBeNaN();
  });

  it("merged time array is sorted even with unsorted inputs", () => {
    const input: NamedTimeSeries[] = [
      { label: "A", t: f64([3, 1]), values: f64([30, 10]) },
      { label: "B", t: f64([2]), values: f64([20]) },
    ];
    const result = alignTimeSeries(input);
    // Merged time should be sorted
    expect(Array.from(result.t)).toEqual([1, 2, 3]);
  });
});
