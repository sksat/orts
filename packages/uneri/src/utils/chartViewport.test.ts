import { describe, it, expect } from "vitest";
import {
  sliceArrays,
  lowerBound,
  upperBound,
  quantizeChartTime,
} from "./chartViewport.js";

/** Helper: create N Float64Arrays where the first is the time column. */
function makeArrays(
  times: number[],
  columnCount: number = 3,
): Float64Array[] {
  const arrays: Float64Array[] = [];
  const t = new Float64Array(times);
  arrays.push(t);
  for (let c = 1; c < columnCount; c++) {
    arrays.push(new Float64Array(times.map((v) => v * (c * 10))));
  }
  return arrays;
}

describe("sliceArrays", () => {
  it("returns null for null input", () => {
    expect(sliceArrays(null, undefined, null)).toBeNull();
  });

  it("returns same data when no time constraint", () => {
    const data = makeArrays([0, 10, 20, 30]);
    const result = sliceArrays(data, undefined, null)!;
    expect(result[0].length).toBe(4);
    expect(result[0][0]).toBe(0);
    expect(result[0][3]).toBe(30);
  });

  it("with currentTime clips to right edge", () => {
    const data = makeArrays([0, 10, 20, 30, 40, 50]);
    const result = sliceArrays(data, 25, null)!;
    // upperBound finds first index with t > 25, which is index 3 (t=30)
    // so subarray(0, 3) → points at t=0, 10, 20
    expect(result[0].length).toBe(3);
    expect(result[0][0]).toBe(0);
    expect(result[0][2]).toBe(20);
    // Verify all columns are sliced consistently
    expect(result[1].length).toBe(3);
    expect(result[1][2]).toBe(200); // 20 * 10
  });

  it("with timeRange clips to left edge (sliding window)", () => {
    const data = makeArrays([0, 10, 20, 30, 40, 50]);
    // No currentTime → window is [tMax - timeRange, tMax] = [25, 50]
    // lowerBound finds first index with t >= 25, which is index 3 (t=30)
    const result = sliceArrays(data, undefined, 25)!;
    expect(result[0].length).toBe(3);
    expect(result[0][0]).toBe(30);
    expect(result[0][2]).toBe(50);
  });

  it("with both currentTime and timeRange", () => {
    const data = makeArrays([0, 10, 20, 30, 40, 50]);
    // currentTime=40, timeRange=15 → window [25, 40]
    // lowerBound(25) → index 3 (t=30), upperBound(40) → index 5 (t=50 is first > 40)
    // subarray(3, 5) → points at t=30, t=40
    const result = sliceArrays(data, 40, 15)!;
    expect(result[0].length).toBe(2);
    expect(result[0][0]).toBe(30);
    expect(result[0][1]).toBe(40);
  });

  it("zero-copy: returns subarrays sharing the original buffer", () => {
    const data = makeArrays([0, 10, 20, 30, 40, 50]);
    const result = sliceArrays(data, 30, null)!;
    expect(result[0].buffer).toBe(data[0].buffer);
    expect(result[1].buffer).toBe(data[1].buffer);
    expect(result[2].buffer).toBe(data[2].buffer);
  });

  it("with empty arrays", () => {
    const data = makeArrays([]);
    const result = sliceArrays(data, 10, null)!;
    expect(result[0].length).toBe(0);
  });

  it("works with 2 columns", () => {
    const data = makeArrays([0, 10, 20, 30], 2);
    expect(data.length).toBe(2);
    const result = sliceArrays(data, 15, null)!;
    expect(result.length).toBe(2);
    expect(result[0].length).toBe(2); // t=0, t=10
    expect(result[1].length).toBe(2);
  });

  it("works with 5 columns", () => {
    const data = makeArrays([0, 10, 20, 30, 40], 5);
    expect(data.length).toBe(5);
    const result = sliceArrays(data, 25, 10)!;
    // window [15, 25] → lowerBound(15)=2 (t=20), upperBound(25)=3 (t=30 first > 25)
    // subarray(2, 3) → just t=20
    expect(result.length).toBe(5);
    for (const arr of result) {
      expect(arr.length).toBe(1);
    }
    expect(result[0][0]).toBe(20);
  });

  it("works with 10 columns", () => {
    const data = makeArrays([0, 5, 10, 15, 20], 10);
    expect(data.length).toBe(10);
    const result = sliceArrays(data, undefined, null)!;
    expect(result.length).toBe(10);
    for (const arr of result) {
      expect(arr.length).toBe(5);
    }
  });

  it("returns empty-like data when currentTime is before all points", () => {
    const data = makeArrays([10, 20, 30]);
    const result = sliceArrays(data, 5, null)!;
    expect(result[0].length).toBe(0);
  });

  it("includes exact currentTime match", () => {
    const data = makeArrays([0, 10, 20, 30]);
    const result = sliceArrays(data, 20, null)!;
    // t=20 is included (t <= 20)
    expect(result[0].length).toBe(3);
    expect(result[0][2]).toBe(20);
  });
});

