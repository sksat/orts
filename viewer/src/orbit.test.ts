import { describe, expect, it } from "vitest";
import { parseOrbitCSV, parseOrbitCSVWithMetadata } from "./orbit.js";

describe("parseOrbitCSV", () => {
  it("parses valid CSV lines into OrbitPoints", () => {
    const csv = "0.0,6778.137,0.0,0.0,0.0,7.669,0.0\n10.0,6777.0,76.69,0.0,-0.086,7.668,0.0";
    const points = parseOrbitCSV(csv);

    expect(points).toHaveLength(2);
    expect(points[0]).toEqual({
      t: 0.0,
      x: 6778.137,
      y: 0.0,
      z: 0.0,
      vx: 0.0,
      vy: 7.669,
      vz: 0.0,
      a: 0,
      e: 0,
      inc: 0,
      raan: 0,
      omega: 0,
      nu: 0,
      accel_gravity: 0,
      accel_drag: 0,
      accel_srp: 0,
      accel_third_body_sun: 0,
      accel_third_body_moon: 0,
    });
    expect(points[1].t).toBe(10.0);
  });

  it("skips blank lines and comment lines", () => {
    const csv = [
      "# This is a comment",
      "",
      "0.0,6778.137,0.0,0.0,0.0,7.669,0.0",
      "  ",
      "# Another comment",
      "10.0,6777.0,76.69,0.0,-0.086,7.668,0.0",
    ].join("\n");

    const points = parseOrbitCSV(csv);
    expect(points).toHaveLength(2);
  });

  it("skips lines with fewer than 7 fields", () => {
    const csv = [
      "0.0,6778.137,0.0,0.0,0.0,7.669,0.0",
      "10.0,6777.0,76.69",
      "20.0,6776.0,153.0,0.0,-0.172,7.666,0.0",
    ].join("\n");

    const points = parseOrbitCSV(csv);
    expect(points).toHaveLength(2);
    expect(points[0].t).toBe(0.0);
    expect(points[1].t).toBe(20.0);
  });

  it("skips lines with non-numeric values", () => {
    const csv = [
      "0.0,6778.137,0.0,0.0,0.0,7.669,0.0",
      "abc,6777.0,76.69,0.0,-0.086,7.668,0.0",
      "20.0,6776.0,153.0,0.0,-0.172,7.666,0.0",
    ].join("\n");

    const points = parseOrbitCSV(csv);
    expect(points).toHaveLength(2);
  });

  it("returns empty array for empty input", () => {
    expect(parseOrbitCSV("")).toHaveLength(0);
    expect(parseOrbitCSV("# only comments\n# here")).toHaveLength(0);
  });
});

describe("parseOrbitCSVWithMetadata", () => {
  it("extracts metadata from comment headers", () => {
    const csv = [
      "# Orts 2-body orbit propagation",
      "# mu = 398600.4418 km^3/s^2",
      "# epoch_jd = 2460390.0",
      "# central_body = earth",
      "# central_body_radius = 6378.137 km",
      "# t[s],x[km],y[km],z[km],vx[km/s],vy[km/s],vz[km/s]",
      "0.0,6778.137,0.0,0.0,0.0,7.669,0.0",
    ].join("\n");
    const { points, metadata } = parseOrbitCSVWithMetadata(csv);
    expect(points).toHaveLength(1);
    expect(metadata.epochJd).toBeCloseTo(2460390.0);
    expect(metadata.mu).toBeCloseTo(398600.4418);
    expect(metadata.centralBody).toBe("earth");
    expect(metadata.centralBodyRadius).toBeCloseTo(6378.137);
  });

  it("returns null metadata for CSV without metadata comments", () => {
    const csv = "0.0,6778.137,0.0,0.0,0.0,7.669,0.0";
    const { points, metadata } = parseOrbitCSVWithMetadata(csv);
    expect(points).toHaveLength(1);
    expect(metadata.epochJd).toBeNull();
    expect(metadata.mu).toBeNull();
    expect(metadata.centralBody).toBeNull();
    expect(metadata.centralBodyRadius).toBeNull();
  });

  it("handles partial metadata", () => {
    const csv = ["# epoch_jd = 2460390.0", "0.0,6778.137,0.0,0.0,0.0,7.669,0.0"].join("\n");
    const { metadata } = parseOrbitCSVWithMetadata(csv);
    expect(metadata.epochJd).toBeCloseTo(2460390.0);
    expect(metadata.mu).toBeNull();
  });
});
