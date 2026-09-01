/**
 * Convert CSV metadata into a SimInfo-compatible object.
 *
 * CSV files don't carry the full SimInfo structure that a WebSocket server
 * provides, so we fill in sensible defaults for missing fields.
 */

import type { CSVMetadata } from "../orbit.js";
import type { RrdMetadata } from "../wasm/rrdWasmInit.js";
import type { CentralBody } from "./centralBody.js";
import type { SimInfo } from "./types.js";

/**
 * Build a SimInfo from CSV metadata.
 *
 * @param metadata - Parsed CSV comment headers
 * @param fileName - Original file name (used as satellite display name)
 * @param dt - Estimated time step between data points [s]
 */
export function csvMetadataToSimInfo(
  metadata: CSVMetadata,
  fileName: string,
  dt: number,
  centralBody: CentralBody,
): SimInfo {
  const satellites =
    metadata.satellites && metadata.satellites.length > 0
      ? metadata.satellites.map((id) => ({
          id,
          name: id,
          altitude: 0,
          period: 0,
          perturbations: [] as string[],
          shape: null,
        }))
      : [
          {
            id: "default",
            name: metadata.satelliteName ?? fileName,
            altitude: 0,
            period: 0,
            perturbations: [] as string[],
            shape: null,
          },
        ];

  return {
    mu: centralBody.mu,
    dt,
    output_interval: dt,
    stream_interval: dt,
    central_body: centralBody.bodyId,
    central_body_radius: centralBody.bodyRadius,
    epoch_jd: metadata.epochJd,
    satellites,
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
  centralBody: CentralBody,
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
      shape: null,
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
      shape: null,
    });
  }

  return {
    mu: centralBody.mu,
    dt,
    output_interval: dt,
    stream_interval: dt,
    central_body: centralBody.bodyId,
    central_body_radius: centralBody.bodyRadius,
    epoch_jd: metadata.epoch_jd ?? null,
    satellites,
  };
}
