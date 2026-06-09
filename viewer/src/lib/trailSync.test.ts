import { describe, expect, it } from "vitest";
import { decideTrailUpdate, trailSyncState } from "./trailSync.js";
import type { TrailPoint } from "./types.js";

const pts = (...xs: number[]): TrailPoint[] => xs.map((x) => ({ position: [x, 0, 0] }));

describe("decideTrailUpdate", () => {
  it("first sync with no points is a no-op", () => {
    expect(decideTrailUpdate(null, [], undefined)).toEqual({ kind: "noop" });
  });

  it("first sync with points rebuilds", () => {
    expect(decideTrailUpdate(null, pts(1, 2, 3), undefined)).toEqual({ kind: "rebuild" });
  });

  it("unchanged trail is a no-op (so the GPU buffer is left alone)", () => {
    const prev = trailSyncState(pts(1, 2, 3), undefined);
    expect(decideTrailUpdate(prev, pts(1, 2, 3), undefined)).toEqual({ kind: "noop" });
  });

  it("appended points only upload the new tail", () => {
    const prev = trailSyncState(pts(1, 2, 3), undefined);
    expect(decideTrailUpdate(prev, pts(1, 2, 3, 4, 5), undefined)).toEqual({
      kind: "append",
      from: 3,
    });
  });

  it("a shorter trail rebuilds", () => {
    const prev = trailSyncState(pts(1, 2, 3, 4, 5), undefined);
    expect(decideTrailUpdate(prev, pts(1, 2, 3), undefined)).toEqual({ kind: "rebuild" });
  });

  it("a changed trailVersion rebuilds even at the same length", () => {
    const prev = trailSyncState(pts(1, 2, 3), "v1");
    expect(decideTrailUpdate(prev, pts(1, 2, 3), "v2")).toEqual({ kind: "rebuild" });
  });

  it("a rewritten history (tail at the old end no longer matches) rebuilds", () => {
    const prev = trailSyncState(pts(1, 2, 3), undefined);
    // length grew, but index 2 changed from 3 -> 9: not a clean append
    expect(decideTrailUpdate(prev, pts(1, 2, 9, 4, 5), undefined)).toEqual({ kind: "rebuild" });
  });
});

describe("trailSyncState", () => {
  it("captures length, version and the last position", () => {
    expect(trailSyncState(pts(1, 2, 3), "v1")).toEqual({
      length: 3,
      version: "v1",
      lastPosition: [3, 0, 0],
    });
  });

  it("has no last position for an empty trail", () => {
    expect(trailSyncState([], undefined)).toEqual({
      length: 0,
      version: undefined,
      lastPosition: undefined,
    });
  });
});
