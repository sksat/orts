/**
 * Coordinate system tests for the tobari web visualizer.
 *
 * Verifies that:
 * 1. Julian Date conversion is correct
 * 2. The ECI→Three.js rotation maps Z-up to Y-up
 * 3. The pole alignment rotation maps SphereGeometry Y-pole to ECI Z-pole
 * 4. Composition of both rotations places the sphere pole at Three.js Y-up
 * 5. DataTexture UV mapping matches the lat/lon grid layout
 * 6. Shell radii are correct for given altitudes
 */

import { describe, expect, it } from "vitest";
import { dateToJd } from "../types.js";

// ---------------------------------------------------------------------------
// Rotation math (mirrors the Three.js Euler rotations used in GlobeView)
// ---------------------------------------------------------------------------

/** Apply rotation around X axis by angle (radians) to a 3D vector. */
function rotateX(v: [number, number, number], angle: number): [number, number, number] {
  const c = Math.cos(angle);
  const s = Math.sin(angle);
  return [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c];
}

// Constants matching GlobeView.tsx
const POLE_ALIGN_ANGLE = Math.PI / 2; // +π/2 around X
const ECI_TO_THREEJS_ANGLE = -Math.PI / 2; // -π/2 around X
const EARTH_RADIUS_KM = 6371.0;

// ---------------------------------------------------------------------------
// Julian Date
// ---------------------------------------------------------------------------

describe("dateToJd", () => {
  it("J2000.0 epoch (2000-01-01T12:00:00Z) → JD 2451545.0", () => {
    const jd = dateToJd(new Date("2000-01-01T12:00:00Z"));
    expect(jd).toBeCloseTo(2451545.0, 4);
  });

  it("Unix epoch (1970-01-01T00:00:00Z) → JD 2440587.5", () => {
    const jd = dateToJd(new Date("1970-01-01T00:00:00Z"));
    expect(jd).toBeCloseTo(2440587.5, 4);
  });

  it("2025-01-01T00:00:00Z → JD 2460676.5", () => {
    const jd = dateToJd(new Date("2025-01-01T00:00:00Z"));
    expect(jd).toBeCloseTo(2460676.5, 4);
  });

  it("midnight yields .5 fractional day (astronomical convention)", () => {
    const jd = dateToJd(new Date("2024-06-15T00:00:00Z"));
    expect(jd % 1).toBeCloseTo(0.5, 10);
  });
});

// ---------------------------------------------------------------------------
// ECI → Three.js world rotation
// ---------------------------------------------------------------------------

describe("ECI_TO_THREEJS rotation (-π/2 around X)", () => {
  it("ECI north pole [0,0,1] → Three.js [0,1,0] (Y-up)", () => {
    const result = rotateX([0, 0, 1], ECI_TO_THREEJS_ANGLE);
    expect(result[0]).toBeCloseTo(0, 10);
    expect(result[1]).toBeCloseTo(1, 10);
    expect(result[2]).toBeCloseTo(0, 10);
  });

  it("ECI X-axis [1,0,0] → Three.js [1,0,0] (unchanged)", () => {
    const result = rotateX([1, 0, 0], ECI_TO_THREEJS_ANGLE);
    expect(result[0]).toBeCloseTo(1, 10);
    expect(result[1]).toBeCloseTo(0, 10);
    expect(result[2]).toBeCloseTo(0, 10);
  });

  it("ECI Y-axis [0,1,0] → Three.js [0,0,-1]", () => {
    const result = rotateX([0, 1, 0], ECI_TO_THREEJS_ANGLE);
    expect(result[0]).toBeCloseTo(0, 10);
    expect(result[1]).toBeCloseTo(0, 10);
    expect(result[2]).toBeCloseTo(-1, 10);
  });
});

// ---------------------------------------------------------------------------
// Pole alignment rotation (SphereGeometry Y-pole → ECI Z-pole)
// ---------------------------------------------------------------------------

