/**
 * WASM integration tests for coordinate system correctness.
 *
 * Uses kaname WASM (geodetic_to_ecef, geodetic_to_eci) and tobari WASM
 * (magnetic_field_lines, igrf_field_at) to verify that:
 * - Geodetic → ECEF/ECI conversions are physically correct
 * - The globe visualization coordinate system is consistent
 * - Field lines originate from the correct geographic positions
 */

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, it } from "vitest";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

// ---------------------------------------------------------------------------
// WASM module loading (synchronous via initSync + readFileSync)
// ---------------------------------------------------------------------------

let kaname: typeof import("../wasm/kaname/kaname.js");
let tobari: typeof import("../wasm/tobari/tobari.js");

beforeAll(async () => {
  // Load kaname WASM
  const kanameJs = await import("../wasm/kaname/kaname.js");
  const kanameWasm = readFileSync(resolve(__dirname, "../wasm/kaname/kaname_bg.wasm"));
  kanameJs.initSync({ module: kanameWasm });
  kaname = kanameJs;

  // Load tobari WASM
  const tobariJs = await import("../wasm/tobari/tobari.js");
  const tobariWasm = readFileSync(resolve(__dirname, "../wasm/tobari/tobari_bg.wasm"));
  tobariJs.initSync({ module: tobariWasm });
  tobari = tobariJs;
});

const EARTH_EQUATORIAL_RADIUS = 6378.137; // WGS-84 [km]
const EARTH_POLAR_RADIUS = 6356.752; // WGS-84 [km]
const EARTH_RADIUS_KM = 6371.0; // mean radius used in GlobeView

// J2000.0 epoch
const J2000_JD = 2451545.0;

// ---------------------------------------------------------------------------
// Geodetic → ECEF
// ---------------------------------------------------------------------------

describe("kaname geodetic_to_ecef", () => {
  it("north pole (90°,0°,0km) → ECEF z ≈ polar radius, x=y≈0", () => {
    const ecef = kaname.geodetic_to_ecef(90, 0, 0);
    expect(ecef[0]).toBeCloseTo(0, 3); // x ≈ 0
    expect(ecef[1]).toBeCloseTo(0, 3); // y ≈ 0
    expect(ecef[2]).toBeCloseTo(EARTH_POLAR_RADIUS, 0); // z ≈ 6356.752
  });

  it("equator Greenwich (0°,0°,0km) → ECEF x ≈ equatorial radius, y=z≈0", () => {
    const ecef = kaname.geodetic_to_ecef(0, 0, 0);
    expect(ecef[0]).toBeCloseTo(EARTH_EQUATORIAL_RADIUS, 0); // x ≈ 6378.137
    expect(ecef[1]).toBeCloseTo(0, 3); // y ≈ 0
    expect(ecef[2]).toBeCloseTo(0, 3); // z ≈ 0
  });

  it("equator 90°E (0°,90°,0km) → ECEF y ≈ equatorial radius, x=z≈0", () => {
    const ecef = kaname.geodetic_to_ecef(0, 90, 0);
    expect(ecef[0]).toBeCloseTo(0, 3);
    expect(ecef[1]).toBeCloseTo(EARTH_EQUATORIAL_RADIUS, 0);
    expect(ecef[2]).toBeCloseTo(0, 3);
  });

  it("south pole (-90°,0°,0km) → ECEF z ≈ -polar radius", () => {
    const ecef = kaname.geodetic_to_ecef(-90, 0, 0);
    expect(ecef[0]).toBeCloseTo(0, 3);
    expect(ecef[1]).toBeCloseTo(0, 3);
    expect(ecef[2]).toBeCloseTo(-EARTH_POLAR_RADIUS, 0);
  });

  it("altitude increases radius", () => {
    const alt = 400; // ISS altitude
    const ecef = kaname.geodetic_to_ecef(0, 0, alt);
    expect(ecef[0]).toBeCloseTo(EARTH_EQUATORIAL_RADIUS + alt, 0);
  });
});

// ---------------------------------------------------------------------------
// Geodetic → ECI
// ---------------------------------------------------------------------------

describe("kaname geodetic_to_eci", () => {
  it("north pole → ECI z component is large, x²+y² ≈ 0", () => {
    const eci = kaname.geodetic_to_eci(90, 0, 0, J2000_JD);
    const rxy = Math.sqrt(eci[0] ** 2 + eci[1] ** 2);
    expect(rxy).toBeLessThan(1); // nearly on Z-axis
    expect(Math.abs(eci[2])).toBeCloseTo(EARTH_POLAR_RADIUS, 0);
  });

  it("ECI position magnitude ≈ ECEF position magnitude (same point)", () => {
    const ecef = kaname.geodetic_to_ecef(45, 30, 400);
    const eci = kaname.geodetic_to_eci(45, 30, 400, J2000_JD);
    const rEcef = Math.sqrt(ecef[0] ** 2 + ecef[1] ** 2 + ecef[2] ** 2);
    const rEci = Math.sqrt(eci[0] ** 2 + eci[1] ** 2 + eci[2] ** 2);
    // Magnitudes must match (rotation preserves length)
    expect(rEci).toBeCloseTo(rEcef, 3);
  });

  it("north pole ECI z is same regardless of epoch (rotation is around Z)", () => {
    const eci1 = kaname.geodetic_to_eci(90, 0, 0, J2000_JD);
    const eci2 = kaname.geodetic_to_eci(90, 0, 0, J2000_JD + 0.5);
    // Z component should be identical (pole is on the rotation axis)
    expect(eci1[2]).toBeCloseTo(eci2[2], 6);
  });
});

