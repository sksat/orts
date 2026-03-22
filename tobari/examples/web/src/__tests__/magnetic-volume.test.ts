/**
 * Tests for magnetic_field_volume WASM API.
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

const J2000_JD = 2451545.0;

describe("magnetic_field_volume layout", () => {
  it("returns correct length (n_alt * n_lat * n_lon + 2)", () => {
    const nAlt = 4;
    const nLat = 9;
    const nLon = 18;
    const result = tobari.magnetic_field_volume(
      "igrf",
      "total",
      200,
      800,
      nAlt,
      J2000_JD,
      nLat,
      nLon,
    );
    expect(result.length).toBe(nAlt * nLat * nLon + 2);
  });

  it("min/max are appended at the end", () => {
    const nAlt = 2;
    const nLat = 9;
    const nLon = 18;
    const result = tobari.magnetic_field_volume(
      "igrf",
      "total",
      200,
      800,
      nAlt,
      J2000_JD,
      nLat,
      nLon,
    );
    const total = nAlt * nLat * nLon;
    const min = result[total];
    const max = result[total + 1];
    expect(min).toBeLessThan(max);
    expect(min).toBeGreaterThan(0); // total field strength is always positive
  });

  it("all total field values are positive", () => {
    const nAlt = 3;
    const nLat = 9;
    const nLon = 18;
    const result = tobari.magnetic_field_volume(
      "igrf",
      "total",
      200,
      800,
      nAlt,
      J2000_JD,
      nLat,
      nLon,
    );
    const total = nAlt * nLat * nLon;
    for (let i = 0; i < total; i++) {
      expect(result[i]).toBeGreaterThan(0);
    }
  });
});

describe("magnetic_field_volume consistency with latlon_map", () => {
  it("nAlt=1 slice matches magnetic_field_latlon_map at same altitude", () => {
    const nLat = 9;
    const nLon = 18;
    const alt = 400;

    // Volume with single altitude
    const vol = tobari.magnetic_field_volume("igrf", "total", alt, alt, 1, J2000_JD, nLat, nLon);
    // Single map at same altitude
    const map = tobari.magnetic_field_latlon_map("igrf", "total", alt, J2000_JD, nLat, nLon);

    // Values should match (f32 vs f64 precision difference)
    for (let i = 0; i < nLat * nLon; i++) {
      expect(vol[i]).toBeCloseTo(map[i], -1); // ~0.1 nT tolerance
    }
  });
});

describe("magnetic_field_volume altitude dependence", () => {
  it("total field strength decreases with altitude (~1/r³)", () => {
    const nLat = 9;
    const nLon = 18;
    const nAlt = 4;
    const result = tobari.magnetic_field_volume(
      "igrf",
      "total",
      200,
      1000,
      nAlt,
      J2000_JD,
      nLat,
      nLon,
    );

    const sliceSize = nLat * nLon;
    // Compare mean field at lowest vs highest altitude
    let meanLow = 0;
    let meanHigh = 0;
    for (let i = 0; i < sliceSize; i++) {
      meanLow += result[i]; // altitude 0 (200km)
      meanHigh += result[(nAlt - 1) * sliceSize + i]; // altitude 3 (1000km)
    }
    meanLow /= sliceSize;
    meanHigh /= sliceSize;

    // Field at 200km should be stronger than at 1000km
    expect(meanLow).toBeGreaterThan(meanHigh);
    // Ratio should be roughly (R+1000)³/(R+200)³ ≈ 1.4
    const ratio = meanLow / meanHigh;
    expect(ratio).toBeGreaterThan(1.2);
    expect(ratio).toBeLessThan(2.0);
  });
});
