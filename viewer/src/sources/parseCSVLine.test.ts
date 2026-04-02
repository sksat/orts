import { describe, expect, it } from "vitest";
import { emptyMetadata, parseDataLine, parseMetadataLine } from "./parseCSVLine.js";

describe("parseMetadataLine", () => {
  it("parses epoch_jd", () => {
    const meta = emptyMetadata();
    expect(parseMetadataLine("# epoch_jd = 2451545.0", meta)).toBe(true);
    expect(meta.epochJd).toBe(2451545.0);
  });

  it("parses mu with trailing unit text", () => {
    const meta = emptyMetadata();
    expect(parseMetadataLine("# mu = 398600.4418 km^3/s^2", meta)).toBe(true);
    expect(meta.mu).toBe(398600.4418);
  });

  it("parses central_body", () => {
    const meta = emptyMetadata();
    expect(parseMetadataLine("# central_body = moon", meta)).toBe(true);
    expect(meta.centralBody).toBe("moon");
  });

  it("parses central_body_radius with trailing unit text", () => {
    const meta = emptyMetadata();
    expect(parseMetadataLine("# central_body_radius = 1737.4 km", meta)).toBe(true);
    expect(meta.centralBodyRadius).toBe(1737.4);
  });

  it("returns false for non-metadata comment", () => {
    const meta = emptyMetadata();
    expect(parseMetadataLine("# This is a comment", meta)).toBe(false);
  });

  it("returns false for unknown key", () => {
    const meta = emptyMetadata();
    expect(parseMetadataLine("# unknown_key = 42", meta)).toBe(false);
  });
});

describe("parseDataLine", () => {
  it("parses 7-field line (minimum)", () => {
    const point = parseDataLine("0.0, 7000, 0, 0, 0, 7.5, 0");
    expect(point).not.toBeNull();
    expect(point!.t).toBe(0);
    expect(point!.x).toBe(7000);
    expect(point!.vy).toBe(7.5);
    expect(point!.a).toBe(0); // optional, defaults to 0
  });

  it("parses 13-field line with orbital elements", () => {
    const point = parseDataLine(
      "10.0, 7000, 100, 50, -0.1, 7.5, 0.01, 7100, 0.001, 97.5, 100.0, 45.0, 30.0",
    );
    expect(point).not.toBeNull();
    expect(point!.a).toBe(7100);
    expect(point!.e).toBe(0.001);
    expect(point!.inc).toBe(97.5);
  });

  it("returns null for fewer than 7 fields", () => {
    expect(parseDataLine("0.0, 7000, 0, 0, 0, 7.5")).toBeNull();
  });

  it("returns null for non-numeric fields", () => {
    expect(parseDataLine("0.0, abc, 0, 0, 0, 7.5, 0")).toBeNull();
  });

  it("returns null for empty line", () => {
    expect(parseDataLine("")).toBeNull();
  });
});

describe("emptyMetadata", () => {
  it("returns all null fields", () => {
    const meta = emptyMetadata();
    expect(meta.epochJd).toBeNull();
    expect(meta.mu).toBeNull();
    expect(meta.centralBody).toBeNull();
    expect(meta.centralBodyRadius).toBeNull();
  });
});
