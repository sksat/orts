import { describe, it, expect } from "vitest";
import { computeTMin, type TimeRange } from "./useOrbitCharts.js";

describe("computeTMin", () => {
  it("returns undefined when timeRange is null (show all)", () => {
    expect(computeTMin(null, 1000)).toBeUndefined();
  });

  it("returns latestT - timeRange for a positive range", () => {
    // 5 min window, latest at t=1000
    expect(computeTMin(300, 1000)).toBe(700);
  });

  it("returns negative tMin when timeRange exceeds latestT", () => {
    // 5 min window but only 100s of data → tMin = -200 (means show all)
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