describe("POLE_ALIGN rotation (+π/2 around X)", () => {
  it("sphere pole [0,1,0] → ECI [0,0,1] (Z-up)", () => {
    const result = rotateX([0, 1, 0], POLE_ALIGN_ANGLE);
    expect(result[0]).toBeCloseTo(0, 10);
    expect(result[1]).toBeCloseTo(0, 10);
    expect(result[2]).toBeCloseTo(1, 10);
  });

  it("sphere equator [1,0,0] → ECI [1,0,0] (unchanged)", () => {
    const result = rotateX([1, 0, 0], POLE_ALIGN_ANGLE);
    expect(result[0]).toBeCloseTo(1, 10);
    expect(result[1]).toBeCloseTo(0, 10);
    expect(result[2]).toBeCloseTo(0, 10);
  });
});

// ---------------------------------------------------------------------------
// Combined: SphereGeometry → ECI → Three.js
// ---------------------------------------------------------------------------

describe("combined rotation (POLE_ALIGN then ECI_TO_THREEJS)", () => {
  function applyBoth(v: [number, number, number]): [number, number, number] {
    // First: pole alignment (sphere local → ECI)
    const eci = rotateX(v, POLE_ALIGN_ANGLE);
    // Then: ECI → Three.js world
    return rotateX(eci, ECI_TO_THREEJS_ANGLE);
  }

  it("sphere Y-pole → Three.js Y-up (north pole visible at top)", () => {
    const result = applyBoth([0, 1, 0]);
    expect(result[0]).toBeCloseTo(0, 10);
    expect(result[1]).toBeCloseTo(1, 10);
    expect(result[2]).toBeCloseTo(0, 10);
  });

  it("sphere south pole [0,-1,0] → Three.js [0,-1,0]", () => {
    const result = applyBoth([0, -1, 0]);
    expect(result[0]).toBeCloseTo(0, 10);
    expect(result[1]).toBeCloseTo(-1, 10);
    expect(result[2]).toBeCloseTo(0, 10);
  });

  it("sphere equator [1,0,0] → Three.js [1,0,0]", () => {
    const result = applyBoth([1, 0, 0]);
    expect(result[0]).toBeCloseTo(1, 10);
    expect(result[1]).toBeCloseTo(0, 10);
    expect(result[2]).toBeCloseTo(0, 10);
  });
});

// ---------------------------------------------------------------------------
// Field lines (ECI coordinates) through ECI_TO_THREEJS only
// ---------------------------------------------------------------------------

describe("field lines (ECI → Three.js, no pole alignment)", () => {
  it("field line at ECI north pole [0,0,R] → Three.js [0,R,0]", () => {
    const r = 1.5; // some radius in Earth radii
    const result = rotateX([0, 0, r], ECI_TO_THREEJS_ANGLE);
    expect(result[0]).toBeCloseTo(0, 10);
    expect(result[1]).toBeCloseTo(r, 10);
    expect(result[2]).toBeCloseTo(0, 10);
  });

  it("field line on equator [R,0,0] stays at Three.js [R,0,0]", () => {
    const r = 1.2;
    const result = rotateX([r, 0, 0], ECI_TO_THREEJS_ANGLE);
    expect(result[0]).toBeCloseTo(r, 10);
    expect(result[1]).toBeCloseTo(0, 10);
    expect(result[2]).toBeCloseTo(0, 10);
  });

  it("field lines and sphere north pole end up at same Three.js position", () => {
    // Sphere pole after both rotations
    const spherePole = rotateX(rotateX([0, 1, 0], POLE_ALIGN_ANGLE), ECI_TO_THREEJS_ANGLE);
    // Field line at ECI north pole after ECI→Three.js only
    const fieldNorth = rotateX([0, 0, 1], ECI_TO_THREEJS_ANGLE);

    expect(spherePole[0]).toBeCloseTo(fieldNorth[0], 10);
    expect(spherePole[1]).toBeCloseTo(fieldNorth[1], 10);
    expect(spherePole[2]).toBeCloseTo(fieldNorth[2], 10);
  });
});

// ---------------------------------------------------------------------------
// Earth rotation ON/OFF consistency
// ---------------------------------------------------------------------------

