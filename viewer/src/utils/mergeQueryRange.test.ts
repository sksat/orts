import { describe, expect, it } from "vitest";
import { mergeQueryRangePoints, pickTrailBufferForResponse } from "./mergeQueryRange.js";

function makePoint(t: number, entityPath?: string) {
  return {
    t,
    entityPath,
    x: t,
    y: 0,
    z: 0,
    vx: 0,
    vy: 0,
    vz: 0,
    a: 0,
    e: 0,
    inc: 0,
    raan: 0,
    omega: 0,
    nu: 0,
  };
}

interface FakeTrailBuffer {
  id: string;
  getAll: () => ReturnType<typeof makePoint>[];
}

function fakeTrail(id: string, points: ReturnType<typeof makePoint>[]): FakeTrailBuffer {
  return { id, getAll: () => points };
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

describe("pickTrailBufferForResponse", () => {
  // Regression: `handleQueryRangeResponse` used to hardcode
  // `simInfo.satellites[0].id` as the merge target, so a multi-sat
  // response for sat B would be merged against sat A's tail and then
  // dispatched as a rebuild of sat B's trail buffer — contaminating
  // sat B with sat A's 3D position points.

  it("returns the trail buffer matching the response's entity path", () => {
    const sat1 = fakeTrail("sat1", [makePoint(100, "sat1")]);
    const sat2 = fakeTrail("sat2", [makePoint(200, "sat2")]);
    const buffers = new Map([
      ["sat1", sat1],
      ["sat2", sat2],
    ]);

    const response = [makePoint(50, "sat2"), makePoint(100, "sat2")];
    const picked = pickTrailBufferForResponse(response, buffers, "sat1");
    expect(picked?.id).toBe("sat2");
  });

  it("falls back to the provided satId when the response has no entityPath", () => {
    const sat1 = fakeTrail("sat1", []);
    const buffers = new Map([["sat1", sat1]]);

    const response = [makePoint(50)]; // no entityPath
    const picked = pickTrailBufferForResponse(response, buffers, "sat1");
    expect(picked?.id).toBe("sat1");
  });

  it("returns null when the target trail buffer is absent from the map", () => {
    const buffers = new Map<string, FakeTrailBuffer>();
    const response = [makePoint(50, "sat-unknown")];
    expect(pickTrailBufferForResponse(response, buffers, "sat1")).toBeNull();
  });

  it("returns null when the response is empty and no fallback is provided", () => {
    const buffers = new Map<string, FakeTrailBuffer>();
    expect(pickTrailBufferForResponse([], buffers, null)).toBeNull();
  });

  it("does NOT use the fallback satId when the response has an entityPath (regression: #1)", () => {
    // The pre-fix bug: the fallback was unconditional, so a sat2
    // response merged against sat1's tail. Ensure that path is gone.
    const sat1 = fakeTrail("sat1", [makePoint(999, "sat1")]);
    const sat2 = fakeTrail("sat2", [makePoint(200, "sat2")]);
    const buffers = new Map([
      ["sat1", sat1],
      ["sat2", sat2],
    ]);

    const response = [makePoint(50, "sat2")];
    const picked = pickTrailBufferForResponse(response, buffers, "sat1");
    expect(picked?.id).toBe("sat2");
    // Verify the fallback would have picked sat1 — sanity check that
    // this test actually distinguishes the two.
    expect(picked?.id).not.toBe("sat1");
  });
});
