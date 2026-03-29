/** Minimum requirement for a data point: must have a time field. */
export interface TimePoint {
  readonly t: number;
}

/** SQL column type supported by DuckDB. */
export type ColumnType = "DOUBLE" | "INTEGER" | "BIGINT" | "FLOAT";

/** Defines a single column in the time-series table. */
export interface ColumnDef {
  name: string;
  type: ColumnType;
}

/**
 * Defines a derived quantity computed from base columns via SQL.
 *
 * **Constraint**: `sql` MUST be a row-local expression — it may only
 * reference columns from the same row. Window functions (LAG, LEAD,
 * AVG(...) OVER, ROW_NUMBER OVER, etc.) and correlated subqueries are
 * NOT supported and will produce incorrect results with incremental queries.
 */
export interface DerivedColumn {
  name: string;
  sql: string;
  unit?: string;
}

/** Full schema definition for a time-series table. */
export interface TableSchema<T extends TimePoint = TimePoint> {
  tableName: string;
  columns: ColumnDef[];
  derived: DerivedColumn[];
  toRow(point: T): (number | null)[];
}

/** Generic chart data: keyed by column name. */
export type ChartDataMap = {
  t: Float64Array;
  [derivedName: string]: Float64Array;
};
