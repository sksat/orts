/**
 * Message protocol for the chart data Web Worker.
 *
 * The Worker owns DuckDB and the entire tick loop (cold/hot query, merge, trim).
 * The main thread only sends data points and configuration, and receives
 * ready-to-render ChartDataMap via zero-copy ArrayBuffer transfer.
 */

import type { DuckDBInitOptions } from "../db/duckdb.js";
import type { TimeRange } from "../hooks/useTimeSeriesStore.js";
import type { ColumnDef, DerivedColumn } from "../types.js";

// Shared types

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

/**
 * Do the two schemas store the same columns, in the same order, with the same
 * types? The table name is not part of this: the multi-satellite Worker
 * overrides it per satellite, so a name change there means nothing.
 */
export function sameColumns(a: WorkerTableSchema, b: WorkerTableSchema): boolean {
  if (a.columns.length !== b.columns.length) return false;
  return a.columns.every((c, i) => c.name === b.columns[i].name && c.type === b.columns[i].type);
}

/** Do the two schemas derive the same chart columns from the same expressions? */
export function sameDerived(a: WorkerTableSchema, b: WorkerTableSchema): boolean {
  if (a.derived.length !== b.derived.length) return false;
  return a.derived.every((d, i) => d.name === b.derived[i].name && d.sql === b.derived[i].sql);
}

// Main thread → Worker messages

export type MainToWorkerMessage =
  | {
      type: "init";
      schema: WorkerTableSchema;
      tickInterval?: number;
      coldRefreshEveryN?: number;
      hotRowBudget?: number;
      /** How the Worker should source DuckDB-wasm assets (self-host vs CDN). */
      duckDB?: DuckDBInitOptions;
    }
  | { type: "ingest"; rows: RowTuple[]; latestT: number }
  | { type: "rebuild"; rows: RowTuple[]; latestT: number }
  | {
      /**
       * Replace the schema the Worker computes derived columns with.
       *
       * Row tuples carry no column names, so the main thread must send this
       * before any row produced by the new `toRow`. When only the derived
       * expressions change the stored rows are kept and the next query uses
       * the new expressions; when the column list changes the table is
       * recreated and its content dropped (the Worker cannot re-derive rows
       * it no longer has).
       */
      type: "update-schema";
      schema: WorkerTableSchema;
    }
  | {
      type: "configure";
      timeRange: TimeRange;
      maxPoints: number;
    }
  | { type: "dispose" }
  | { type: "debug-query"; id: number; query: "row-count" }
  | { type: "zoom-query"; id: number; tMin: number; tMax: number; maxPoints: number };

// Multi-satellite Worker messages

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
      /** How the Worker should source DuckDB-wasm assets (self-host vs CDN). */
      duckDB?: DuckDBInitOptions;
    }
  | { type: "multi-ingest"; satelliteId: string; rows: RowTuple[]; latestT: number }
  | { type: "multi-rebuild"; satelliteId: string; rows: RowTuple[]; latestT: number }
  | { type: "multi-configure"; timeRange: TimeRange; maxPoints: number }
  | {
      /** Replace the base schema for every satellite table (see `update-schema`). */
      type: "multi-update-schema";
      baseSchema: WorkerTableSchema;
    }
  | {
      type: "multi-update-configs";
      satelliteConfigs: WorkerSatelliteConfig[];
      metricNames: string[];
    }
  | {
      /**
       * Ad-hoc zoom query: return aligned multi-series data for the absolute
       * time window `[tMin, tMax]`, bypassing the configured `timeRange`.
       * Each satellite's DuckDB is queried for the same window; results
       * are aligned and returned as a one-shot response keyed by `id`.
       */
      type: "multi-zoom-query";
      id: number;
      tMin: number;
      tMax: number;
      maxPoints: number;
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
  | {
      /** One-shot response to a `multi-zoom-query`, correlated by `id`. */
      type: "multi-zoom-result";
      id: number;
      metrics: SerializedMultiSeriesData[];
    }
  | { type: "error"; message: string };

// Worker → Main thread messages (single-satellite)

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