describe("lowerBound", () => {
  it("finds first index >= value", () => {
    const arr = new Float64Array([0, 10, 20, 30, 40, 50]);
    // value=25 → first index where arr[i] >= 25 is index 3 (t=30)
    expect(lowerBound(arr, 25, 0, arr.length)).toBe(3);
  });

  it("returns 0 when value is before all elements", () => {
    const arr = new Float64Array([10, 20, 30]);
    expect(lowerBound(arr, 5, 0, arr.length)).toBe(0);
  });

  it("returns length when value is after all elements", () => {
    const arr = new Float64Array([10, 20, 30]);
    expect(lowerBound(arr, 35, 0, arr.length)).toBe(3);
  });

  it("returns exact index for exact match", () => {
    const arr = new Float64Array([0, 10, 20, 30, 40]);
    expect(lowerBound(arr, 20, 0, arr.length)).toBe(2);
  });

  it("respects lo/hi bounds", () => {
    const arr = new Float64Array([0, 10, 20, 30, 40]);
    // Search only in [2, 4) → arr[2]=20, arr[3]=30
    expect(lowerBound(arr, 25, 2, 4)).toBe(3);
  });

  it("handles empty range", () => {
    const arr = new Float64Array([0, 10, 20]);
    expect(lowerBound(arr, 5, 2, 2)).toBe(2);
  });
});

describe("upperBound", () => {
  it("finds first index > value", () => {
    const arr = new Float64Array([0, 10, 20, 30, 40, 50]);
    // value=20 → first index where arr[i] > 20 is index 3 (t=30)
    expect(upperBound(arr, 20, 0, arr.length)).toBe(3);
  });

  it("returns 0 when value is before all elements", () => {
    const arr = new Float64Array([10, 20, 30]);
    expect(upperBound(arr, 5, 0, arr.length)).toBe(0);
  });

  it("returns length when value >= all elements", () => {
    const arr = new Float64Array([10, 20, 30]);
    expect(upperBound(arr, 30, 0, arr.length)).toBe(3);
  });

  it("returns index after exact match (differs from lowerBound)", () => {
    const arr = new Float64Array([0, 10, 20, 30, 40]);
    // lowerBound(20) = 2, upperBound(20) = 3
    expect(upperBound(arr, 20, 0, arr.length)).toBe(3);
    expect(lowerBound(arr, 20, 0, arr.length)).toBe(2);
  });

  it("respects lo/hi bounds", () => {
    const arr = new Float64Array([0, 10, 20, 30, 40]);
    expect(upperBound(arr, 20, 1, 4)).toBe(3);
  });

  it("handles empty range", () => {
    const arr = new Float64Array([0, 10, 20]);
    expect(upperBound(arr, 5, 2, 2)).toBe(2);
  });
});

describe("quantizeChartTime", () => {
  it("snaps to 0.5s steps", () => {
    expect(quantizeChartTime(0.0)).toBe(0);
    expect(quantizeChartTime(0.24)).toBe(0);
    expect(quantizeChartTime(0.25)).toBe(0.5);
    expect(quantizeChartTime(0.49)).toBe(0.5);
    expect(quantizeChartTime(0.74)).toBe(0.5);
    expect(quantizeChartTime(0.75)).toBe(1.0);
    expect(quantizeChartTime(1.0)).toBe(1.0);
  });

  it("returns undefined for undefined input", () => {
    expect(quantizeChartTime(undefined)).toBeUndefined();
  });

  it("produces identical output for inputs within the same quantum", () => {
    const a = quantizeChartTime(5.1);
    const b = quantizeChartTime(5.2);
    const c = quantizeChartTime(5.24);
    expect(a).toBe(b);
    expect(b).toBe(c);
  });

  it("limits chart update frequency: 60fps over 10s produces <= 21 unique values", () => {
    const uniqueValues = new Set<number>();
    for (let frame = 0; frame < 600; frame++) {
      const elapsed = frame / 60;
      uniqueValues.add(quantizeChartTime(elapsed)!);
    }
    expect(uniqueValues.size).toBeLessThanOrEqual(21);
    expect(uniqueValues.size).toBeGreaterThan(0);
  });
});
