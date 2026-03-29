import type { AsyncDuckDBConnection } from "@duckdb/duckdb-wasm";
import type { ChartDataMap, TableSchema, TimePoint } from "../types.js";

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
export function buildInsertSQL<T extends TimePoint>(schema: TableSchema<T>, points: T[]): string {
  if (points.length === 0) return "";
  const sqlVal = (v: number | null | undefined): string =>
    v == null || !Number.isFinite(v) ? "NULL" : String(v);
  const values = points.map((p) => `(${schema.toRow(p).map(sqlVal).join(",")})`).join(",");
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
  tMax?: number,
): string {
  const whereClause = tMin != null ? `WHERE t >= ${tMin}` : "";
  const maxPts = maxPoints ?? 0;

  // Build the SELECT column list: always include t, plus derived expressions
  const derivedCols = schema.derived.map((d) => `${d.sql} AS ${d.name}`).join(", ");
  const selectColumns = derivedCols ? `t, ${derivedCols}` : "t";

  // Base column names for the filtered CTE (all raw columns needed by derived expressions)
  const baseCols = schema.columns.map((c) => c.name).join(", ");

  // No downsampling: simple query
  if (maxPts <= 0) {
    return `SELECT ${selectColumns} FROM ${schema.tableName} ${whereClause} ORDER BY t`;
  }

  // Time-bucket downsampling: divide the time range [tMin, tMax] into
  // maxPoints equal-duration buckets and pick the first row from each.
  // This ensures even *temporal* coverage regardless of data density —
  // sparse overview regions retain proportional representation even when
  // dense streaming data dominates by row count.
  //
  // When tMax is provided, all tables use the same t_hi for bucket
  // boundaries, ensuring aligned timestamps across multi-table queries.
  const tHiExpr = tMax != null ? `${tMax} AS t_hi` : `MAX(t) AS t_hi`;

  return (
    `WITH filtered AS (SELECT ${baseCols} FROM ${schema.tableName} ${whereClause}), ` +
    `bounds AS (SELECT MIN(t) AS t_lo, ${tHiExpr}, COUNT(*) AS total FROM filtered), ` +
    `bucketed AS (SELECT f.*, ` +
    `CASE WHEN b.t_hi = b.t_lo THEN 0 ` +
    `ELSE LEAST(GREATEST(CAST(FLOOR((CAST(f.t AS DOUBLE) - CAST(b.t_lo AS DOUBLE)) ` +
    `* ${maxPts}.0 / (CAST(b.t_hi AS DOUBLE) - CAST(b.t_lo AS DOUBLE))) AS INTEGER), 0), ${maxPts} - 1) ` +
    `END AS bucket, b.total FROM filtered f, bounds b), ` +
    `ranked AS (SELECT *, ROW_NUMBER() OVER (PARTITION BY bucket ORDER BY t) AS rn FROM bucketed) ` +
    `SELECT ${selectColumns} FROM (` +
    `SELECT * FROM ranked WHERE total <= ${maxPts} OR rn = 1 ` +
    `UNION ` +
    `SELECT * FROM ranked WHERE t = (SELECT MAX(t) FROM filtered)` +
    `) sub ORDER BY t`
  );
}

// ---------------------------------------------------------------------------
// Compaction SQL builders
// ---------------------------------------------------------------------------

/**
 * Build SQL to create a temp table of "keeper" t values from old data.
 * Uses NTILE to divide old rows into equal-count buckets, keeping the
 * earliest t (MIN) from each bucket as the representative point.
 */
export function buildCompactKeepersSQL(
  tableName: string,
  cutoffT: number,
  targetOldRows: number,
): string {
  return (
    `CREATE TEMP TABLE IF NOT EXISTS _compact_keepers AS ` +
    `WITH old_data AS (` +
    `SELECT t, NTILE(${targetOldRows}) OVER (ORDER BY t) AS bucket ` +
    `FROM ${tableName} WHERE t < ${cutoffT}) ` +
    `SELECT MIN(t) AS t FROM old_data GROUP BY bucket`
  );
}

/**
 * Build SQL to delete non-keeper old rows (those older than cutoff
 * and not in the keepers temp table).
 */
export function buildCompactDeleteSQL(tableName: string, cutoffT: number): string {
  return (
    `DELETE FROM ${tableName} ` +
    `WHERE t < ${cutoffT} AND t NOT IN (SELECT t FROM _compact_keepers)`
  );
}

// ---------------------------------------------------------------------------
// Async DuckDB operations
// ---------------------------------------------------------------------------

const BATCH_SIZE = 1000;

/**
 * Create (or replace) the table described by the schema.
 */
export async function createTable(conn: AsyncDuckDBConnection, schema: TableSchema): Promise<void> {
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
export async function clearTable(conn: AsyncDuckDBConnection, schema: TableSchema): Promise<void> {
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
  tMax?: number,
): Promise<ChartDataMap> {
  const sql = buildDerivedQuery(schema, tMin, maxPoints, tMax);
  const result = await conn.query(sql);

  const t = result.getChildAt(0)!.toArray() as Float64Array;
  const map: ChartDataMap = { t };

  for (let i = 0; i < schema.derived.length; i++) {
    const col = result.getChildAt(i + 1)!.toArray() as Float64Array;
    map[schema.derived[i].name] = col;
  }

  return map;
}

/** Configuration for periodic DuckDB compaction. */
export interface CompactOptions {
  /** Compact when total rows exceed this threshold. */
  maxRows: number;
  /** Keep this many recent rows at full resolution. */
  keepRecentRows: number;
  /** Downsample old data to this many representative rows. */
  targetOldRows: number;
}

/** Default compaction thresholds (see plan for rationale). */
export const COMPACT_DEFAULTS: CompactOptions = {
  maxRows: 50_000,
  keepRecentRows: 10_000,
  targetOldRows: 5_000,
};

/**
 * Compact old data in DuckDB to control memory usage.
 *
 * Keeps the most recent `keepRecentRows` at full resolution and
 * downsamples older rows to `targetOldRows` using NTILE bucketing.
 * Returns true if compaction was performed.
 *
 * Assumes t values are unique (one per simulation step).
 */
export async function compactTable(
  conn: AsyncDuckDBConnection,
  schema: TableSchema,
  opts: CompactOptions = COMPACT_DEFAULTS,
): Promise<boolean> {
  // 1. Check row count
  const countRes = await conn.query(`SELECT COUNT(*) AS total FROM ${schema.tableName}`);
  const total = Number(countRes.getChildAt(0)!.get(0));
  if (total <= opts.maxRows) return false;

  // 2. Find cutoff t (boundary between old and recent)
  if (opts.keepRecentRows >= total) return false;
  const cutoffRes = await conn.query(
    `SELECT t FROM (SELECT t, ROW_NUMBER() OVER (ORDER BY t DESC) AS rn ` +
      `FROM ${schema.tableName}) sub WHERE rn = ${opts.keepRecentRows} LIMIT 1`,
  );
  const cutoffT = Number(cutoffRes.getChildAt(0)!.get(0));

  // 3. Create temp table of keeper t values, delete non-keepers, cleanup
  try {
    await conn.query(buildCompactKeepersSQL(schema.tableName, cutoffT, opts.targetOldRows));
    await conn.query(buildCompactDeleteSQL(schema.tableName, cutoffT));
  } finally {
    await conn.query(`DROP TABLE IF EXISTS _compact_keepers`);
  }

  return true;
}
