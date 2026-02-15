import { describe, it, expect } from "vitest";
import { getDisplayScaleProfile, computeSceneAmplification } from "./displayScale.js";
import { getSatelliteModelConfig, computeTrueModelScale } from "./satelliteModels.js";
import { transformToLvlh } from "./coordTransform.js";
import type { LvlhAxes } from "./sceneFrame.js";
import type { FrameCenter } from "./referenceFrame.js";

const EARTH_RADIUS_KM = 6378.137;

// LVLH axes for equatorial circular orbit: satellite at +X, velocity +Y
const equatorialAxes: LvlhAxes = {
  radial: [1, 0, 0],
  inTrack: [0, 1, 0],
  crossTrack: [0, 0, 1],
};

describe("getDisplayScaleProfile", () => {
  it("returns body-centered profile for central_body", () => {
    const profile = getDisplayScaleProfile({ type: "central_body" });
    expect(profile.name).toBe("body-centered");
  });

  it("returns satellite-centered profile for satellite", () => {
    const profile = getDisplayScaleProfile({ type: "satellite", id: "iss" });
    expect(profile.name).toBe("satellite-centered");
  });

  it("satellite-centered profile has default camera direction along -X (behind satellite)", () => {
    const profile = getDisplayScaleProfile({ type: "satellite", id: "iss" });
    expect(profile.defaultCameraDirection).toEqual([-1, 0, 0]);
  });

  it("body-centered profile has no default camera direction", () => {
    const profile = getDisplayScaleProfile({ type: "central_body" });
    expect(profile.defaultCameraDirection).toBeNull();
  });
});

describe("computeSceneAmplification", () => {
  it("ISS amplification equals exaggerated/true scale ratio", () => {
    const config = getSatelliteModelConfig("iss")!;
    const amplification = computeSceneAmplification(config, EARTH_RADIUS_KM);
    const trueScale = computeTrueModelScale(config, EARTH_RADIUS_KM)!;
    expect(amplification).toBeCloseTo(config.scale / trueScale, 5);
  });

  it("sphere fallback amplification equals exaggerated/true radius ratio", () => {
    // Unknown satellite → sphere fallback
    const amplification = computeSceneAmplification(null, EARTH_RADIUS_KM);
    // True radius = 10m / 6378.137km in scene units
    const trueRadius = 0.010 / EARTH_RADIUS_KM;
    // Exaggerated radius = 0.005 (body-centered sphere radius)
    const expectedAmp = 0.005 / trueRadius;
    expect(amplification).toBeCloseTo(expectedAmp, 5);
  });

  it("amplification is always > 1 (environment is enlarged)", () => {
    const issConfig = getSatelliteModelConfig("iss")!;
    expect(computeSceneAmplification(issConfig, EARTH_RADIUS_KM)).toBeGreaterThan(1);
    expect(computeSceneAmplification(null, EARTH_RADIUS_KM)).toBeGreaterThan(1);
  });
});

/**
 * Physical accuracy tests.
 *
 * These verify that the amplified satellite-centered scene produces
 * geometrically correct angular sizes and distance ratios as seen
 * from the satellite, independent of the amplification factor.
 *
 * Key invariant: amplification cancels out in angular computations.
 *   halfAngle = arcsin(earthSceneRadius / earthSceneDistance)
 *             = arcsin(amp / (r * amp / R))
 *             = arcsin(R / r)
 *   which is the exact physical formula.
 */