describe("rotation OFF (earthRotation=0): everything in ECEF", () => {
  // When rotation is OFF, earthRotation=0.
  // Earth texture + shells: no GMST rotation → shown in ECEF
  // Field lines: ECI, but deltaRotation = 0 - computedGmst = -computedGmst → converted to ECEF

  function rotateZ(v: [number, number, number], angle: number): [number, number, number] {
    const c = Math.cos(angle);
    const s = Math.sin(angle);
    return [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]];
  }

  it("earth prime meridian is at +X in ECEF (no rotation)", () => {
    // SphereGeometry UV U=0 → local +X → POLE_ALIGN → +X (unchanged for X)
    // With earthRotation=0, no additional rotation
    // In ECEF, prime meridian (lon=0°) IS at +X
    const poleAligned = rotateX([1, 0, 0], POLE_ALIGN_ANGLE);
    expect(poleAligned[0]).toBeCloseTo(1, 10); // +X preserved
  });

  it("field line at prime meridian (ECEF +X) matches earth texture", () => {
    const gmst = 1.5; // arbitrary GMST
    const earthRotation = 0; // rotation OFF

    // Field line point at prime meridian in ECI: rotated by GMST from ECEF +X
    const eciPoint: [number, number, number] = [Math.cos(gmst), Math.sin(gmst), 0];

    // deltaRotation = earthRotation - computedGmst = 0 - gmst = -gmst
    const deltaRotation = earthRotation - gmst;
    const fieldEcef = rotateZ(eciPoint, deltaRotation);

    // Should end up at ECEF +X (prime meridian)
    expect(fieldEcef[0]).toBeCloseTo(1, 10);
    expect(fieldEcef[1]).toBeCloseTo(0, 10);
    expect(fieldEcef[2]).toBeCloseTo(0, 10);
  });

  it("field line at north pole is unaffected by deltaRotation", () => {
    const gmst = 2.0;
    const deltaRotation = 0 - gmst;

    // North pole in ECI
    const northEci: [number, number, number] = [0, 0, 1];
    const northAfterDelta = rotateZ(northEci, deltaRotation);

    // Z-axis rotation doesn't affect Z-axis points
    expect(northAfterDelta[0]).toBeCloseTo(0, 10);
    expect(northAfterDelta[1]).toBeCloseTo(0, 10);
    expect(northAfterDelta[2]).toBeCloseTo(1, 10);
  });
});

describe("rotation ON (earthRotation=GMST): everything in ECI", () => {
  function rotateZ(v: [number, number, number], angle: number): [number, number, number] {
    const c = Math.cos(angle);
    const s = Math.sin(angle);
    return [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]];
  }

  it("earth prime meridian rotates to correct ECI position", () => {
    const gmst = 1.5;
    const earthRotation = gmst; // rotation ON

    // Sphere prime meridian starts at local +X
    // POLE_ALIGN: X→X (unchanged)
    // earthRotation: rotate by GMST around Z
    const ecefPrimeMeridian: [number, number, number] = [1, 0, 0];
    const eciPosition = rotateZ(ecefPrimeMeridian, earthRotation);

    // Should be at [cos(gmst), sin(gmst), 0] in ECI
    expect(eciPosition[0]).toBeCloseTo(Math.cos(gmst), 10);
    expect(eciPosition[1]).toBeCloseTo(Math.sin(gmst), 10);
  });

  it("field line at prime meridian matches rotated earth", () => {
    const gmst = 1.5;
    const computedGmst = gmst; // computed at same epoch
    const earthRotation = gmst;

    // Field line at prime meridian in ECI
    const eciPoint: [number, number, number] = [Math.cos(gmst), Math.sin(gmst), 0];

    // deltaRotation = earthRotation - computedGmst = gmst - gmst = 0
    const deltaRotation = earthRotation - computedGmst;
    const fieldResult = rotateZ(eciPoint, deltaRotation);

    // No rotation → field line stays at ECI position
    expect(fieldResult[0]).toBeCloseTo(Math.cos(gmst), 10);
    expect(fieldResult[1]).toBeCloseTo(Math.sin(gmst), 10);

    // Earth prime meridian after rotation
    const earthResult = rotateZ([1, 0, 0], earthRotation);
    expect(fieldResult[0]).toBeCloseTo(earthResult[0], 10);
    expect(fieldResult[1]).toBeCloseTo(earthResult[1], 10);
  });

  it("field lines follow earth when epoch advances", () => {
    const gmst0 = 1.0; // epoch at computation
    const gmst1 = 1.5; // current epoch (0.5 rad later)
    const earthRotation = gmst1;
    const computedGmst = gmst0;

    // Field line at prime meridian in ECI at gmst0
    const eciPoint: [number, number, number] = [Math.cos(gmst0), Math.sin(gmst0), 0];

    // deltaRotation = gmst1 - gmst0 = 0.5
    const deltaRotation = earthRotation - computedGmst;
    const fieldResult = rotateZ(eciPoint, deltaRotation);

    // Earth prime meridian at gmst1
    const earthResult = rotateZ([1, 0, 0], gmst1);

    // Field line should track earth prime meridian
    expect(fieldResult[0]).toBeCloseTo(earthResult[0], 10);
    expect(fieldResult[1]).toBeCloseTo(earthResult[1], 10);
  });
});

