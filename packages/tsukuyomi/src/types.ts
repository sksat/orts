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

/** Defines a derived quantity computed from base columns via SQL. */
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
  toRow(point: T): number[];
}

/** Generic chart data: keyed by column name. */
export type ChartDataMap = {
  t: Float64Array;
  [derivedName: string]: Float64Array;
};
