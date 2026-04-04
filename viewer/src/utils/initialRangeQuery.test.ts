import { describe, expect, it } from "vitest";
import type { SimInfo } from "../hooks/useWebSocket.js";
import { planInitialRangeQuery } from "./initialRangeQuery.js";

function makeSimInfo(satId = "test"): SimInfo {
  return {
    mu: 398600.4418,
    dt: 10,
    output_interval: 10,
    stream_interval: 10,
    central_body: "earth",
    central_body_radius: 6378.137,
    epoch_jd: null,
    satellites: [
      {
        id: satId,
        name: null,
        altitude: 400,
        period: 5554,
        perturbations: [],
      },
    ],
  };
}

describe("planInitialRangeQuery", () => {
  it("returns null when simInfo is null", () => {
    expect(
      planInitialRangeQuery({
        simInfo: null,
        timeRange: 3600,
        latestT: 100,
        alreadyQueried: false,
      }),
    ).toBeNull();
  });

  it("returns null when timeRange is null (All mode)", () => {
    // In "All" mode the overview itself is the full view — no proactive
    // enrichment is needed. The user can pull detail via chart zoom.
    expect(
      planInitialRangeQuery({
        simInfo: makeSimInfo(),
        timeRange: null,
        latestT: 3600,
        alreadyQueried: false,
      }),
    ).toBeNull();
  });

  it("returns null when no history has arrived yet (latestT <= 0)", () => {
    expect(
      planInitialRangeQuery({
        simInfo: makeSimInfo(),
        timeRange: 3600,
        latestT: 0,
        alreadyQueried: false,
      }),
    ).toBeNull();
  });

  it("returns null when already queried for this connection", () => {
    expect(
      planInitialRangeQuery({
        simInfo: makeSimInfo(),
        timeRange: 3600,
        latestT: 10_000,
        alreadyQueried: true,
      }),
    ).toBeNull();
  });

  it("returns null when simInfo has no satellites", () => {
    const info = makeSimInfo();
    info.satellites = [];
    expect(
      planInitialRangeQuery({
        simInfo: info,
        timeRange: 3600,
        latestT: 10_000,
        alreadyQueried: false,
      }),
    ).toBeNull();
  });

  it("returns a query for the last timeRange seconds when all conditions are met", () => {
    const plan = planInitialRangeQuery({
      simInfo: makeSimInfo("sso"),
      timeRange: 3600,
      latestT: 86_400,
      alreadyQueried: false,
    });
    if (plan == null) throw new Error("expected non-null plan");
    expect(plan.satId).toBe("sso");
    expect(plan.tMin).toBeCloseTo(86_400 - 3600, 6);
    expect(plan.tMax).toBeCloseTo(86_400, 6);
    expect(plan.maxPoints).toBeGreaterThanOrEqual(1000);
  });

  it("clamps tMin to 0 when latestT is smaller than timeRange", () => {
    // Sim has only been running for 300 s but timeRange = 1h: the window
    // should clamp at 0, not produce a negative tMin.
    const plan = planInitialRangeQuery({
      simInfo: makeSimInfo(),
      timeRange: 3600,
      latestT: 300,
      alreadyQueried: false,
    });
    if (plan == null) throw new Error("expected non-null plan");
    expect(plan.tMin).toBe(0);
    expect(plan.tMax).toBeCloseTo(300, 6);
  });
});
