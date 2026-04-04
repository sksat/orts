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
// Worker → Main thread messages
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
