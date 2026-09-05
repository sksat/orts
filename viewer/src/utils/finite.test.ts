import { describe, expect, it } from "vitest";
import { finiteOrNull } from "./finite.js";

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
