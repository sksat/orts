import { describe, expect, it } from "vitest";
import { TrailBuffer } from "../utils/TrailBuffer.js";
import { reconcileTrailEntry, type TrailEntry } from "./trailReconcile.js";
import type { TrailPoint } from "./types.js";

const pts = (...xs: number[]): TrailPoint[] => xs.map((x, i) => ({ position: [x, 0, 0], time: i }));
const entry = (): TrailEntry => ({ buffer: new TrailBuffer(64), sync: null });

describe("reconcileTrailEntry", () => {
  it("first reconcile fills the buffer with the points", () => {
    const e = entry();
    reconcileTrailEntry(e, pts(1, 2, 3), undefined);
    expect(e.buffer.length).toBe(3);
    expect(e.buffer.getAll().map((p) => p.x)).toEqual([1, 2, 3]);
    expect(e.buffer.getAll().map((p) => p.t)).toEqual([0, 1, 2]);
  });

  it("an extended array appends only the tail and keeps the generation", () => {
    const e = entry();
    reconcileTrailEntry(e, pts(1, 2), undefined);
    const gen = e.buffer.generation;
    reconcileTrailEntry(e, pts(1, 2, 3, 4), undefined);
    expect(e.buffer.length).toBe(4);
    expect(e.buffer.generation).toBe(gen); // incremental append, no GPU rebuild
    expect(e.buffer.getAll().map((p) => p.x)).toEqual([1, 2, 3, 4]);
  });

  it("is idempotent: repeating the same inputs is a no-op (StrictMode-safe)", () => {
    const e = entry();
    const points = pts(1, 2, 3);
    reconcileTrailEntry(e, points, undefined);
    const gen = e.buffer.generation;
    reconcileTrailEntry(e, points, undefined); // double-invoked effect
    expect(e.buffer.length).toBe(3); // no double-append
    expect(e.buffer.generation).toBe(gen);
  });

  it("a version bump rebuilds (contents replaced, generation bumped)", () => {
    const e = entry();
    reconcileTrailEntry(e, pts(1, 2, 3), "v1");
    const gen = e.buffer.generation;
    reconcileTrailEntry(e, pts(7, 8), "v2");
    expect(e.buffer.length).toBe(2);
    expect(e.buffer.getAll().map((p) => p.x)).toEqual([7, 8]);
    expect(e.buffer.generation).toBeGreaterThan(gen); // full GPU rewrite signalled
  });

  it("a shrink rebuilds", () => {
    const e = entry();
    reconcileTrailEntry(e, pts(1, 2, 3, 4), undefined);
    reconcileTrailEntry(e, pts(1, 2), undefined);
    expect(e.buffer.length).toBe(2);
    expect(e.buffer.getAll().map((p) => p.x)).toEqual([1, 2]);
  });

  it("empty first reconcile stays empty and later points still fill", () => {
    const e = entry();
    reconcileTrailEntry(e, [], undefined);
    expect(e.buffer.length).toBe(0);
    reconcileTrailEntry(e, pts(5), undefined);
    expect(e.buffer.getAll().map((p) => p.x)).toEqual([5]);
  });
});
