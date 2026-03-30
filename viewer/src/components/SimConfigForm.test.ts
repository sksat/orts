import { describe, expect, it } from "vitest";
import { buildSimConfig, PRESETS } from "./SimConfigForm.js";

describe("buildSimConfig", () => {
  it("builds config from ISS preset with NORAD orbit and attitude", () => {
    const config = buildSimConfig({
      orbitMode: "preset",
      presetIndex: 0, // ISS
      altitude: 400,
      inclination: 0,
      raan: 0,
      tleLine1: "",
      tleLine2: "",
      dt: 1,
      outputInterval: 10,
      atmosphere: "exponential",
    });

    expect(config.dt).toBe(1);
    expect(config.output_interval).toBe(10);
    expect(config.atmosphere).toBe("exponential");
    expect(config.satellites).toHaveLength(1);

    const sat = config.satellites[0];
    expect(sat.id).toBe("iss");
    expect(sat.name).toBe("ISS");
    expect(sat.orbit.type).toBe("norad");
    expect((sat.orbit as { type: "norad"; norad_id: number }).norad_id).toBe(25544);
    expect(sat.attitude).toBeDefined();
    expect(sat.attitude?.mass).toBe(420_000);
    expect(sat.attitude?.inertia_diag).toEqual([128_913_000, 107_321_000, 201_433_000]);
  });

  it("builds config from SSO preset", () => {
    const config = buildSimConfig({
      orbitMode: "preset",
      presetIndex: 1, // SSO
      altitude: 400,
      inclination: 0,
      raan: 0,
      tleLine1: "",
      tleLine2: "",
      dt: 1,
      outputInterval: 10,
      atmosphere: "exponential",
    });

    const orbit = config.satellites[0].orbit as {
      type: "circular";
      altitude: number;
      inclination: number;
    };
    expect(orbit.altitude).toBe(800);
    expect(orbit.inclination).toBe(98.6);
    expect(config.satellites[0].attitude).toBeUndefined();
  });

  it("builds config from GEO preset", () => {
    const config = buildSimConfig({
      orbitMode: "preset",
      presetIndex: 2, // GEO
      altitude: 400,
      inclination: 0,
      raan: 0,
      tleLine1: "",
      tleLine2: "",
      dt: 1,
      outputInterval: 10,
      atmosphere: "exponential",
    });

    const orbit = config.satellites[0].orbit as {
      type: "circular";
      altitude: number;
      inclination: number;
    };
    expect(orbit.altitude).toBe(35786);
    expect(orbit.inclination).toBe(0);
  });

  it("builds config from custom circular orbit", () => {
    const config = buildSimConfig({
      orbitMode: "circular",
      presetIndex: 0,
      altitude: 600,
      inclination: 45.0,
      raan: 90.0,
      tleLine1: "",
      tleLine2: "",
      dt: 5,
      outputInterval: 10,
      atmosphere: "harris-priester",
    });

    expect(config.dt).toBe(5);
    expect(config.atmosphere).toBe("harris-priester");

    const orbit = config.satellites[0].orbit as {
      type: "circular";
      altitude: number;
      inclination: number;
      raan: number;
    };
    expect(orbit.type).toBe("circular");
    expect(orbit.altitude).toBe(600);
    expect(orbit.inclination).toBe(45.0);
    expect(orbit.raan).toBe(90.0);
  });

  it("builds config from TLE input", () => {
    const line1 = "1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993";
    const line2 = "2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000";

    const config = buildSimConfig({
      orbitMode: "tle",
      presetIndex: 0,
      altitude: 400,
      inclination: 0,
      raan: 0,
      tleLine1: line1,
      tleLine2: line2,
      dt: 1,
      outputInterval: 10,
      atmosphere: "exponential",
    });

    const orbit = config.satellites[0].orbit as { type: "tle"; line1: string; line2: string };
    expect(orbit.type).toBe("tle");
    expect(orbit.line1).toBe(line1);
    expect(orbit.line2).toBe(line2);
  });

  it("uses custom dt and atmosphere", () => {
    const config = buildSimConfig({
      orbitMode: "preset",
      presetIndex: 0,
      altitude: 400,
      inclination: 0,
      raan: 0,
      tleLine1: "",
      tleLine2: "",
      dt: 1,
      outputInterval: 5,
      atmosphere: "nrlmsise00",
    });

    expect(config.dt).toBe(1);
    expect(config.output_interval).toBe(5);
    expect(config.atmosphere).toBe("nrlmsise00");
  });
});

describe("PRESETS", () => {
  it("has ISS, SSO, and GEO presets", () => {
    expect(PRESETS).toHaveLength(3);
    expect(PRESETS[0].label).toBe("ISS");
    expect(PRESETS[1].label).toBe("SSO");
    expect(PRESETS[2].label).toBe("GEO");
  });
});