describe("physical accuracy: angular size of Earth", () => {
  /**
   * Compute the half-angle subtended by Earth from a satellite at the
   * given altitude, using the same math as the viewer scene.
   *
   * Scene setup (mirroring Scene.tsx):
   *   effectiveScaleRadius = centralBodyRadius / amplification
   *   Earth center LVLH position = transformToLvlh(0,0,0, satPos, axes, effectiveScaleRadius)
   *   Earth scene radius = amplification
   *   halfAngle = arcsin(earthSceneRadius / distance(earthScenePos, origin))
   */
  function computeSceneHalfAngle(
    altitudeKm: number,
    modelConfig: ReturnType<typeof getSatelliteModelConfig>,
  ): number {
    const r = EARTH_RADIUS_KM + altitudeKm;
    const amp = computeSceneAmplification(modelConfig, EARTH_RADIUS_KM);
    const effectiveScaleRadius = EARTH_RADIUS_KM / amp;

    // Satellite at [r, 0, 0] in ECI, Earth center at [0, 0, 0]
    const satPos: [number, number, number] = [r, 0, 0];
    const earthScenePos = transformToLvlh(0, 0, 0, satPos, equatorialAxes, effectiveScaleRadius);

    const earthSceneDistance = Math.sqrt(
      earthScenePos[0] ** 2 + earthScenePos[1] ** 2 + earthScenePos[2] ** 2,
    );
    const earthSceneRadius = amp;

    return Math.asin(earthSceneRadius / earthSceneDistance);
  }

  /** Physical half-angle: arcsin(R / (R + h)). */
  function physicalHalfAngle(altitudeKm: number): number {
    return Math.asin(EARTH_RADIUS_KM / (EARTH_RADIUS_KM + altitudeKm));
  }

  const issConfig = getSatelliteModelConfig("iss");

  it("ISS altitude (~400 km): Earth angular size matches physics", () => {
    const sceneAngle = computeSceneHalfAngle(400, issConfig);
    const physAngle = physicalHalfAngle(400);

    // Expected: arcsin(6378.137 / 6778.137) ≈ 70.2°
    const expectedDeg = 70.2;
    expect(sceneAngle * 180 / Math.PI).toBeCloseTo(expectedDeg, 0);
    expect(sceneAngle).toBeCloseTo(physAngle, 10);
  });

  it("LEO 200 km: Earth angular size matches physics", () => {
    const sceneAngle = computeSceneHalfAngle(200, issConfig);
    const physAngle = physicalHalfAngle(200);
    expect(sceneAngle).toBeCloseTo(physAngle, 10);
  });

  it("GEO ~35786 km: Earth angular size matches physics", () => {
    const sceneAngle = computeSceneHalfAngle(35786, issConfig);
    const physAngle = physicalHalfAngle(35786);

    // Expected: arcsin(6378 / 42164) ≈ 8.7°
    const expectedDeg = 8.7;
    expect(sceneAngle * 180 / Math.PI).toBeCloseTo(expectedDeg, 0);
    expect(sceneAngle).toBeCloseTo(physAngle, 10);
  });

  it("sphere fallback also gives correct angular size", () => {
    // Unknown satellite, sphere fallback
    const sceneAngle = computeSceneHalfAngle(400, null);
    const physAngle = physicalHalfAngle(400);
    expect(sceneAngle).toBeCloseTo(physAngle, 10);
  });

  it("angular size is independent of amplification factor", () => {
    // The cancellation is algebraically exact, but verify numerically
    // that different satellite configs (different amplifications) yield
    // the same angular size for the same altitude.
    const angleIss = computeSceneHalfAngle(400, issConfig);
    const angleFallback = computeSceneHalfAngle(400, null);
    expect(angleIss).toBeCloseTo(angleFallback, 10);
  });
});

