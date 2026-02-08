import { describe, it, expect } from "vitest";
import { parseOrbitCSV } from "./orbit.js";

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