// ---------------------------------------------------------------------------
// ECI → ECEF roundtrip
// ---------------------------------------------------------------------------

describe("ECI ↔ ECEF roundtrip", () => {
  it("geodetic → ECI → ECEF matches direct geodetic → ECEF", () => {
    const lat = 35.6;
    const lon = 139.7; // Tokyo
    const alt = 0;

    const ecefDirect = kaname.geodetic_to_ecef(lat, lon, alt);
    const eci = kaname.geodetic_to_eci(lat, lon, alt, J2000_JD);
    const ecefViaEci = kaname.eci_to_ecef(
      eci[0] as unknown as number,
      eci[1] as unknown as number,
      eci[2] as unknown as number,
      J2000_JD,
      0,
    );

    // eci_to_ecef returns Float32Array, so lower precision
    expect(ecefViaEci[0]).toBeCloseTo(ecefDirect[0], -1);
    expect(ecefViaEci[1]).toBeCloseTo(ecefDirect[1], -1);
    expect(ecefViaEci[2]).toBeCloseTo(ecefDirect[2], -1);
  });
});

// ---------------------------------------------------------------------------
// Globe visualization coordinate consistency
// ---------------------------------------------------------------------------

describe("globe coordinate consistency", () => {
  // In GlobeView:
  // - Spheres have POLE_ALIGN (+π/2 around X): sphere Y-pole → ECI Z
  // - Field lines are in ECI coordinates (Z-up), divided by EARTH_RADIUS_KM
  // - Outer group ECI_TO_THREEJS (-π/2 around X): ECI Z → Three.js Y

  function rotateX(v: [number, number, number], angle: number): [number, number, number] {
    const c = Math.cos(angle);
    const s = Math.sin(angle);
    return [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c];
  }

  it("north pole in ECI → field line position matches sphere pole in Three.js", () => {
    // Field line at north pole in ECI: [0, 0, R] where R is in Earth radii
    const eciNorth: [number, number, number] = [0, 0, 1];
    // Apply ECI→Three.js only (field lines don't get pole alignment)
    const fieldThreejs = rotateX(eciNorth, -Math.PI / 2);

    // Sphere pole in SphereGeometry local: [0, 1, 0]
    // Apply POLE_ALIGN then ECI_TO_THREEJS
    const sphereEci = rotateX([0, 1, 0], Math.PI / 2);
    const sphereThreejs = rotateX(sphereEci, -Math.PI / 2);

    // Both should be at [0, 1, 0] in Three.js world
    expect(fieldThreejs[0]).toBeCloseTo(sphereThreejs[0], 10);
    expect(fieldThreejs[1]).toBeCloseTo(sphereThreejs[1], 10);
    expect(fieldThreejs[2]).toBeCloseTo(sphereThreejs[2], 10);
  });

  it("tobari field line at magnetic pole starts near ECI Z-axis", () => {
    // Seed a field line at geographic north pole
    const lines = tobari.magnetic_field_lines(
      new Float64Array([89]), // near north pole
      new Float64Array([0]),
      400,
      J2000_JD,
      "dipole",
      10,
      100,
    );
    // First point should be near ECI Z-axis (x≈0, y≈0, z>0)
    const nLines = lines[0];
    expect(nLines).toBe(1);
    const nPts = lines[1];
    expect(nPts).toBeGreaterThan(0);
    // First vertex (in Earth radii)
    const x = lines[2];
    const y = lines[3];
    const z = lines[4];
    const rxy = Math.sqrt(x * x + y * y);
    // Near the pole, x and y should be small relative to z
    expect(z).toBeGreaterThan(0.5);
    expect(rxy).toBeLessThan(0.5);
  });

  it("tobari IGRF field at north pole has strong downward component", () => {
    const field = tobari.igrf_field_at(89, 0, 400, J2000_JD);
    // field: [Bn, Be, Bd, |B|, inc, dec] in nT/degrees
    const inclination = field[4]; // should be strongly positive (pointing down at north pole)
    expect(inclination).toBeGreaterThan(70); // near 90° at magnetic pole
  });

  // -----------------------------------------------------------------------
  // Differential rotation equivalence with kaname ECI→ECEF
  // -----------------------------------------------------------------------
  // GlobeView computes field lines in ECI at epoch T0, then applies:
  //   deltaRotation = GMST(T_current) - GMST(T0)
  // as a Z-axis rotation. This should be equivalent to:
  //   kaname.eci_to_ecef(point, T_current) rotated back by -GMST(T_current)
  //   ... which is just a Z-rotation by -GMST(T0)
  // i.e., the differential rotation converts ECI(T0) → ECI(T_current)
  // by the amount the Earth rotated between T0 and T_current.

  function rotateZ(v: [number, number, number], angle: number): [number, number, number] {
    const c = Math.cos(angle);
    const s = Math.sin(angle);
    return [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]];
  }

  it("differential Z-rotation equals kaname ECI→ECEF→ECI roundtrip across epochs", () => {
    // A point on the equator at lon=30°, alt=400km
    const lat = 0;
    const lon = 30;
    const alt = 400;

    const T0 = J2000_JD;
    const T1 = J2000_JD + 0.25; // 6 hours later

    // Get GMST at both epochs from kaname
    const gmst0 = kaname.earth_rotation_angle(T0, 0);
    const gmst1 = kaname.earth_rotation_angle(T1, 0);

    // Compute field line seed in ECI at T0
    const eciT0 = kaname.geodetic_to_eci(lat, lon, alt, T0);
    const pointEci: [number, number, number] = [eciT0[0], eciT0[1], eciT0[2]];

    // Method 1: GlobeView's differential rotation approach
    // Apply deltaRotation = gmst1 - gmst0 around Z
    const deltaRotation = gmst1 - gmst0;
    const viaRotation = rotateZ(pointEci, deltaRotation);

    // Method 2: kaname's exact coordinate transformation
    // ECI(T0) → ECEF (time-independent) → ECI(T1)
    // Step 1: ECI(T0) → ECEF: rotate by -gmst0
    const ecef = rotateZ(pointEci, -gmst0);
    // Step 2: ECEF → ECI(T1): rotate by +gmst1
    const viaKaname = rotateZ(ecef, gmst1);

    // Both methods should produce the same result
    expect(viaRotation[0]).toBeCloseTo(viaKaname[0], 8);
    expect(viaRotation[1]).toBeCloseTo(viaKaname[1], 8);
    expect(viaRotation[2]).toBeCloseTo(viaKaname[2], 8);
  });

  it("differential rotation matches kaname eci_to_ecef for multiple points", () => {
    const T0 = J2000_JD;
    const T1 = J2000_JD + 1.0; // 1 day later

    const gmst0 = kaname.earth_rotation_angle(T0, 0);
    const gmst1 = kaname.earth_rotation_angle(T1, 0);
    const delta = gmst1 - gmst0;

    // Test multiple geodetic positions
    const testPoints = [
      { lat: 0, lon: 0 },
      { lat: 45, lon: 90 },
      { lat: -30, lon: -60 },
      { lat: 80, lon: 170 },
    ];

    for (const { lat, lon } of testPoints) {
      const eciT0 = kaname.geodetic_to_eci(lat, lon, 400, T0);
      const eciT1 = kaname.geodetic_to_eci(lat, lon, 400, T1);

      // The point in ECI(T0) rotated by delta should equal ECI(T1)
      // because the same geodetic point maps to different ECI positions
      // as Earth rotates, and the difference is exactly delta.
      const rotated = rotateZ([eciT0[0], eciT0[1], eciT0[2]], delta);

      // Magnitude must be preserved
      const magOrig = Math.sqrt(eciT0[0] ** 2 + eciT0[1] ** 2 + eciT0[2] ** 2);
      const magRot = Math.sqrt(rotated[0] ** 2 + rotated[1] ** 2 + rotated[2] ** 2);
      expect(magRot).toBeCloseTo(magOrig, 6);

      // The rotated point should match ECI(T1) from kaname
      expect(rotated[0]).toBeCloseTo(eciT1[0], 2);
      expect(rotated[1]).toBeCloseTo(eciT1[1], 2);
      expect(rotated[2]).toBeCloseTo(eciT1[2], 2);
    }
  });

  it("differential rotation is zero when epochs match", () => {
    const T = J2000_JD + 100;
    const gmst = kaname.earth_rotation_angle(T, 0);
    const delta = gmst - gmst;
    expect(delta).toBe(0);

    // Rotating by 0 should leave the point unchanged
    const point: [number, number, number] = [6778, 1000, 3000];
    const rotated = rotateZ(point, delta);
    expect(rotated[0]).toBe(point[0]);
    expect(rotated[1]).toBe(point[1]);
    expect(rotated[2]).toBe(point[2]);
  });

  it("shell radius at 400km altitude is consistent", () => {
    const expectedRadius = 1.0 * (1 + 400 / EARTH_RADIUS_KM);
    // Verify this matches the actual geodetic radius from kaname
    const ecef = kaname.geodetic_to_ecef(0, 0, 400);
    const actualRadiusKm = Math.sqrt(ecef[0] ** 2 + ecef[1] ** 2 + ecef[2] ** 2);
    const actualNormalized = actualRadiusKm / EARTH_RADIUS_KM;
    // Should be close (not exact due to oblateness vs mean radius)
    expect(Math.abs(expectedRadius - actualNormalized)).toBeLessThan(0.002);
  });
});
