import type { AsyncDuckDBConnection } from "@duckdb/duckdb-wasm";
import { OrbitPoint } from "../orbit.js";

/** Columnar chart data: [t, altitude, energy, angularMomentum, velocity] */
export type ChartData = [
  Float64Array,
  Float64Array,
  Float64Array,
  Float64Array,
  Float64Array,
];

export async function createOrbitTable(
  conn: AsyncDuckDBConnection
): Promise<void> {
  await conn.query(`
    CREATE OR REPLACE TABLE orbit_points (
      t DOUBLE, x DOUBLE, y DOUBLE, z DOUBLE,
      vx DOUBLE, vy DOUBLE, vz DOUBLE
    )
  `);
}

export async function insertPoints(
  conn: AsyncDuckDBConnection,
  points: OrbitPoint[]
): Promise<void> {
  if (points.length === 0) return;
  const BATCH_SIZE = 1000;
  for (let i = 0; i < points.length; i += BATCH_SIZE) {
    const batch = points.slice(i, i + BATCH_SIZE);
    const values = batch
      .map(
        (p) => `(${p.t},${p.x},${p.y},${p.z},${p.vx},${p.vy},${p.vz})`
      )
      .join(",");
    await conn.query(`INSERT INTO orbit_points VALUES ${values}`);
  }
}

export async function clearTable(
  conn: AsyncDuckDBConnection
): Promise<void> {
  await conn.query("DELETE FROM orbit_points");
}

/**
 * Build SQL for derived orbital quantities with optional query-time downsampling.
 *
 * When maxPoints is specified and the filtered row count exceeds it, uses
 * ROW_NUMBER() to evenly sample rows while preserving first and last points.
 * This matches the server-side downsample pattern (cli/src/main.rs:275).
 */
export function buildDerivedQuery(
  mu: number,
  bodyRadius: number,
  tMin?: number,
  maxPoints?: number,
): string {
  const whereClause = tMin != null ? `WHERE t >= ${tMin}` : "";
  const maxPts = maxPoints ?? 0;

  const derivedColumns = `
    t,
    sqrt(x*x + y*y + z*z) - ${bodyRadius} AS altitude,
    (vx*vx + vy*vy + vz*vz)/2.0 - ${mu} / sqrt(x*x + y*y + z*z) AS energy,
    sqrt(
      power(y*vz - z*vy, 2) +
      power(z*vx - x*vz, 2) +
      power(x*vy - y*vx, 2)
    ) AS angular_momentum,
    sqrt(vx*vx + vy*vy + vz*vz) AS velocity`;

  // No downsampling: simple query
  if (maxPts <= 0) {
    return `SELECT ${derivedColumns} FROM orbit_points ${whereClause} ORDER BY t`;
  }

  // Query-time downsampling via ROW_NUMBER window function
  return `
    WITH filtered AS (
      SELECT t, x, y, z, vx, vy, vz
      FROM orbit_points
      ${whereClause}
    ),
    numbered AS (
      SELECT *,
        ROW_NUMBER() OVER (ORDER BY t) AS rn,
        COUNT(*) OVER () AS total
      FROM filtered
    )
    SELECT ${derivedColumns}
    FROM numbered
    WHERE total <= ${maxPts}
       OR rn = 1
       OR rn = total
       OR (rn - 1) % GREATEST(1, CAST(CEIL(total::DOUBLE / ${maxPts}) AS INTEGER)) = 0
    ORDER BY t`;
}

export async function queryDerivedQuantities(
  conn: AsyncDuckDBConnection,
  mu: number,
  bodyRadius = 6378.137,
  tMin?: number,
  maxPoints?: number,
): Promise<ChartData> {
  const sql = buildDerivedQuery(mu, bodyRadius, tMin, maxPoints);
  const result = await conn.query(sql);

  const t = result.getChildAt(0)!.toArray() as Float64Array;
  const alt = result.getChildAt(1)!.toArray() as Float64Array;
  const energy = result.getChildAt(2)!.toArray() as Float64Array;
  const angMom = result.getChildAt(3)!.toArray() as Float64Array;
  const vel = result.getChildAt(4)!.toArray() as Float64Array;

  return [t, alt, energy, angMom, vel];
}

/**
 * Replace data in a time range with higher-resolution points.
 * Used when the viewer zooms into a chart and fetches full-resolution
 * data from the server via query_range.
 */
export async function replaceRange(
  conn: AsyncDuckDBConnection,
  tMin: number,
  tMax: number,
  points: OrbitPoint[]
): Promise<void> {
  await conn.query(`DELETE FROM orbit_points WHERE t >= ${tMin} AND t <= ${tMax}`);
  await insertPoints(conn, points);
}
