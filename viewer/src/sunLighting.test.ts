import { describe, expect, it } from "vitest";
import type { LvlhAxes } from "./sceneFrame.js";
import { AU_KM, inverseSquareIntensity, sunDirectionInDisplayFrame } from "./sunLighting.js";

type Vec3 = [number, number, number];
const dot = (a: Vec3, b: Vec3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const mag = (v: Vec3): number => Math.sqrt(dot(v, v));

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

describe("sunDirectionInDisplayFrame", () => {
  const ECI_SUN: Vec3 = [0.6, 0, 0.8];

  it("returns the ECI direction unchanged for an inertial frame", () => {
    expect(
      sunDirectionInDisplayFrame(ECI_SUN, {
        isEcef: false,
        era: 1.234,
        lvlhActive: false,
        lvlhAxes: null,
      }),
    ).toEqual(ECI_SUN);
  });

  it("returns the ECI direction unchanged for ECEF when ERA is unavailable", () => {
    expect(
      sunDirectionInDisplayFrame(ECI_SUN, {
        isEcef: true,
        era: null,
        lvlhActive: false,
        lvlhAxes: null,
      }),
    ).toEqual(ECI_SUN);
  });

  it("rotates by -ERA about Z for the ECEF (Earth-fixed) frame", () => {
    // ERA = +π/2: an ECI +X sun maps to -Y in the Earth-fixed frame.
    const out = sunDirectionInDisplayFrame([1, 0, 0], {
      isEcef: true,
      era: Math.PI / 2,
      lvlhActive: false,
      lvlhAxes: null,
    });
    expect(out[0]).toBeCloseTo(0, 12);
    expect(out[1]).toBeCloseTo(-1, 12);
    expect(out[2]).toBeCloseTo(0, 12);
  });

  it("projects the sun onto the LVLH basis [inTrack, crossTrack, radial]", () => {
    // Permuted orthonormal basis: ECI +X lands on the radial (3rd) component.
    const axes: LvlhAxes = {
      inTrack: [0, 1, 0],
      crossTrack: [0, 0, 1],
      radial: [1, 0, 0],
    };
    const out = sunDirectionInDisplayFrame([1, 0, 0], {
      isEcef: false,
      era: undefined,
      lvlhActive: true,
      lvlhAxes: axes,
    });
    expect(out[0]).toBeCloseTo(0, 12);
    expect(out[1]).toBeCloseTo(0, 12);
    expect(out[2]).toBeCloseTo(1, 12);
  });

  it("LVLH takes precedence over the ECEF branch", () => {
    const axes: LvlhAxes = {
      inTrack: [1, 0, 0],
      crossTrack: [0, 1, 0],
      radial: [0, 0, 1],
    };
    // isEcef + era set, but lvlhActive wins → identity basis leaves it unchanged.
    expect(
      sunDirectionInDisplayFrame(ECI_SUN, {
        isEcef: true,
        era: Math.PI / 3,
        lvlhActive: true,
        lvlhAxes: axes,
      }),
    ).toEqual(ECI_SUN);
  });

  it("preserves magnitude under the orthonormal LVLH projection", () => {
    const axes = {
      inTrack: [0, 1, 0],
      crossTrack: [0, 0, 1],
      radial: [1, 0, 0],
    } satisfies LvlhAxes;
    const out = sunDirectionInDisplayFrame(ECI_SUN, {
      isEcef: false,
      era: undefined,
      lvlhActive: true,
      lvlhAxes: axes,
    });
    expect(mag(out as Vec3)).toBeCloseTo(mag(ECI_SUN), 12);
  });

  it("falls back to ECI when lvlhActive is true but axes are null", () => {
    expect(
      sunDirectionInDisplayFrame(ECI_SUN, {
        isEcef: false,
        era: undefined,
        lvlhActive: true,
        lvlhAxes: null,
      }),
    ).toEqual(ECI_SUN);
  });
});
