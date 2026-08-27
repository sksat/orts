/**
 * Worker-based alternative to useTimeSeriesStore.
 *
 * Moves the entire DuckDB tick loop (insert, query, merge, trim) to a
 * dedicated Web Worker, keeping the main thread free for rendering.
 *
 * Same return type as useTimeSeriesStore for drop-in replacement.
 */

import { useEffect, useRef, useState } from "react";
import type { DuckDBInitOptions } from "../db/duckdb.js";
import type { IngestBuffer } from "../db/IngestBuffer.js";
import type { ChartDataMap, TableSchema, TimePoint } from "../types.js";
import { ChartDataWorkerClient } from "../worker/chartDataWorkerClient.js";
import {
  type RowTuple,
  sameColumns,
  sameDerived,
  type WorkerTableSchema,
} from "../worker/protocol.js";
import {
  DISPLAY_MAX_POINTS,
  type TimeRange,
  type UseTimeSeriesStoreReturn,
} from "./useTimeSeriesStore.js";

export interface UseTimeSeriesStoreWorkerOptions<T extends TimePoint> {
  schema: TableSchema<T>;
  ingestBufferRef: React.RefObject<IngestBuffer<T>>;
  /** Show only last N seconds of data, or null for all history. */
  timeRange?: TimeRange;
  /** Maximum number of points to display (default: DISPLAY_MAX_POINTS). */
  maxPoints?: number;
  /** Polling interval in ms for draining the IngestBuffer (default: 250). */
  drainInterval?: number;
  /** Worker tick interval in ms (default: 250). */
  tickInterval?: number;
  /** Run cold (full downsampled) refresh every Nth tick (default: 20). */
  coldRefreshEveryN?: number;
  /** Trigger cold refresh when hot buffer exceeds this many rows (default: 500). */
  hotRowBudget?: number;
  /** Optional ref to receive the worker client instance (for debug queries etc.). */
  clientRef?: React.MutableRefObject<ChartDataWorkerClient | null>;
  /** Set to false to disable the Worker (no Worker is spawned). Default: true. */
  enabled?: boolean;
  /**
   * How the Worker should source DuckDB-wasm assets. Pass self-hosted bundle
   * URLs here to avoid the jsDelivr CDN. Defaults to the CDN when omitted.
   */
  duckDB?: DuckDBInitOptions;
}

/** Extract the serializable portion of a TableSchema (excluding toRow). */
export function toWorkerSchema(schema: TableSchema): WorkerTableSchema {
  return {
    tableName: schema.tableName,
    columns: schema.columns,
    derived: schema.derived,
  };
}

/** The subset of `ChartDataWorkerClient` the drain step needs. */
export interface WorkerSyncTarget {
  updateSchema(schema: WorkerTableSchema): void;
  ingest(rows: RowTuple[], latestT: number): void;
  rebuild(rows: RowTuple[], latestT: number): void;
  configure(timeRange: TimeRange, maxPoints: number): void;
}

/** What the Worker was last told, so changes can be detected and forwarded. */
export interface WorkerSyncState<T extends TimePoint> {
  schema: TableSchema<T>;
  timeRange: TimeRange;
  maxPoints: number;
}

/**
 * One drain step: forward a schema change, then the buffered points (as a
 * rebuild or an incremental ingest), then configuration changes.
 *
 * The schema goes first because row tuples carry no column names — the Worker
 * must know the current schema before it sees rows produced by it. `sent` is
 * updated in place with what was forwarded.
 */
