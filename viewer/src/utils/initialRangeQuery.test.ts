import { describe, expect, it } from "vitest";
import type { SatelliteInfo, SimInfo } from "../hooks/useWebSocket.js";
import { planInitialRangeQuery } from "./initialRangeQuery.js";

function makeSatellite(id: string): SatelliteInfo {
  return {
    id,
    name: null,
    altitude: 400,
    period: 5554,
    perturbations: [],
  };
}

function makeSimInfo(...satIds: string[]): SimInfo {
  const ids = satIds.length > 0 ? satIds : ["test"];
  return {
    mu: 398600.4418,
    dt: 10,
    output_interval: 10,
    stream_interval: 10,
    central_body: "earth",
    central_body_radius: 6378.137,
    epoch_jd: null,
    satellites: ids.map(makeSatellite),
  };
}

describe("planInitialRangeQuery", () => {
  it("returns empty array when simInfo is null", () => {
    expect(
      planInitialRangeQuery({
        simInfo: null,
        timeRange: 3600,
        latestT: 100,
        alreadyQueried: false,
      }),
    ).toEqual([]);
  });

  it("returns empty array when timeRange is null (All mode)", () => {
    // In "All" mode the overview itself is the full view — no proactive
    // enrichment is needed. The user can pull detail via chart zoom.
    expect(
      planInitialRangeQuery({
        simInfo: makeSimInfo(),
        timeRange: null,
        latestT: 3600,
        alreadyQueried: false,
      }),
    ).toEqual([]);
  });

  it("returns empty array when no history has arrived yet (latestT <= 0)", () => {
    expect(
      planInitialRangeQuery({
        simInfo: makeSimInfo(),
        timeRange: 3600,
        latestT: 0,
        alreadyQueried: false,
      }),
    ).toEqual([]);
  });

  it("returns empty array when already queried for this connection", () => {
    expect(
      planInitialRangeQuery({
        simInfo: makeSimInfo(),
        timeRange: 3600,
        latestT: 10_000,
        alreadyQueried: true,
      }),
    ).toEqual([]);
  });

  it("returns empty array when simInfo has no satellites", () => {
    const info = makeSimInfo();
    info.satellites = [];
    expect(
      planInitialRangeQuery({
        simInfo: info,
        timeRange: 3600,
        latestT: 10_000,
        alreadyQueried: false,
      }),
    ).toEqual([]);
  });

  it("returns one query per satellite for a single-sat sim", () => {
    const plans = planInitialRangeQuery({
      simInfo: makeSimInfo("sso"),
      timeRange: 3600,
      latestT: 86_400,
      alreadyQueried: false,
    });
    expect(plans).toHaveLength(1);
    const [plan] = plans;
    expect(plan.satId).toBe("sso");
    expect(plan.tMin).toBeCloseTo(86_400 - 3600, 6);
    expect(plan.tMax).toBeCloseTo(86_400, 6);
    expect(plan.maxPoints).toBeGreaterThanOrEqual(1000);
  });

  it("returns one query per satellite for a multi-sat sim (regression: M3)", () => {
    // Without this fix, the initial range query only enriched
    // satellites[0], leaving the other sats stuck at overview density
    // after reconnect.
    const plans = planInitialRangeQuery({
      simInfo: makeSimInfo("iss", "hubble", "starlink-1"),
      timeRange: 1800,
      latestT: 50_000,
      alreadyQueried: false,
    });
    expect(plans).toHaveLength(3);
    expect(plans.map((p) => p.satId)).toEqual(["iss", "hubble", "starlink-1"]);
    // All plans share the same window anchored at the current sim time.
    for (const plan of plans) {
      expect(plan.tMin).toBeCloseTo(50_000 - 1800, 6);
      expect(plan.tMax).toBeCloseTo(50_000, 6);
    }
  });

  it("clamps tMin to 0 when latestT is smaller than timeRange", () => {
    // Sim has only been running for 300 s but timeRange = 1h: the window
    // should clamp at 0, not produce a negative tMin.
    const plans = planInitialRangeQuery({
      simInfo: makeSimInfo(),
      timeRange: 3600,
      latestT: 300,
      alreadyQueried: false,
    });
    expect(plans).toHaveLength(1);
    expect(plans[0].tMin).toBe(0);
    expect(plans[0].tMax).toBeCloseTo(300, 6);
  });
});
