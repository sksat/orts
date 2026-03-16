import { describe, expect, it } from "vitest";
import { mergeQueryRangePoints } from "./mergeQueryRange.js";

function makePoint(t: number) {
  return { t, x: t, y: 0, z: 0, vx: 0, vy: 0, vz: 0, a: 0, e: 0, inc: 0, raan: 0, omega: 0, nu: 0 };
}

describe("mergeQueryRangePoints", () => {
  it("preserves streaming points newer than response", () => {
    const response = [makePoint(100), makePoint(200), makePoint(300)];
    const trail = [makePoint(100), makePoint(200), makePoint(300), makePoint(400), makePoint(500)];
    const merged = mergeQueryRangePoints(response, trail);
    expect(merged).toHaveLength(5);
    expect(merged[merged.length - 1].t).toBe(500);
    expect(merged[merged.length - 2].t).toBe(400);
  });

  it("returns only response when no newer trail points exist", () => {
    const response = [makePoint(100), makePoint(200), makePoint(500)];
    const trail = [makePoint(100), makePoint(200), makePoint(500)];
    const merged = mergeQueryRangePoints(response, trail);
    expect(merged).toHaveLength(3);
    expect(merged[merged.length - 1].t).toBe(500);
  });

  it("handles empty response", () => {
    const trail = [makePoint(100), makePoint(200)];
    const merged = mergeQueryRangePoints([], trail);
    expect(merged).toHaveLength(2);
    expect(merged[0].t).toBe(100);
  });

  it("handles empty trail", () => {
    const response = [makePoint(100), makePoint(200)];
    const merged = mergeQueryRangePoints(response, []);
    expect(merged).toHaveLength(2);
  });

  it("latest is always the true latest from both sources", () => {
    // Simulate: user zooms to old range [100,300], but streaming is at t=500
    const response = [makePoint(100), makePoint(200), makePoint(300)];
    const trail = [makePoint(200), makePoint(300), makePoint(400), makePoint(500)];
    const merged = mergeQueryRangePoints(response, trail);
    expect(merged[merged.length - 1].t).toBe(500);
  });
});
