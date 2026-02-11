import { describe, it, expect } from "vitest";
import { buildDerivedQuery } from "./orbitStore.js";

const MU = 398600.4418;
const BODY_RADIUS = 6378.137;

describe("buildDerivedQuery", () => {
  it("returns basic query without maxPoints", () => {
    const sql = buildDerivedQuery(MU, BODY_RADIUS);
    expect(sql).toContain("FROM orbit_points");
    expect(sql).toContain("ORDER BY t");
    // No downsampling filter
    expect(sql).not.toContain("ROW_NUMBER");
    expect(sql).not.toContain("rn");
  });

  it("includes ROW_NUMBER downsampling with maxPoints", () => {
    const sql = buildDerivedQuery(MU, BODY_RADIUS, undefined, 2000);
    expect(sql).toContain("ROW_NUMBER");
    expect(sql).toContain("2000");
    expect(sql).toContain("ORDER BY t");
  });

  it("includes WHERE clause with tMin", () => {
    const sql = buildDerivedQuery(MU, BODY_RADIUS, 500);
    expect(sql).toContain("WHERE t >= 500");
  });

  it("includes both tMin filter and maxPoints", () => {
    const sql = buildDerivedQuery(MU, BODY_RADIUS, 500, 2000);
    expect(sql).toContain("WHERE t >= 500");
    expect(sql).toContain("ROW_NUMBER");
  });

  it("preserves first and last points via rn = 1 and rn = total", () => {
    const sql = buildDerivedQuery(MU, BODY_RADIUS, undefined, 100);
    expect(sql).toContain("rn = 1");
    expect(sql).toContain("rn = total");
  });

  it("passes all rows when maxPoints is 0", () => {
    const sql = buildDerivedQuery(MU, BODY_RADIUS, undefined, 0);
    // maxPts <= 0 should be in the WHERE clause to pass all rows
    expect(sql).not.toContain("ROW_NUMBER");
  });

  it("computes altitude, energy, angular_momentum, velocity", () => {
    const sql = buildDerivedQuery(MU, BODY_RADIUS);
    expect(sql).toContain("altitude");
    expect(sql).toContain("energy");
    expect(sql).toContain("angular_momentum");
    expect(sql).toContain("velocity");
  });

  it("uses provided mu and bodyRadius values", () => {
    const sql = buildDerivedQuery(42828.0, 3389.5); // Mars
    expect(sql).toContain("42828");
    expect(sql).toContain("3389.5");
  });
});
