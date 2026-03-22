/**
 * WASM integration tests for space weather loading and lookup.
 *
 * Verifies that:
 * - CSSI format data can be loaded via load_space_weather
 * - GFZ format data can be loaded
 * - space_weather_lookup returns valid F10.7/Ap values
 * - atmosphere_latlon_map_sw uses real space weather
 */

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, it } from "vitest";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

// Note: load_space_weather uses OnceLock, so only one test file can load data.
// We test with CSSI format (the primary use case).

let tobari: typeof import("../wasm/tobari/tobari.js");

beforeAll(async () => {
  const tobariJs = await import("../wasm/tobari/tobari.js");
  const tobariWasm = readFileSync(resolve(__dirname, "../wasm/tobari/tobari_bg.wasm"));
  tobariJs.initSync({ module: tobariWasm });
  tobari = tobariJs;
});

describe("load_space_weather", () => {
  it("loads CSSI format data successfully", () => {
    const cssiText = readFileSync(
      resolve(__dirname, "../../../../tests/fixtures/cssi_test_weather.txt"),
      "utf-8",
    );
    const result = tobari.load_space_weather(cssiText);
    expect(result).toBe(true);
  });

  it("returns false on second call (OnceLock)", () => {
    const result = tobari.load_space_weather("garbage data");
    expect(result).toBe(false);
  });
});

describe("space_weather_lookup", () => {
  it("returns 10 values for a valid epoch", () => {
    // 2019-07-01 is in the CSSI test fixture
    const jd = 2458665.5; // 2019-07-01 00:00 UTC
    const sw = tobari.space_weather_lookup(jd);
    expect(sw.length).toBe(10);
  });

  it("returns valid F10.7 (positive, physically reasonable)", () => {
    const jd = 2458665.5; // 2019-07-01
    const sw = tobari.space_weather_lookup(jd);
    const f107_daily = sw[0];
    const f107_avg = sw[1];
    // Solar minimum: F10.7 should be ~60-80 SFU
    expect(f107_daily).toBeGreaterThan(50);
    expect(f107_daily).toBeLessThan(200);
    expect(f107_avg).toBeGreaterThan(50);
    expect(f107_avg).toBeLessThan(200);
  });

  it("returns valid Ap (non-negative)", () => {
    const jd = 2458665.5;
    const sw = tobari.space_weather_lookup(jd);
    const ap_daily = sw[2];
    expect(ap_daily).toBeGreaterThanOrEqual(0);
    expect(ap_daily).toBeLessThan(400); // Ap range: 0-400
  });

  it("returns 7-element ap history", () => {
    const jd = 2458665.5;
    const sw = tobari.space_weather_lookup(jd);
    // indices 3..9 are ap_3hour_history
    for (let i = 3; i < 10; i++) {
      expect(sw[i]).toBeGreaterThanOrEqual(0);
    }
  });
});

describe("space_weather_date_range", () => {
  it("returns valid date range", () => {
    const range = tobari.space_weather_date_range();
    expect(range.length).toBe(2);
    const [jdFirst, jdLast] = [range[0], range[1]];
    expect(jdFirst).toBeLessThan(jdLast);
    // CSSI test fixture starts at 2019-06-28
    expect(jdFirst).toBeCloseTo(2458662.5, 0); // 2019-06-28
  });
});

describe("atmosphere_latlon_map_sw", () => {
  it("returns density grid using real space weather", () => {
    const nLat = 18;
    const nLon = 36;
    const jd = 2458665.5; // 2019-07-01
    const data = tobari.atmosphere_latlon_map_sw("nrlmsise00", 400, jd, nLat, nLon);
    expect(data.length).toBe(nLat * nLon);

    // All values should be positive (valid density)
    for (let i = 0; i < data.length; i++) {
      expect(data[i]).toBeGreaterThan(0);
    }
  });

  it("produces different results from constant weather", () => {
    const nLat = 9;
    const nLon = 18;
    const jd = 2458665.5;

    const swData = tobari.atmosphere_latlon_map_sw("nrlmsise00", 400, jd, nLat, nLon);
    // Compare with constant weather at a different F10.7
    const constData = tobari.atmosphere_latlon_map("nrlmsise00", 400, jd, nLat, nLon, 250, 50);

    // Should be different (real weather ~70 SFU vs constant 250 SFU)
    let totalDiff = 0;
    for (let i = 0; i < swData.length; i++) {
      totalDiff += Math.abs(swData[i] - constData[i]);
    }
    expect(totalDiff).toBeGreaterThan(0);
  });
});
