// uneri - DuckDB + uPlot time-series charting library

export type {
  MultiSeriesData,
  SeriesConfig,
} from "./components/TimeSeriesChart.js";
// Components
export {
  buildMultiSeriesConfig,
  safeYRange,
  TimeSeriesChart,
} from "./components/TimeSeriesChart.js";
// DB
export { initDuckDB } from "./db/duckdb.js";
export { IngestBuffer } from "./db/IngestBuffer.js";
export type { CompactOptions } from "./db/store.js";
export {
  buildCompactDeleteSQL,
  buildCompactKeepersSQL,
  buildCreateTableSQL,
  buildDerivedQuery,
  buildInsertSQL,
  COMPACT_DEFAULTS,
  clearTable,
  compactTable,
  createTable,
  insertPoints,
  queryDerived,
} from "./db/store.js";
export type { UseDuckDBReturn } from "./hooks/useDuckDB.js";
// Hooks
export { useDuckDB } from "./hooks/useDuckDB.js";
export type {
  TimeRange,
  UseTimeSeriesStoreOptions,
  UseTimeSeriesStoreReturn,
} from "./hooks/useTimeSeriesStore.js";
export {
  computeTMin,
  DISPLAY_MAX_POINTS,
  useTimeSeriesStore,
} from "./hooks/useTimeSeriesStore.js";
// Types
export type {
  ChartDataMap,
  ColumnDef,
  ColumnType,
  DerivedColumn,
  TableSchema,
  TimePoint,
} from "./types.js";
export type {
  AlignedMultiSeries,
  NamedTimeSeries,
} from "./utils/alignTimeSeries.js";
export { alignTimeSeries } from "./utils/alignTimeSeries.js";
// Utilities
export {
  lowerBound,
  quantizeChartTime,
  sliceArrays,
  upperBound,
} from "./utils/chartViewport.js";
