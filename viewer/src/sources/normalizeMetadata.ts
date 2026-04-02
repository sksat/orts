/**
 * Convert CSV metadata into a SimInfo-compatible object.
 *
 * CSV files don't carry the full SimInfo structure that a WebSocket server
 * provides, so we fill in sensible defaults for missing fields.
 */

import type { CSVMetadata } from "../orbit.js";
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
        name: fileName,
        altitude: 0,
        period: 0,
        perturbations: [],
      },
    ],
  };
}
