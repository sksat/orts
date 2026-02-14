import { describe, it, expect } from "vitest";
import { jdToUTCString, sunDirectionECI, earthRotationAngle } from "./astro.js";

describe("jdToUTCString", () => {
  it("converts J2000.0 epoch to 2000-01-01T12:00:00Z", () => {
    const j2000Jd = 2451545.0;
    expect(jdToUTCString(j2000Jd, 0)).toBe("2000-01-01T12:00:00Z");
  });

  it("converts 2024-03-20 12:00 UTC correctly", () => {
    const jd = 2460390.0; // 2024-03-20 12:00:00 UTC
    expect(jdToUTCString(jd, 0)).toBe("2024-03-20T12:00:00Z");
  });

  it("adds sim time offset", () => {
    const j2000Jd = 2451545.0;
    // +3600 seconds = +1 hour → 13:00:00
    expect(jdToUTCString(j2000Jd, 3600)).toBe("2000-01-01T13:00:00Z");
  });

  it("adds one day of sim time", () => {
    const j2000Jd = 2451545.0;
    // +86400 seconds = +1 day → 2000-01-02
    expect(jdToUTCString(j2000Jd, 86400)).toBe("2000-01-02T12:00:00Z");
  });

  it("handles fractional seconds by truncating", () => {
    const j2000Jd = 2451545.0;
    // Result should not include milliseconds
    const result = jdToUTCString(j2000Jd, 0.5);
    expect(result).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/);
  });
});

describe("sunDirectionECI", () => {
  it("returns unit vector", () => {
    const [x, y, z] = sunDirectionECI(2460390.0, 0);
    const norm = Math.sqrt(x * x + y * y + z * z);
    expect(norm).toBeCloseTo(1.0, 10);
  });

  it("points near +X at March equinox", () => {
    // 2024-03-20 ~03:06 UTC
    const jd = 2460389.629;
    const [x, y, z] = sunDirectionECI(jd, 0);
    expect(x).toBeGreaterThan(0.9);
    expect(Math.abs(y)).toBeLessThan(0.2);
    expect(Math.abs(z)).toBeLessThan(0.1);
  });

  it("points near -X at September equinox", () => {
    // 2024-09-22 ~12:44 UTC
    const jd = 2460576.031;
    const [x, y, z] = sunDirectionECI(jd, 0);
    expect(x).toBeLessThan(-0.9);
    expect(Math.abs(y)).toBeLessThan(0.2);
    expect(Math.abs(z)).toBeLessThan(0.1);
  });
});

describe("earthRotationAngle", () => {
  const TAU = 2 * Math.PI;
  const J2000_JD = 2451545.0;

  it("returns ~4.895 rad at J2000", () => {
    // At J2000.0, ERA = 2π × 0.7790572732640 ≈ 4.8949 rad
    // Matches Rust test: coords/src/epoch.rs gmst_at_j2000
    const era = earthRotationAngle(J2000_JD, 0);
    const expected = TAU * 0.7790572732640;
    expect(Math.abs(era - expected)).toBeLessThan(0.01);
  });

  it("increases by ~2π × 1.00274 per solar day", () => {
    // After one solar day (86400s), ERA should increase by ~360.986°
    // Matches Rust test: coords/src/epoch.rs gmst_increases_one_sidereal_day
    const era0 = earthRotationAngle(J2000_JD, 0);
    const era1 = earthRotationAngle(J2000_JD, 86400);
    const delta = era1 > era0 ? era1 - era0 : era1 + TAU - era0;
    const expectedDelta = (TAU * 1.00273781191135448) % TAU;
    expect(Math.abs(delta - expectedDelta)).toBeLessThan(0.001);
  });

  it("is normalized to [0, 2π)", () => {
    // ERA should always be in [0, 2π) for any time
    // Matches Rust test: coords/src/epoch.rs gmst_normalized
    for (const days of [0, 0.5, 1, 100, 365.25, 3652.5]) {
      const era = earthRotationAngle(J2000_JD, days * 86400);
      expect(era).toBeGreaterThanOrEqual(0);
      expect(era).toBeLessThan(TAU);
    }
  });

  it("advances with sim time offset", () => {
    // Using epochJd with simTimeSec should match epochJd+offset with 0
    const era1 = earthRotationAngle(J2000_JD, 3600);
    const era2 = earthRotationAngle(J2000_JD + 3600 / 86400, 0);
    expect(Math.abs(era1 - era2)).toBeLessThan(1e-10);
  });
});
