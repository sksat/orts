/**
 * Message protocol for the chart data Web Worker.
 *
 * The Worker owns DuckDB and the entire tick loop (cold/hot query, merge, trim).
 * The main thread only sends data points and configuration, and receives
 * ready-to-render ChartDataMap via zero-copy ArrayBuffer transfer.
 */

import type { TimeRange } from "../hooks/useTimeSeriesStore.js";
import type { ColumnDef, DerivedColumn } from "../types.js";

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

/** A single row as a tuple of nullable numbers, produced by schema.toRow(). */
export type RowTuple = (number | null)[];

/**
 * Serializable subset of TableSchema — excludes the `toRow` function
 * which cannot be transferred to a Worker.
 */
export interface WorkerTableSchema {
  tableName: string;
  columns: ColumnDef[];
  derived: DerivedColumn[];
}

// ---------------------------------------------------------------------------
// Main thread → Worker messages
// ---------------------------------------------------------------------------

export type MainToWorkerMessage =
  | {
      type: "init";
      schema: WorkerTableSchema;
      tickInterval?: number;
      coldRefreshEveryN?: number;
      hotRowBudget?: number;
    }
  | { type: "ingest"; rows: RowTuple[]; latestT: number }
  | { type: "rebuild"; rows: RowTuple[]; latestT: number }
  | {
      type: "configure";
      timeRange: TimeRange;
      maxPoints: number;
    }
  | { type: "dispose" }
  | { type: "debug-query"; id: number; query: "row-count" }
  | { type: "zoom-query"; id: number; tMin: number; tMax: number; maxPoints: number };

// ---------------------------------------------------------------------------
// Multi-satellite Worker messages
// ---------------------------------------------------------------------------

/** Serializable satellite config (matches SatelliteConfig from buildMultiChartData). */
export interface WorkerSatelliteConfig {
  id: string;
  label: string;
  color: string;
}

export type MultiMainToWorkerMessage =
  | {
      type: "multi-init";
      baseSchema: WorkerTableSchema;
      satelliteConfigs: WorkerSatelliteConfig[];
      metricNames: string[];
      tickInterval?: number;
      queryEveryN?: number;
      compactEveryN?: number;
    }
  | { type: "multi-ingest"; satelliteId: string; rows: RowTuple[]; latestT: number }
  | { type: "multi-rebuild"; satelliteId: string; rows: RowTuple[]; latestT: number }
  | { type: "multi-configure"; timeRange: TimeRange; maxPoints: number }
  | {
      type: "multi-update-configs";
      satelliteConfigs: WorkerSatelliteConfig[];
      metricNames: string[];
    }
  | { type: "dispose" };

/**
 * Serialized MultiSeriesData for a single metric.
 * `t` is the aligned time array, `values` are per-satellite value arrays.
 */
export interface SerializedMultiSeriesData {
  metricName: string;
  seriesLabels: string[];
  seriesColors: string[];
  /** [t, values[0], values[1], ...] — all transferred. */
  buffers: ArrayBuffer[];
}

export type MultiWorkerToMainMessage =
  | { type: "ready" }
  | {
      type: "multi-chart-data";
      /** One entry per metric that has data. */
      metrics: SerializedMultiSeriesData[];
    }
  | { type: "error"; message: string };

// ---------------------------------------------------------------------------
// Worker → Main thread messages (single-satellite)
// ---------------------------------------------------------------------------

export type WorkerToMainMessage =
  | { type: "ready" }
  | {
      type: "chart-data";
      /** Column names in the same order as buffers. */
      keys: string[];
      /** One ArrayBuffer per column — transferred (zero-copy). */
      buffers: ArrayBuffer[];
    }
  | { type: "error"; message: string }
  | { type: "debug-result"; id: number; result: number }
  | {
      type: "zoom-result";
      id: number;
      keys: string[];
      buffers: ArrayBuffer[];
    };
