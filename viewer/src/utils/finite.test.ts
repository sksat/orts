import { describe, expect, it } from "vitest";
import { finiteOrNull, firstFiniteTime } from "./finite.js";

describe("finiteOrNull", () => {
  it("keeps zero, which a falsy check would drop", () => {
    // t = 0 is the first sample of every run, not a missing value.
    expect(finiteOrNull(0)).toBe(0);
    expect(finiteOrNull(-0)).toBe(-0);
  });

  it("keeps ordinary and extreme finite values", () => {
    expect(finiteOrNull(2460000.5)).toBe(2460000.5);
    expect(finiteOrNull(Number.MAX_VALUE)).toBe(Number.MAX_VALUE);
    expect(finiteOrNull(Number.MIN_VALUE)).toBe(Number.MIN_VALUE);
  });

  it("rejects the non-finite values that would spread through the scene", () => {
    expect(finiteOrNull(Number.NaN)).toBeNull();
    expect(finiteOrNull(Number.POSITIVE_INFINITY)).toBeNull();
    expect(finiteOrNull(Number.NEGATIVE_INFINITY)).toBeNull();
  });

  it("passes absence through unchanged", () => {
    expect(finiteOrNull(null)).toBeNull();
    expect(finiteOrNull(undefined)).toBeNull();
  });
});

describe("firstFiniteTime", () => {
  it("skips a sample whose time is not a number and keeps looking", () => {
    // The case the scene got wrong twice: one satellite's `t` is NaN while
    // another carries a good time, and stopping at the first sample that exists
    // leaves the scene with no time rather than that one.
    expect(firstFiniteTime([{ t: Number.NaN }, { t: 120 }])).toBe(120);
    expect(firstFiniteTime([null, undefined, { t: Number.POSITIVE_INFINITY }, { t: 7 }])).toBe(7);
  });

  it("takes the first finite time it finds", () => {
    expect(firstFiniteTime([{ t: 0 }, { t: 120 }])).toBe(0);
    expect(firstFiniteTime([{ t: -30 }, { t: 120 }])).toBe(-30);
  });

  it("reports none when nothing carries a time", () => {
    expect(firstFiniteTime(undefined)).toBeNull();
    expect(firstFiniteTime([])).toBeNull();
    expect(firstFiniteTime([null, { t: Number.NaN }, {}])).toBeNull();
  });
});
