import { describe, expect, it } from "vitest";
import type { CSVMetadata } from "../orbit.js";
import { csvMetadataToSimInfo } from "./normalizeMetadata.js";

describe("csvMetadataToSimInfo", () => {
  const fullMetadata: CSVMetadata = {
    epochJd: 2451545.0,
    mu: 398600.4418,
    centralBody: "earth",
    centralBodyRadius: 6378.137,
    satelliteName: null,
    satellites: null,
  };

  it("converts full metadata to SimInfo with filename fallback", () => {
    const info = csvMetadataToSimInfo(fullMetadata, "test.csv", 10.0);
    expect(info.mu).toBe(398600.4418);
    expect(info.central_body).toBe("earth");
    expect(info.central_body_radius).toBe(6378.137);
    expect(info.epoch_jd).toBe(2451545.0);
    expect(info.satellites).toHaveLength(1);
    expect(info.satellites[0].name).toBe("test.csv");
  });

  it("uses satellite name from metadata when available", () => {
    const withName: CSVMetadata = { ...fullMetadata, satelliteName: "ISS" };
    const info = csvMetadataToSimInfo(withName, "iss_orbit.csv", 10.0);
    expect(info.satellites[0].name).toBe("ISS");
  });

  it("uses defaults for null fields", () => {
    const empty: CSVMetadata = {
      epochJd: null,
      mu: null,
      centralBody: null,
      centralBodyRadius: null,
      satelliteName: null,
      satellites: null,
    };
    const info = csvMetadataToSimInfo(empty, "orbit.csv", 5.0);
    expect(info.mu).toBe(398600.4418);
    expect(info.central_body).toBe("earth");
    expect(info.central_body_radius).toBe(6378.137);
    expect(info.epoch_jd).toBeNull();
  });

  it("sets dt from provided value", () => {
    const info = csvMetadataToSimInfo(fullMetadata, "test.csv", 7.5);
    expect(info.dt).toBe(7.5);
    expect(info.output_interval).toBe(7.5);
    expect(info.stream_interval).toBe(7.5);
  });

  it("satellite id is 'default' and name is filename", () => {
    const info = csvMetadataToSimInfo(fullMetadata, "apollo11.csv", 1.0);
    expect(info.satellites[0].id).toBe("default");
    expect(info.satellites[0].name).toBe("apollo11.csv");
    expect(info.satellites[0].perturbations).toEqual([]);
  });

  it("builds multi-satellite SimInfo from satellites metadata", () => {
    const multiSat: CSVMetadata = {
      ...fullMetadata,
      satellites: ["iss", "sso"],
    };
    const info = csvMetadataToSimInfo(multiSat, "constellation.csv", 10.0);
    expect(info.satellites).toHaveLength(2);
    expect(info.satellites[0].id).toBe("iss");
    expect(info.satellites[0].name).toBe("iss");
    expect(info.satellites[1].id).toBe("sso");
    expect(info.satellites[1].name).toBe("sso");
  });

  it("single-entry satellites list produces one satellite with id", () => {
    const singleSat: CSVMetadata = {
      ...fullMetadata,
      satellites: ["iss"],
    };
    const info = csvMetadataToSimInfo(singleSat, "iss.csv", 10.0);
    expect(info.satellites).toHaveLength(1);
    expect(info.satellites[0].id).toBe("iss");
    expect(info.satellites[0].name).toBe("iss");
  });

  it("falls back to single satellite when satellites is empty array", () => {
    const emptySats: CSVMetadata = {
      ...fullMetadata,
      satellites: [],
    };
    const info = csvMetadataToSimInfo(emptySats, "test.csv", 10.0);
    expect(info.satellites).toHaveLength(1);
    expect(info.satellites[0].id).toBe("default");
  });
});
