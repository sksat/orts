import { describe, expect, it } from "vitest";
import {
  buildSimConfig,
  defaultStepsFor,
  PRESETS,
  pacingChoices,
  pacingOfChoice,
  resolveSteps,
} from "./SimConfigForm.js";

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

describe("speed choices", () => {
  it("names the server's default once the server has said it", () => {
    expect(pacingChoices(null)[0]).toEqual({ value: "", label: "Server default" });
    expect(pacingChoices("accelerated")[0].label).toBe("Server default (accelerated)");
    expect(pacingChoices("realtime")[0].label).toBe("Server default (realtime)");
  });

  it("offers both pacings besides the default", () => {
    expect(pacingChoices(null).map((c) => c.value)).toEqual(["", "realtime", "accelerated"]);
  });

  it("asks for a pacing only when one is chosen", () => {
    // Leaving the key out is what lets `orts serve --realtime` decide.
    expect(pacingOfChoice("")).toBeUndefined();
    expect(pacingOfChoice("realtime")).toBe("realtime");
    expect(pacingOfChoice("accelerated")).toBe("accelerated");
  });
});

describe("dt and output interval follow the speed", () => {
  it("defaults realtime to 0.1 s steps and accelerated to 1 s / 10 s", () => {
    // Realtime sends a state per output interval of wall time, so the
    // accelerated default (one every 10 s) would leave the view still.
    expect(defaultStepsFor("realtime")).toEqual({ dt: 0.1, outputInterval: 0.1 });
    expect(defaultStepsFor("accelerated")).toEqual({ dt: 1, outputInterval: 10 });
  });

  it("takes the defaults of the pacing the start will run at", () => {
    const none = { dt: null, outputInterval: null };
    expect(resolveSteps(none, "realtime", null)).toEqual({ dt: 0.1, outputInterval: 0.1 });
    // "Server default" follows what the server said its default is…
    expect(resolveSteps(none, "", "realtime")).toEqual({ dt: 0.1, outputInterval: 0.1 });
    expect(resolveSteps(none, "", "accelerated")).toEqual({ dt: 1, outputInterval: 10 });
    // …and, before the server has said, the accelerated one it has always had.
    expect(resolveSteps(none, "", null)).toEqual({ dt: 1, outputInterval: 10 });
  });

  it("keeps a value the user typed whichever speed is chosen", () => {
    const typedDt = { dt: 5, outputInterval: null };
    expect(resolveSteps(typedDt, "accelerated", null)).toEqual({ dt: 5, outputInterval: 10 });
    const typedBoth = { dt: 2, outputInterval: 4 };
    expect(resolveSteps(typedBoth, "realtime", null)).toEqual({ dt: 2, outputInterval: 4 });
  });

  it("fits the default it fills in around the value the user typed", () => {
    // The server refuses an output interval below dt. A typed dt of 5 s with
    // realtime's 0.1 s output, or a typed 0.5 s output with accelerated's
    // 1 s dt, would make the start fail on a field the user never touched.
    const typedDt = { dt: 5, outputInterval: null };
    expect(resolveSteps(typedDt, "realtime", null)).toEqual({ dt: 5, outputInterval: 5 });
    const typedOutput = { dt: null, outputInterval: 0.5 };
    expect(resolveSteps(typedOutput, "accelerated", null)).toEqual({
      dt: 0.5,
      outputInterval: 0.5,
    });
    expect(resolveSteps(typedOutput, "realtime", null)).toEqual({ dt: 0.1, outputInterval: 0.5 });
  });
});