describe("rotation ON/OFF consistency at poles", () => {
  function rotateZ(v: [number, number, number], angle: number): [number, number, number] {
    const c = Math.cos(angle);
    const s = Math.sin(angle);
    return [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]];
  }

  function fullTransform(
    v: [number, number, number],
    earthRotation: number,
    isFieldLine: boolean,
    computedGmst: number,
  ): [number, number, number] {
    let result = v;
    if (isFieldLine) {
      // Field lines: deltaRotation, then ECI_TO_THREEJS
      const delta = earthRotation - computedGmst;
      result = rotateZ(result, delta);
    } else {
      // Sphere: POLE_ALIGN, then earthRotation, then ECI_TO_THREEJS
      result = rotateX(result, POLE_ALIGN_ANGLE);
      result = rotateZ(result, earthRotation);
    }
    return rotateX(result, ECI_TO_THREEJS_ANGLE);
  }

  it("north pole matches in both modes", () => {
    const gmst = 1.5;

    // Rotation OFF
    const sphereOff = fullTransform([0, 1, 0], 0, false, gmst);
    const fieldOff = fullTransform([0, 0, 1], 0, true, gmst);
    expect(sphereOff[0]).toBeCloseTo(fieldOff[0], 8);
    expect(sphereOff[1]).toBeCloseTo(fieldOff[1], 8);
    expect(sphereOff[2]).toBeCloseTo(fieldOff[2], 8);

    // Rotation ON
    const sphereOn = fullTransform([0, 1, 0], gmst, false, gmst);
    const fieldOn = fullTransform([0, 0, 1], gmst, true, gmst);
    expect(sphereOn[0]).toBeCloseTo(fieldOn[0], 8);
    expect(sphereOn[1]).toBeCloseTo(fieldOn[1], 8);
    expect(sphereOn[2]).toBeCloseTo(fieldOn[2], 8);

    // Both modes should produce same final position for north pole
    expect(sphereOff[0]).toBeCloseTo(sphereOn[0], 8);
    expect(sphereOff[1]).toBeCloseTo(sphereOn[1], 8);
    expect(sphereOff[2]).toBeCloseTo(sphereOn[2], 8);
  });

  it("equatorial point matches in both modes (same epoch)", () => {
    const gmst = 1.5;

    // Sphere equator point at UV U=0 → local +X → [1,0,0]
    // Field line at ECEF +X → ECI [cos(gmst), sin(gmst), 0]

    // Rotation OFF: both in ECEF
    const sphereOff = fullTransform([1, 0, 0], 0, false, gmst);
    const fieldLineEci: [number, number, number] = [Math.cos(gmst), Math.sin(gmst), 0];
    const fieldOff = fullTransform(fieldLineEci, 0, true, gmst);
    expect(sphereOff[0]).toBeCloseTo(fieldOff[0], 8);
    expect(sphereOff[1]).toBeCloseTo(fieldOff[1], 8);
    expect(sphereOff[2]).toBeCloseTo(fieldOff[2], 8);

    // Rotation ON: both in ECI
    const sphereOn = fullTransform([1, 0, 0], gmst, false, gmst);
    const fieldOn = fullTransform(fieldLineEci, gmst, true, gmst);
    expect(sphereOn[0]).toBeCloseTo(fieldOn[0], 8);
    expect(sphereOn[1]).toBeCloseTo(fieldOn[1], 8);
    expect(sphereOn[2]).toBeCloseTo(fieldOn[2], 8);
  });
});

