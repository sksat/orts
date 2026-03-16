import { describe, expect, it } from "vitest";
import { computeTMin, DISPLAY_MAX_POINTS, type TimeRange } from "./useTimeSeriesStore.js";

describe("computeTMin", () => {
  it("returns undefined when timeRange is null (show all)", () => {
    expect(computeTMin(null, 1000)).toBeUndefined();
  });

  it("returns latestT - timeRange for a positive range", () => {
    // 5 min window, latest at t=1000
    expect(computeTMin(300, 1000)).toBe(700);
  });

  it("returns negative tMin when timeRange exceeds latestT", () => {
    // 5 min window but only 100s of data -> tMin = -200 (means show all)
    expect(computeTMin(300, 100)).toBe(-200);
  });

  it("returns latestT when timeRange is 0", () => {
    // Edge case: show nothing
    expect(computeTMin(0, 1000)).toBe(1000);
  });

  it("handles -Infinity latestT (no data yet)", () => {
    expect(computeTMin(300, -Infinity)).toBe(-Infinity);
  });
});

describe("DISPLAY_MAX_POINTS", () => {
  it("is a reasonable display budget for charts", () => {
    expect(DISPLAY_MAX_POINTS).toBeGreaterThanOrEqual(500);
    expect(DISPLAY_MAX_POINTS).toBeLessThanOrEqual(5000);
  });
});

describe("TimeRange type", () => {
  it("accepts null for showing all data", () => {
    const range: TimeRange = null;
    expect(range).toBeNull();
  });

  it("accepts a number for time window in seconds", () => {
    const range: TimeRange = 300;
    expect(range).toBe(300);
  });
});