describe("physical accuracy: distance ratios", () => {
  /**
   * Compute the scene-distance between two ECI points as seen in
   * the amplified LVLH frame.
   */
  function sceneDistance(
    aEci: [number, number, number],
    bEci: [number, number, number],
    satPos: [number, number, number],
    effectiveScaleRadius: number,
  ): number {
    const aLvlh = transformToLvlh(...aEci, satPos, equatorialAxes, effectiveScaleRadius);
    const bLvlh = transformToLvlh(...bEci, satPos, equatorialAxes, effectiveScaleRadius);
    return Math.sqrt(
      (aLvlh[0] - bLvlh[0]) ** 2 +
      (aLvlh[1] - bLvlh[1]) ** 2 +
      (aLvlh[2] - bLvlh[2]) ** 2,
    );
  }

  function eciDistance(a: [number, number, number], b: [number, number, number]): number {
    return Math.sqrt((a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2 + (a[2] - b[2]) ** 2);
  }

  it("distance ratio between two points is preserved in amplified scene", () => {
    const issConfig = getSatelliteModelConfig("iss");
    const amp = computeSceneAmplification(issConfig, EARTH_RADIUS_KM);
    const effectiveScaleRadius = EARTH_RADIUS_KM / amp;

    const satPos: [number, number, number] = [6778, 0, 0];

    // Two points along the orbit
    const pointA: [number, number, number] = [6778, 100, 0]; // 100 km ahead
    const pointB: [number, number, number] = [6778, 300, 0]; // 300 km ahead

    const sceneDistA = sceneDistance(satPos, pointA, satPos, effectiveScaleRadius);
    const sceneDistB = sceneDistance(satPos, pointB, satPos, effectiveScaleRadius);
    const eciDistA = eciDistance(satPos, pointA);
    const eciDistB = eciDistance(satPos, pointB);

    // Ratio should be preserved: 300/100 = 3.0
    expect(sceneDistB / sceneDistA).toBeCloseTo(eciDistB / eciDistA, 10);
  });

  it("altitude difference maps to correct scene distance ratio", () => {
    const issConfig = getSatelliteModelConfig("iss");
    const amp = computeSceneAmplification(issConfig, EARTH_RADIUS_KM);
    const effectiveScaleRadius = EARTH_RADIUS_KM / amp;

    const satPos: [number, number, number] = [6778, 0, 0];

    // Point 50 km above and 100 km above the satellite (radially outward)
    const above50: [number, number, number] = [6828, 0, 0];
    const above100: [number, number, number] = [6878, 0, 0];

    const sceneDist50 = sceneDistance(satPos, above50, satPos, effectiveScaleRadius);
    const sceneDist100 = sceneDistance(satPos, above100, satPos, effectiveScaleRadius);

    // Physical ratio: 100 km / 50 km = 2.0
    expect(sceneDist100 / sceneDist50).toBeCloseTo(2.0, 10);
  });
});

describe("physical accuracy: angular separation between points", () => {
  it("angular separation between two trail points matches physical angle", () => {
    const issConfig = getSatelliteModelConfig("iss");
    const amp = computeSceneAmplification(issConfig, EARTH_RADIUS_KM);
    const effectiveScaleRadius = EARTH_RADIUS_KM / amp;

    const satPos: [number, number, number] = [6778, 0, 0];

    // Two points on the orbit: 100 km and 200 km ahead along in-track
    const p1Eci: [number, number, number] = [6778, 100, 0];
    const p2Eci: [number, number, number] = [6778, 200, 0];

    const p1Lvlh = transformToLvlh(...p1Eci, satPos, equatorialAxes, effectiveScaleRadius);
    const p2Lvlh = transformToLvlh(...p2Eci, satPos, equatorialAxes, effectiveScaleRadius);

    // Angular separation as seen from origin (satellite)
    const dot12 = p1Lvlh[0] * p2Lvlh[0] + p1Lvlh[1] * p2Lvlh[1] + p1Lvlh[2] * p2Lvlh[2];
    const mag1 = Math.sqrt(p1Lvlh[0] ** 2 + p1Lvlh[1] ** 2 + p1Lvlh[2] ** 2);
    const mag2 = Math.sqrt(p2Lvlh[0] ** 2 + p2Lvlh[1] ** 2 + p2Lvlh[2] ** 2);
    const sceneAngle = Math.acos(dot12 / (mag1 * mag2));

    // Physical angular separation from satellite
    const d1 = [100, 0, 0]; // relative to sat
    const d2 = [200, 0, 0];
    // Both along the same axis, so angular separation is 0. Let's use offset points instead.
    // Actually p1 and p2 are both along in-track from satPos, so they're collinear from sat.
    // Use a cross-track offset to get a real angle.
    const p1off: [number, number, number] = [6778, 100, 50];
    const p2off: [number, number, number] = [6778, 200, 0];

    const p1offLvlh = transformToLvlh(...p1off, satPos, equatorialAxes, effectiveScaleRadius);
    const p2offLvlh = transformToLvlh(...p2off, satPos, equatorialAxes, effectiveScaleRadius);

    const dotOff = p1offLvlh[0] * p2offLvlh[0] + p1offLvlh[1] * p2offLvlh[1] + p1offLvlh[2] * p2offLvlh[2];
    const mag1off = Math.sqrt(p1offLvlh[0] ** 2 + p1offLvlh[1] ** 2 + p1offLvlh[2] ** 2);
    const mag2off = Math.sqrt(p2offLvlh[0] ** 2 + p2offLvlh[1] ** 2 + p2offLvlh[2] ** 2);
    const sceneAngleOff = Math.acos(dotOff / (mag1off * mag2off));

    // Physical angle: d1 = [0, 100, 50], d2 = [0, 200, 0] (relative to sat in ECI)
    const phys1 = [0, 100, 50];
    const phys2 = [0, 200, 0];
    const physDot = phys1[0] * phys2[0] + phys1[1] * phys2[1] + phys1[2] * phys2[2];
    const physMag1 = Math.sqrt(phys1[0] ** 2 + phys1[1] ** 2 + phys1[2] ** 2);
    const physMag2 = Math.sqrt(phys2[0] ** 2 + phys2[1] ** 2 + phys2[2] ** 2);
    const physAngle = Math.acos(physDot / (physMag1 * physMag2));

    expect(sceneAngleOff).toBeCloseTo(physAngle, 10);
  });

  it("angular size of Earth surface feature matches physical angle", () => {
    // A 1000 km wide feature on Earth's surface, as seen from ISS
    const issConfig = getSatelliteModelConfig("iss");
    const amp = computeSceneAmplification(issConfig, EARTH_RADIUS_KM);
    const effectiveScaleRadius = EARTH_RADIUS_KM / amp;

    const satPos: [number, number, number] = [6778, 0, 0];

    // Two points on Earth's surface at radius 6378 km, separated by ~1000 km
    // Place them symmetrically along Y axis from sub-satellite point
    const halfAngle = 500 / EARTH_RADIUS_KM; // ~0.0784 rad
    const surfA: [number, number, number] = [
      EARTH_RADIUS_KM * Math.cos(halfAngle),
      EARTH_RADIUS_KM * Math.sin(halfAngle),
      0,
    ];
    const surfB: [number, number, number] = [
      EARTH_RADIUS_KM * Math.cos(halfAngle),
      -EARTH_RADIUS_KM * Math.sin(halfAngle),
      0,
    ];

    // Scene coordinates
    const aLvlh = transformToLvlh(...surfA, satPos, equatorialAxes, effectiveScaleRadius);
    const bLvlh = transformToLvlh(...surfB, satPos, equatorialAxes, effectiveScaleRadius);

    // Angular separation from satellite
    const dotAB = aLvlh[0] * bLvlh[0] + aLvlh[1] * bLvlh[1] + aLvlh[2] * bLvlh[2];
    const magA = Math.sqrt(aLvlh[0] ** 2 + aLvlh[1] ** 2 + aLvlh[2] ** 2);
    const magB = Math.sqrt(bLvlh[0] ** 2 + bLvlh[1] ** 2 + bLvlh[2] ** 2);
    const sceneAngle = Math.acos(dotAB / (magA * magB));

    // Physical angular separation from satellite (using ECI relative vectors)
    const dA = [surfA[0] - satPos[0], surfA[1] - satPos[1], surfA[2] - satPos[2]];
    const dB = [surfB[0] - satPos[0], surfB[1] - satPos[1], surfB[2] - satPos[2]];
    const physDot = dA[0] * dB[0] + dA[1] * dB[1] + dA[2] * dB[2];
    const physMagA = Math.sqrt(dA[0] ** 2 + dA[1] ** 2 + dA[2] ** 2);
    const physMagB = Math.sqrt(dB[0] ** 2 + dB[1] ** 2 + dB[2] ** 2);
    const physAngle = Math.acos(physDot / (physMagA * physMagB));

    expect(sceneAngle).toBeCloseTo(physAngle, 10);

    // Sanity check: 1000 km feature at 400 km altitude subtends ~100°
    // (large angle because 1000 km is a significant portion of the visible surface)
    const angleDeg = sceneAngle * 180 / Math.PI;
    expect(angleDeg).toBeGreaterThan(90);
    expect(angleDeg).toBeLessThan(110);
  });
});

describe("physical accuracy: camera FOV and Earth coverage", () => {
  it("Earth limb angle from ISS altitude is ~70°", () => {
    const altitude = 400;
    const halfAngle = Math.asin(EARTH_RADIUS_KM / (EARTH_RADIUS_KM + altitude));
    const halfAngleDeg = halfAngle * 180 / Math.PI;

    // From ISS, Earth's limb is at ~70° from nadir → 20° from horizontal
    expect(halfAngleDeg).toBeCloseTo(70.2, 0);
    // Full angular diameter ~140°
    expect(halfAngleDeg * 2).toBeGreaterThan(139);
    expect(halfAngleDeg * 2).toBeLessThan(141);
  });

  it("Earth limb angle from GEO is ~8.7°", () => {
    const altitude = 35786;
    const halfAngle = Math.asin(EARTH_RADIUS_KM / (EARTH_RADIUS_KM + altitude));
    const halfAngleDeg = halfAngle * 180 / Math.PI;
    expect(halfAngleDeg).toBeCloseTo(8.7, 0);
  });

  it("horizon dip angle from ISS matches expected value", () => {
    // Horizon dip = angle below horizontal to the Earth's limb
    // = arccos(R / (R+h)) = 90° - arcsin(R / (R+h))
    const altitude = 400;
    const r = EARTH_RADIUS_KM + altitude;
    const horizonDipRad = Math.acos(EARTH_RADIUS_KM / r);
    const horizonDipDeg = horizonDipRad * 180 / Math.PI;

    // ~19.8° below horizontal — consistent with astronaut observations
    expect(horizonDipDeg).toBeCloseTo(19.8, 0);
  });
});