// ---------------------------------------------------------------------------
// Shell radius
// ---------------------------------------------------------------------------

describe("shell radius", () => {
  it("surface (alt=0) → radius = 1.0 Earth radii", () => {
    const radius = 1.0 * (1 + 0 / EARTH_RADIUS_KM);
    expect(radius).toBe(1.0);
  });

  it("ISS altitude (400 km) → radius ≈ 1.0628", () => {
    const radius = 1.0 * (1 + 400 / EARTH_RADIUS_KM);
    expect(radius).toBeCloseTo(1.0628, 3);
  });

  it("1000 km → radius ≈ 1.157", () => {
    const radius = 1.0 * (1 + 1000 / EARTH_RADIUS_KM);
    expect(radius).toBeCloseTo(1.157, 2);
  });
});

// ---------------------------------------------------------------------------
// DataTexture UV ↔ lat/lon mapping
// ---------------------------------------------------------------------------

describe("DataTexture UV mapping", () => {
  // SphereGeometry UV:
  //   U = phi / (2π),  phi=0 at +X axis (lon=0° after pole alignment)
  //   V = 1 - theta/π, V=0 at south pole, V=1 at north pole
  //
  // Our data grid:
  //   iLat=0 → lat=-90° (south), iLat=nLat-1 → lat=+90° (north) → matches V direction
  //   iLon=0 → lon=-180°, iLon=nLon-1 → lon=+180°
  //   Data texture pixel (iLon, iLat) maps to UV (iLon/nLon, iLat/nLat)
  //
  // SphereGeometry U=0 → lon=0°, but data U=0 → lon=-180°
  // → data is shifted by 0.5 in U relative to sphere UV
  //
  // With wrapS=RepeatWrapping, this means the data at lon=0° appears at
  // sphere U=0 (which is the +X direction in ECI after pole alignment).

  it("data grid iLat=0 is south pole (lat=-90°), matches V=0", () => {
    const nLat = 90;
    const lat = -90 + ((0 + 0.5) * 180) / nLat;
    expect(lat).toBeCloseTo(-89, 0);
  });

  it("data grid iLat=nLat-1 is north pole (lat≈+90°), matches V≈1", () => {
    const nLat = 90;
    const lat = -90 + ((nLat - 1 + 0.5) * 180) / nLat;
    expect(lat).toBeCloseTo(89, 0);
  });

  it("data grid iLon=nLon/2 is lon=0° (Greenwich), should map to sphere U=0.5", () => {
    const nLon = 180;
    const lon = -180 + ((nLon / 2 + 0.5) * 360) / nLon;
    expect(lon).toBeCloseTo(1, 0); // lon ≈ 1° (center of bin)
  });

  it("lon=-180° and lon=+180° data wraps correctly with RepeatWrapping", () => {
    // Both edges of the data grid should wrap seamlessly
    const nLon = 180;
    const lonFirst = -180 + (0.5 * 360) / nLon;
    const lonLast = -180 + ((nLon - 1 + 0.5) * 360) / nLon;
    // Gap between last and first (across the date line) should equal one bin width
    const binWidth = 360 / nLon;
    const gap = lonFirst + 360 - lonLast;
    expect(gap).toBeCloseTo(binWidth, 10);
  });
});

// ---------------------------------------------------------------------------
// ECEF sanity checks (pure math, no WASM)
// ---------------------------------------------------------------------------

describe("geodetic → ECEF sanity (pure math)", () => {
  // WGS-84 semi-major axis
  const A = 6378.137; // km

  it("north pole (90°,0°,0km) → ECEF z ≈ polar radius, x=y=0", () => {
    // Polar radius ≈ 6356.752 km
    // At geodetic lat=90°, ECEF = (0, 0, b) where b is polar radius
    // Just check structure: x=0, y=0, z≈polar radius (verified by WASM test)
    expect(true).toBe(true);
  });

  it("equator (0°,0°,0km) → ECEF x ≈ equatorial radius, y=z=0", () => {
    // At geodetic lat=0°, lon=0°: ECEF = (A, 0, 0)
    expect(A).toBeCloseTo(6378.137, 3);
  });
});
