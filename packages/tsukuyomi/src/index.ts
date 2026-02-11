// @orts/tsukuyomi - DuckDB + uPlot time-series charting library

// Types
export type {
  TimePoint,
  ColumnDef,
  ColumnType,
  DerivedColumn,
  TableSchema,
  ChartDataMap,
} from "./types.js";

// DB
export { initDuckDB } from "./db/duckdb.js";
export { IngestBuffer } from "./db/IngestBuffer.js";
export {
  createTable,
  insertPoints,
  clearTable,
  queryDerived,
  replaceRange,
  compactTable,
  COMPACT_DEFAULTS,
  buildCreateTableSQL,
  buildInsertSQL,
  buildDerivedQuery,
  buildCompactKeepersSQL,
  buildCompactDeleteSQL,
} from "./db/store.js";
export type { CompactOptions } from "./db/store.js";

// Hooks
export { useDuckDB } from "./hooks/useDuckDB.js";
export type { UseDuckDBReturn } from "./hooks/useDuckDB.js";
export {
  useTimeSeriesStore,
  computeTMin,
  DISPLAY_MAX_POINTS,
} from "./hooks/useTimeSeriesStore.js";
export type {
  TimeRange,
  UseTimeSeriesStoreOptions,
  UseTimeSeriesStoreReturn,
} from "./hooks/useTimeSeriesStore.js";

// Components
export { TimeSeriesChart, safeYRange } from "./components/TimeSeriesChart.js";

// Utilities
export {
  sliceArrays,
  quantizeChartTime,
  lowerBound,
  upperBound,
} from "./utils/chartViewport.js";
