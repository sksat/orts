/**
 * Tests verifying atmosphere density changes with epoch (rotation OFF).
 *
 * When Earth rotation display is OFF, the atmosphere data should still
 * change as the epoch advances because the sun direction moves in ECI
 * relative to fixed ECEF points (~15 deg/hour).
 */

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, it } from "vitest";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

let tobari: typeof import("../wasm/tobari/tobari.js");

beforeAll(async () => {
  const tobariJs = await import("../wasm/tobari/tobari.js");
  const tobariWasm = readFileSync(resolve(__dirname, "../wasm/tobari/tobari_bg.wasm"));
  tobariJs.initSync({ module: tobariWasm });
  tobari = tobariJs;
});

// J2000.0 epoch
const J2000_JD = 2451545.0;

describe("HP density changes with epoch (rotation OFF scenario)", () => {
  it("density at equator changes after 6 hours (90° sun movement)", () => {
    // Fixed geodetic point: equator, lon=0°, 400km
    const t0 = J2000_JD;
    const t1 = J2000_JD + 0.25; // 6 hours later

    const rho0 = tobari.harris_priester_density(0, 0, 400, t0);
    const rho1 = tobari.harris_priester_density(0, 0, 400, t1);

    // After 6 hours, the sun has moved ~90° relative to this ECEF point
    // HP density should be noticeably different
    const relDiff = Math.abs(rho1 - rho0) / Math.max(rho0, rho1);
    expect(relDiff).toBeGreaterThan(0.01); // At least 1% change
  });

  it("density at equator changes after 1 hour (15° sun movement)", () => {
    const t0 = J2000_JD;
    const t1 = J2000_JD + 1 / 24; // 1 hour later

    const rho0 = tobari.harris_priester_density(0, 0, 400, t0);
    const rho1 = tobari.harris_priester_density(0, 0, 400, t1);

    // 1 hour = 15° of Earth rotation
    // With n=2 (broad bulge), the change is subtle but nonzero
    expect(rho0).not.toBe(rho1);
  });

  it("max density longitude shifts ~15°/hour", () => {
    const t0 = J2000_JD;
    const t1 = J2000_JD + 1 / 24; // 1 hour later

    // Sample density at 10° longitude intervals on equator
    const lons = Array.from({ length: 36 }, (_, i) => -180 + i * 10);

    const densities0 = lons.map((lon) => tobari.harris_priester_density(0, lon, 400, t0));
    const densities1 = lons.map((lon) => tobari.harris_priester_density(0, lon, 400, t1));

    // Find longitude of max density at each epoch
    const maxIdx0 = densities0.indexOf(Math.max(...densities0));
    const maxIdx1 = densities1.indexOf(Math.max(...densities1));

    const lonShift = Math.abs(lons[maxIdx1] - lons[maxIdx0]);

    // Should shift by roughly 15° (±10° bin width + model broadness)
    // The key assertion: max density longitude DOES change
    expect(maxIdx0).not.toBe(maxIdx1);
    // Shift should be in the right ballpark (10-20° for 1 hour)
    expect(lonShift).toBeGreaterThanOrEqual(10);
    expect(lonShift).toBeLessThanOrEqual(30);
  });

  it("latlon map has different values at different epochs", () => {
    const nLat = 9;
    const nLon = 18;
    const t0 = J2000_JD;
    const t1 = J2000_JD + 0.25; // 6 hours later

    const map0 = tobari.atmosphere_latlon_map("harris-priester", 400, t0, nLat, nLon, 150, 15);
    const map1 = tobari.atmosphere_latlon_map("harris-priester", 400, t1, nLat, nLon, 150, 15);

    // Maps should be different
    let totalDiff = 0;
    for (let i = 0; i < map0.length; i++) {
      totalDiff += Math.abs(map0[i] - map1[i]);
    }
    expect(totalDiff).toBeGreaterThan(0);

    // Count how many grid points changed
    let changedCount = 0;
    for (let i = 0; i < map0.length; i++) {
      if (Math.abs(map0[i] - map1[i]) / map0[i] > 0.01) changedCount++;
    }
    // Most points should change (sun moved 90°)
    expect(changedCount).toBeGreaterThan(map0.length * 0.3);
  });
});

describe("atmosphere animation visibility", () => {
  it("HP n=2 bulge contrast: max/min density ratio at 400km", () => {
    // Measure the day/night contrast to understand why changes look subtle
    const lons = Array.from({ length: 36 }, (_, i) => -180 + i * 10);
    const densities = lons.map((lon) => tobari.harris_priester_density(0, lon, 400, J2000_JD));

    const maxRho = Math.max(...densities);
    const minRho = Math.min(...densities);
    const ratio = maxRho / minRho;

    // HP at 400km: max/min ratio is typically 2-4x
    // This is why on a log scale the change looks subtle
    expect(ratio).toBeGreaterThan(1.5);
    expect(ratio).toBeLessThan(5);
  });
});
