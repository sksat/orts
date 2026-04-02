/**
 * Pure functions for parsing CSV orbit data lines.
 *
 * Shared between the main thread (orbit.ts) and the CSV parse Worker.
 * No DOM or React dependencies.
 */

import type { CSVMetadata, OrbitPoint } from "../orbit.js";

/**
 * Try to parse a CSV comment line as metadata.
 * Returns the key-value pair if the line matches `# key = value`, else null.
 */
export function parseMetadataLine(line: string, metadata: CSVMetadata): boolean {
  const match = line.match(/^#\s*(\w+)\s*=\s*(.+)/);
  if (!match) return false;

  const [, key, value] = match;
  switch (key) {
    case "epoch_jd":
      metadata.epochJd = Number(value.trim());
      break;
    case "mu":
      metadata.mu = Number(value.trim().split(/\s/)[0]);
      break;
    case "central_body":
      metadata.centralBody = value.trim();
      break;
    case "central_body_radius":
      metadata.centralBodyRadius = Number(value.trim().split(/\s/)[0]);
      break;
    case "satellite": {
      const trimmed = value.trim();
      if (trimmed) metadata.satelliteName = trimmed;
      break;
    }
    default:
      return false;
  }
  return true;
}

/**
 * Parse a single CSV data line into an OrbitPoint, or return null if invalid.
 *
 * Expected format: `t,x,y,z,vx,vy,vz[,a,e,inc,raan,omega,nu]`
 * Minimum 7 numeric fields required.
 */
export function parseDataLine(line: string): OrbitPoint | null {
  const parts = line.split(",").map((s) => s.trim());
  if (parts.length < 7) return null;

  const nums = parts.map(Number);
  if (nums.some(Number.isNaN)) return null;

  return {
    t: nums[0],
    x: nums[1],
    y: nums[2],
    z: nums[3],
    vx: nums[4],
    vy: nums[5],
    vz: nums[6],
    a: nums[7] ?? 0,
    e: nums[8] ?? 0,
    inc: nums[9] ?? 0,
    raan: nums[10] ?? 0,
    omega: nums[11] ?? 0,
    nu: nums[12] ?? 0,
    accel_gravity: 0,
    accel_drag: 0,
    accel_srp: 0,
    accel_third_body_sun: 0,
    accel_third_body_moon: 0,
  };
}

/**
 * Create a fresh CSVMetadata object with all fields null.
 */
export function emptyMetadata(): CSVMetadata {
  return {
    epochJd: null,
    mu: null,
    centralBody: null,
    centralBodyRadius: null,
    satelliteName: null,
  };
}