export function drainToWorker<T extends TimePoint>(
  client: WorkerSyncTarget,
  buffer: IngestBuffer<T>,
  current: WorkerSyncState<T>,
  sent: WorkerSyncState<T>,
): void {
  // Schema changes must reach the Worker: it bakes mu/bodyRadius-style
  // constants into its derived SQL, so a stale schema silently returns
  // values derived from the previous central body. The content is compared,
  // not just the identity, so a caller that rebuilds an equal schema object
  // every render does not send a message every drain.
  if (current.schema !== sent.schema) {
    const next = toWorkerSchema(current.schema);
    const prev = toWorkerSchema(sent.schema);
    if (prev.tableName !== next.tableName || !sameColumns(prev, next) || !sameDerived(prev, next)) {
      client.updateSchema(next);
    }
    sent.schema = current.schema;
  }

  const rebuildData = buffer.consumeRebuild();
  if (rebuildData !== null) {
    client.rebuild(
      rebuildData.map((p) => current.schema.toRow(p)),
      buffer.latestT,
    );
  } else {
    const points = buffer.drain();
    if (points.length > 0) {
      client.ingest(
        points.map((p) => current.schema.toRow(p)),
        buffer.latestT,
      );
    }
  }

  if (current.timeRange !== sent.timeRange || current.maxPoints !== sent.maxPoints) {
    client.configure(current.timeRange, current.maxPoints);
    sent.timeRange = current.timeRange;
    sent.maxPoints = current.maxPoints;
  }
}

export function useTimeSeriesStoreWorker<T extends TimePoint>(
  options: UseTimeSeriesStoreWorkerOptions<T>,
): UseTimeSeriesStoreReturn {
  const {
    schema,
    ingestBufferRef,
    timeRange = null,
    maxPoints = DISPLAY_MAX_POINTS,
    drainInterval = 250,
    tickInterval,
    coldRefreshEveryN,
    hotRowBudget,
    clientRef: externalClientRef,
    enabled = true,
    duckDB,
  } = options;
  const duckDBRef = useRef(duckDB);
  duckDBRef.current = duckDB;

  const [data, setData] = useState<ChartDataMap | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  // Refs for stable access
  const schemaRef = useRef(schema);
  schemaRef.current = schema;
  const timeRangeRef = useRef(timeRange);
  timeRangeRef.current = timeRange;
  const maxPointsRef = useRef(maxPoints);
  maxPointsRef.current = maxPoints;
  // `enabled` is depended on directly in the effect below so the worker
  // can be started/stopped as the hook transitions between enabled and
  // disabled states. The ref pattern used for the other props (refs for
  // "capture-once" values) does not apply here because this is a
  // lifecycle gate, not a drain-time read.

  // Track what the Worker was last told, to detect changes
  const sentRef = useRef<WorkerSyncState<T>>({ schema, timeRange, maxPoints });

  const clientRef = useRef<ChartDataWorkerClient | null>(null);

  useEffect(() => {
    if (!enabled) return;

    const client = new ChartDataWorkerClient();
    clientRef.current = client;
    if (externalClientRef) externalClientRef.current = client;

    client.onData((chartData) => {
      setData(chartData);
      setIsLoading(false);
    });

    client.onError((message) => {
      console.warn("useTimeSeriesStoreWorker: Worker error:", message);
    });

    // Initialize Worker with schema and tick parameters
    client.init(toWorkerSchema(schemaRef.current), {
      tickInterval,
      coldRefreshEveryN,
      hotRowBudget,
      duckDB: duckDBRef.current,
    });

    // Send initial configuration
    client.configure(timeRangeRef.current, maxPointsRef.current);
    sentRef.current = {
      schema: schemaRef.current,
      timeRange: timeRangeRef.current,
      maxPoints: maxPointsRef.current,
    };

    // Lightweight drain loop: pull from IngestBuffer → toRow() → send to Worker
    let cancelled = false;
    let drainTimer = 0;

    const drain = () => {
      if (cancelled) return;

      drainToWorker(
        client,
        ingestBufferRef.current,
        {
          schema: schemaRef.current,
          timeRange: timeRangeRef.current,
          maxPoints: maxPointsRef.current,
        },
        sentRef.current,
      );

      drainTimer = window.setTimeout(drain, drainInterval) as unknown as number;
    };

    drainTimer = window.setTimeout(drain, drainInterval) as unknown as number;

    return () => {
      cancelled = true;
      clearTimeout(drainTimer);
      client.dispose();
      clientRef.current = null;
      if (externalClientRef) externalClientRef.current = null;
    };
    // Everything else inside the effect is accessed via refs so their
    // identity changes do not re-create the worker; `enabled` is the
    // only lifecycle gate.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled]);

  return { data, isLoading };
}
