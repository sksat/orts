/**
 * Convert CSV metadata into a SimInfo-compatible object.
 *
 * CSV files don't carry the full SimInfo structure that a WebSocket server
 * provides, so we fill in sensible defaults for missing fields.
 */

import type { CSVMetadata } from "../orbit.js";
import type { RrdMetadata } from "../wasm/rrdWasmInit.js";
import type { SimInfo } from "./types.js";

/** WGS84 Earth gravitational parameter [km³/s²]. */
const DEFAULT_MU = 398600.4418;

/** WGS84 Earth equatorial radius [km]. */
const DEFAULT_RADIUS = 6378.137;

/**
 * Build a SimInfo from CSV metadata.
 *
 * @param metadata - Parsed CSV comment headers
 * @param fileName - Original file name (used as satellite display name)
 * @param dt - Estimated time step between data points [s]
 */
export function csvMetadataToSimInfo(metadata: CSVMetadata, fileName: string, dt: number): SimInfo {
  return {
    mu: metadata.mu ?? DEFAULT_MU,
    dt,
    output_interval: dt,
    stream_interval: dt,
    central_body: metadata.centralBody ?? "earth",
    central_body_radius: metadata.centralBodyRadius ?? DEFAULT_RADIUS,
    epoch_jd: metadata.epochJd,
    satellites: [
      {
        id: "default",
        name: metadata.satelliteName ?? fileName,
        altitude: 0,
        period: 0,
        perturbations: [],
      },
    ],
  };
}

/**
 * Build a SimInfo from RRD metadata.
 *
 * @param metadata - Decoded RRD metadata from WASM
 * @param fileName - Original file name
 * @param dt - Estimated time step between data points [s]
 * @param entityPaths - Distinct entity paths found in the RRD data
 */
export function rrdMetadataToSimInfo(
  metadata: RrdMetadata,
  fileName: string,
  dt: number,
  entityPaths: string[],
): SimInfo {
  const satellites = entityPaths.map((path) => {
    // Extract name from entity path (last segment after /sat/)
    const satMatch = path.match(/\/sat\/(.+)/);
    const name = satMatch ? satMatch[1] : path;
    return {
      id: path,
      name,
      altitude: metadata.altitude ?? 0,
      period: metadata.period ?? 0,
      perturbations: [] as string[],
    };
  });

  // If no satellite entities found, create a default one
  if (satellites.length === 0) {
    satellites.push({
      id: "default",
      name: fileName,
      altitude: 0,
      period: 0,
      perturbations: [],
    });
  }

  return {
    mu: metadata.mu ?? DEFAULT_MU,
    dt,
    output_interval: dt,
    stream_interval: dt,
    central_body: metadata.body_name ?? "earth",
    central_body_radius: metadata.body_radius ?? DEFAULT_RADIUS,
    epoch_jd: metadata.epoch_jd ?? undefined,
    satellites,
  };
}
