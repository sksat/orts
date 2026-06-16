import { describe, expect, it } from "vitest";
import type { SatelliteInfo, SimInfo } from "./hooks/useWebSocket.js";
import { deriveSimInfo } from "./simInfoDerived.js";

const sat = (over: Partial<SatelliteInfo> & { id: string }): SatelliteInfo => ({
  name: null,
  altitude: 0,
  period: 0,
  perturbations: [],
  shape: null,
  ...over,
});

const simInfo = (over: Partial<SimInfo>): SimInfo => ({
  mu: 0,
  dt: 1,
  output_interval: 1,
  stream_interval: 1,
  central_body: "earth",
  central_body_radius: 6378.137,
  epoch_jd: null,
  satellites: [],
  ...over,
});

describe("deriveSimInfo", () => {
  it("returns app defaults when simInfo is null", () => {
    expect(deriveSimInfo(null)).toEqual({
      centralBody: "earth",
      centralBodyRadius: 6378.137,
      epochJd: undefined,
      satelliteNames: undefined,
      activePerturbations: [],
    });
  });

  it("maps each satellite id to its name (null names preserved)", () => {
    const d = deriveSimInfo(
      simInfo({
        satellites: [sat({ id: "a", name: "Alpha" }), sat({ id: "b", name: null })],
      }),
    );
    expect(d.satelliteNames).toEqual(
      new Map([
        ["a", "Alpha"],
        ["b", null],
      ]),
    );
  });

  it("unions perturbation names across satellites, de-duplicated", () => {
    const d = deriveSimInfo(
      simInfo({
        satellites: [
          sat({ id: "a", perturbations: ["drag", "srp"] }),
          sat({ id: "b", perturbations: ["srp", "j2"] }),
        ],
      }),
    );
    expect(new Set(d.activePerturbations)).toEqual(new Set(["drag", "srp", "j2"]));
    expect(d.activePerturbations).toHaveLength(3);
  });

  it("passes through central body, radius and epoch (null epoch → undefined)", () => {
    const withEpoch = deriveSimInfo(
      simInfo({ central_body: "mars", central_body_radius: 3389.5, epoch_jd: 2451545.0 }),
    );
    expect(withEpoch.centralBody).toBe("mars");
    expect(withEpoch.centralBodyRadius).toBe(3389.5);
    expect(withEpoch.epochJd).toBe(2451545.0);

    expect(deriveSimInfo(simInfo({ epoch_jd: null })).epochJd).toBeUndefined();
  });

  it("returns an empty perturbation list (not undefined) for a satellite-less sim", () => {
    expect(deriveSimInfo(simInfo({ satellites: [] })).activePerturbations).toEqual([]);
  });
});
