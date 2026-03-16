import { describe, expect, it } from "vitest";
import { computeTrueModelScale, getSatelliteModelConfig } from "./satelliteModels.js";

const EARTH_RADIUS_KM = 6378.137;

describe("getSatelliteModelConfig", () => {
  it("returns ISS config by id", () => {
    const config = getSatelliteModelConfig("iss");
    expect(config).not.toBeNull();
    expect(config?.physicalSpanKm).toBe(0.109);
  });

  it("returns ISS config by name pattern", () => {
    const config = getSatelliteModelConfig("25544", "ISS (ZARYA)");
    expect(config).not.toBeNull();
    expect(config?.physicalSpanKm).toBe(0.109);
  });

  it("returns null for unknown satellite", () => {
    expect(getSatelliteModelConfig("unknown")).toBeNull();
    expect(getSatelliteModelConfig("12345", "NOAA 19")).toBeNull();
  });
});

describe("computeTrueModelScale", () => {
  it("returns null when nativeSpanUnits is not set", () => {
    const config = {
      modelUrl: "test.glb",
      scale: 0.001,
      rotation: [0, 0, 0] as [number, number, number],
    };
    expect(computeTrueModelScale(config, EARTH_RADIUS_KM)).toBeNull();
  });

  it("returns null when nativeSpanUnits is zero", () => {
    const config = {
      modelUrl: "test.glb",
      scale: 0.001,
      rotation: [0, 0, 0] as [number, number, number],
      nativeSpanUnits: 0,
    };
    expect(computeTrueModelScale(config, EARTH_RADIUS_KM)).toBeNull();
  });

  it("computes correct true scale for ISS with Earth", () => {
    const config = getSatelliteModelConfig("iss")!;
    const trueScale = computeTrueModelScale(config, EARTH_RADIUS_KM)!;
    expect(trueScale).not.toBeNull();

    // At true scale, model span × trueScale should equal physical span in scene units
    // physicalSpanKm / centralBodyRadius = desired span in scene units
    const expectedSpanSceneUnits = 0.109 / EARTH_RADIUS_KM; // ~1.709e-5
    const actualSpanSceneUnits = config.nativeSpanUnits! * trueScale;
    expect(actualSpanSceneUnits).toBeCloseTo(expectedSpanSceneUnits, 10);
  });

  it("ISS true scale is much smaller than exaggerated scale", () => {
    const config = getSatelliteModelConfig("iss")!;
    const trueScale = computeTrueModelScale(config, EARTH_RADIUS_KM)!;
    // Exaggerated scale = 0.0003, true scale should be ~1.5e-7
    // Ratio (exaggeration factor) should be ~1966
    expect(config.scale / trueScale).toBeGreaterThan(1000);
    expect(config.scale / trueScale).toBeLessThan(3000);
  });

  it("true scale gives physically correct size ratio to Earth", () => {
    const config = getSatelliteModelConfig("iss")!;
    const trueScale = computeTrueModelScale(config, EARTH_RADIUS_KM)!;

    // ISS physical span = 109 m. Earth diameter = 12756.274 km = 12,756,274 m
    // Ratio: 109 / 12,756,274 ≈ 8.54e-6
    const physicalRatio = 109 / (EARTH_RADIUS_KM * 2 * 1000);

    // In scene units: satellite span = nativeSpanUnits * trueScale
    // Earth diameter = 2.0 (Earth radius = 1.0 in scene units)
    const sceneRatio = (config.nativeSpanUnits! * trueScale) / 2.0;

    expect(sceneRatio).toBeCloseTo(physicalRatio, 8);
  });

  it("scales inversely with central body radius", () => {
    const config = getSatelliteModelConfig("iss")!;
    const scaleEarth = computeTrueModelScale(config, EARTH_RADIUS_KM)!;
    const scaleMars = computeTrueModelScale(config, 3396.2)!;

    // Larger body radius → smaller true scale (satellite is smaller relative to body)
    expect(scaleEarth).toBeLessThan(scaleMars);
    expect(scaleEarth / scaleMars).toBeCloseTo(3396.2 / EARTH_RADIUS_KM, 5);
  });
});
