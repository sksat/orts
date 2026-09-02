import { describe, expect, it } from "vitest";
import { AU_KM, inverseSquareIntensity } from "./sunLighting.js";

describe("inverseSquareIntensity", () => {
  it("is 1.0 at exactly 1 AU", () => {
    expect(inverseSquareIntensity(AU_KM)).toBeCloseTo(1.0, 12);
  });

  it("follows the inverse-square law (2 AU → 1/4, 0.5 AU → 4)", () => {
    expect(inverseSquareIntensity(2 * AU_KM)).toBeCloseTo(0.25, 12);
    expect(inverseSquareIntensity(0.5 * AU_KM)).toBeCloseTo(4.0, 12);
  });

  it("clamps non-positive distance to unit intensity (no Infinity/NaN)", () => {
    expect(inverseSquareIntensity(0)).toBe(1.0);
    expect(inverseSquareIntensity(-1)).toBe(1.0);
  });
});
