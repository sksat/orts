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

const R_EARTH = 6378.137;

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

export async function queryDerivedQuantities(
  conn: AsyncDuckDBConnection,
  mu: number
): Promise<ChartData> {
  const result = await conn.query(`
    SELECT
      t,
      sqrt(x*x + y*y + z*z) - ${R_EARTH} AS altitude,
      (vx*vx + vy*vy + vz*vz)/2.0 - ${mu} / sqrt(x*x + y*y + z*z) AS energy,
      sqrt(
        power(y*vz - z*vy, 2) +
        power(z*vx - x*vz, 2) +
        power(x*vy - y*vx, 2)
      ) AS angular_momentum,
      sqrt(vx*vx + vy*vy + vz*vz) AS velocity
    FROM orbit_points
    ORDER BY t
  `);

  const t = result.getChildAt(0)!.toArray() as Float64Array;
  const alt = result.getChildAt(1)!.toArray() as Float64Array;
  const energy = result.getChildAt(2)!.toArray() as Float64Array;
  const angMom = result.getChildAt(3)!.toArray() as Float64Array;
  const vel = result.getChildAt(4)!.toArray() as Float64Array;

  return [t, alt, energy, angMom, vel];
}
