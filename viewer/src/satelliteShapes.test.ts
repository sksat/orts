import { beforeEach, describe, expect, it } from "vitest";
import {
  DEFAULT_ATTITUDE_SHAPE,
  isMarkerShape,
  type MarkerShape,
  readSatShapeParam,
  resolveMarkerShape,
  writeSatShapeParam,
} from "./satelliteShapes.js";

describe("isMarkerShape", () => {
  it("accepts known shapes and rejects others", () => {
    expect(isMarkerShape("sphere")).toBe(true);
    expect(isMarkerShape("axes-cube")).toBe(true);
    expect(isMarkerShape("box-cone")).toBe(false);
    expect(isMarkerShape("pyramid")).toBe(false);
    expect(isMarkerShape("")).toBe(false);
  });
});

describe("resolveMarkerShape precedence", () => {
  it("per-satellite override wins over everything", () => {
    // override = sphere beats sim shape, global default, and the automatic choice.
    expect(
      resolveMarkerShape({
        override: "sphere",
        simShape: "axes-cube",
        globalDefault: "axes-cube",
        hasAttitude: true,
      }),
    ).toBe("sphere");
  });

  it("sim-declared shape beats the global default and auto", () => {
    expect(
      resolveMarkerShape({ simShape: "sphere", globalDefault: "axes-cube", hasAttitude: true }),
    ).toBe("sphere");
  });

  it("global default applies when there is no override or sim shape", () => {
    expect(resolveMarkerShape({ globalDefault: "sphere", hasAttitude: true })).toBe("sphere");
  });

  it("auto: attitude → orientation-revealing cube, no attitude → sphere", () => {
    expect(resolveMarkerShape({ hasAttitude: true })).toBe(DEFAULT_ATTITUDE_SHAPE);
    expect(resolveMarkerShape({ hasAttitude: false })).toBe("sphere");
  });

  it("a null global default means auto (not a forced shape)", () => {
    const opts = { globalDefault: null, hasAttitude: true } satisfies {
      globalDefault: MarkerShape | null;
      hasAttitude: boolean;
    };
    expect(resolveMarkerShape(opts)).toBe(DEFAULT_ATTITUDE_SHAPE);
  });
});

describe("satShape URL param", () => {
  beforeEach(() => {
    history.replaceState(null, "", "/");
  });

  it("returns null when absent or invalid", () => {
    expect(readSatShapeParam()).toBeNull();
    history.replaceState(null, "", "/?satShape=pyramid");
    expect(readSatShapeParam()).toBeNull();
  });

  it("round-trips a valid shape and clears on null", () => {
    writeSatShapeParam("axes-cube");
    expect(window.location.search).toBe("?satShape=axes-cube");
    expect(readSatShapeParam()).toBe("axes-cube");

    writeSatShapeParam(null);
    expect(window.location.search).toBe("");
    expect(readSatShapeParam()).toBeNull();
  });
});

describe("a refused attitude", () => {
  it("takes the sphere over every request for a shape", () => {
    // The cube's faces answer "which way is the body pointing", and a sample
    // whose attitude the viewer could not use has no answer. So this outranks
    // the per-satellite override, the simulation's declaration and the global
    // default alike.
    for (const requested of ["axes-cube", "sphere"] as const) {
      expect(
        resolveMarkerShape({ override: requested, hasAttitude: true, attitudeRefused: true }),
      ).toBe("sphere");
      expect(
        resolveMarkerShape({ simShape: requested, hasAttitude: true, attitudeRefused: true }),
      ).toBe("sphere");
      expect(
        resolveMarkerShape({ globalDefault: requested, hasAttitude: true, attitudeRefused: true }),
      ).toBe("sphere");
    }
  });

  it("leaves a satellite with no attitude alone", () => {
    // Not the same case: nothing was claimed, so a requested cube still stands —
    // in the orbit view the marker is a position marker and reads as one.
    expect(
      resolveMarkerShape({ override: "axes-cube", hasAttitude: false, attitudeRefused: false }),
    ).toBe("axes-cube");
    expect(resolveMarkerShape({ override: "axes-cube", hasAttitude: false })).toBe("axes-cube");
  });
});
