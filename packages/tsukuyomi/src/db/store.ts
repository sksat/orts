import type { AsyncDuckDBConnection } from "@duckdb/duckdb-wasm";
import type { TableSchema, TimePoint, ChartDataMap } from "../types.js";

// ---------------------------------------------------------------------------
// SQL builders (pure functions, testable without DuckDB)
// ---------------------------------------------------------------------------

/**
 * Generate a CREATE OR REPLACE TABLE statement from a schema definition.
 */
export function buildCreateTableSQL(schema: TableSchema): string {
  const cols = schema.columns.map((c) => `${c.name} ${c.type}`).join(", ");
  return `CREATE OR REPLACE TABLE ${schema.tableName} (${cols})`;
}

/**
 * Generate an INSERT INTO ... VALUES statement for a batch of points.
 * Returns an empty string when the batch is empty.
 */
export function buildInsertSQL<T extends TimePoint>(
  schema: TableSchema<T>,
  points: T[],
): string {
  if (points.length === 0) return "";
  const values = points
    .map((p) => `(${schema.toRow(p).join(",")})`)
    .join(",");
  return `INSERT INTO ${schema.tableName} VALUES ${values}`;
}

/**
 * Build a SELECT query for derived quantities with optional query-time
 * downsampling via time-bucket partitioning.
 *
 * When maxPoints is specified and > 0, divides the time range into
 * maxPoints equal-duration buckets and picks the first point in each
 * bucket. This ensures even *temporal* coverage regardless of data
 * density — critical when sparse overview data and dense streaming
 * data coexist in the same table.
 */
export function buildDerivedQuery(
  schema: TableSchema,
  tMin?: number,
  maxPoints?: number,
): string {
  const whereClause = tMin != null ? `WHERE t >= ${tMin}` : "";
  const maxPts = maxPoints ?? 0;

  // Build the SELECT column list: always include t, plus derived expressions
  const derivedCols = schema.derived
    .map((d) => `${d.sql} AS ${d.name}`)
    .join(", ");
  const selectColumns = derivedCols ? `t, ${derivedCols}` : "t";

  // Base column names for the filtered CTE (all raw columns needed by derived expressions)
  const baseCols = schema.columns.map((c) => c.name).join(", ");

  // No downsampling: simple query
  if (maxPts <= 0) {
    return `SELECT ${selectColumns} FROM ${schema.tableName} ${whereClause} ORDER BY t`;
  }

  // Time-bucket downsampling: divide time range into maxPoints equal-duration
  // buckets and pick the first point (by t) in each bucket. This distributes
  // the display budget evenly across time, not across row count.
  return (
    `WITH filtered AS (SELECT ${baseCols} FROM ${schema.tableName} ${whereClause}), ` +
    `stats AS (SELECT MIN(t) AS t_lo, MAX(t) AS t_hi, COUNT(*) AS total FROM filtered), ` +
    `bucketed AS (SELECT f.*, ` +
    `CASE WHEN s.t_hi = s.t_lo THEN 0 ` +
    `ELSE LEAST(CAST(FLOOR((f.t - s.t_lo) / (s.t_hi - s.t_lo) * ${maxPts}) AS INTEGER), ${maxPts} - 1) ` +
    `END AS bucket, s.total ` +
    `FROM filtered f, stats s), ` +
    `ranked AS (SELECT *, ROW_NUMBER() OVER (PARTITION BY bucket ORDER BY t) AS rn FROM bucketed) ` +
    `SELECT ${selectColumns} FROM ranked WHERE total <= ${maxPts} OR rn = 1 ORDER BY t`
  );
}

// ---------------------------------------------------------------------------
// Async DuckDB operations
// ---------------------------------------------------------------------------

const BATCH_SIZE = 1000;

/**
 * Create (or replace) the table described by the schema.
 */
export async function createTable(
  conn: AsyncDuckDBConnection,
  schema: TableSchema,
): Promise<void> {
  await conn.query(buildCreateTableSQL(schema));
}

/**
 * Insert an array of points into the table in batches of 1000.
 */
export async function insertPoints<T extends TimePoint>(
  conn: AsyncDuckDBConnection,
  schema: TableSchema<T>,
  points: T[],
): Promise<void> {
  if (points.length === 0) return;
  for (let i = 0; i < points.length; i += BATCH_SIZE) {
    const batch = points.slice(i, i + BATCH_SIZE);
    const sql = buildInsertSQL(schema, batch);
    if (sql) await conn.query(sql);
  }
}

/**
 * Delete all rows from the table.
 */
export async function clearTable(
  conn: AsyncDuckDBConnection,
  schema: TableSchema,
): Promise<void> {
  await conn.query(`DELETE FROM ${schema.tableName}`);
}

/**
 * Run the derived query and return results as a ChartDataMap.
 */
export async function queryDerived(
  conn: AsyncDuckDBConnection,
  schema: TableSchema,
  tMin?: number,
  maxPoints?: number,
): Promise<ChartDataMap> {
  const sql = buildDerivedQuery(schema, tMin, maxPoints);
  const result = await conn.query(sql);

  const t = result.getChildAt(0)!.toArray() as Float64Array;
  const map: ChartDataMap = { t };

  for (let i = 0; i < schema.derived.length; i++) {
    const col = result.getChildAt(i + 1)!.toArray() as Float64Array;
    map[schema.derived[i].name] = col;
  }

  return map;
}

/**
 * Replace data in a time range with higher-resolution points.
 * Deletes existing rows in [tMin, tMax] then inserts the new points.
 */
export async function replaceRange<T extends TimePoint>(
  conn: AsyncDuckDBConnection,
  schema: TableSchema<T>,
  tMin: number,
  tMax: number,
  points: T[],
): Promise<void> {
  await conn.query(
    `DELETE FROM ${schema.tableName} WHERE t >= ${tMin} AND t <= ${tMax}`,
  );
  await insertPoints(conn, schema, points);
}
