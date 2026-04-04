/**
 * Worker-based alternative to useTimeSeriesStore.
 *
 * Moves the entire DuckDB tick loop (insert, query, merge, trim) to a
 * dedicated Web Worker, keeping the main thread free for rendering.
 *
 * Same return type as useTimeSeriesStore for drop-in replacement.
 */

import { useEffect, useRef, useState } from "react";
import type { IngestBuffer } from "../db/IngestBuffer.js";
import type { ChartDataMap, TableSchema, TimePoint } from "../types.js";
import { ChartDataWorkerClient } from "../worker/chartDataWorkerClient.js";
import type { WorkerTableSchema } from "../worker/protocol.js";
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
}

/** Extract the serializable portion of a TableSchema (excluding toRow). */
function toWorkerSchema(schema: TableSchema): WorkerTableSchema {
  return {
    tableName: schema.tableName,
    columns: schema.columns,
    derived: schema.derived,
  };
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
  } = options;

  const [data, setData] = useState<ChartDataMap | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  // Refs for stable access
  const schemaRef = useRef(schema);
  schemaRef.current = schema;
  const timeRangeRef = useRef(timeRange);
  timeRangeRef.current = timeRange;
  const maxPointsRef = useRef(maxPoints);
  maxPointsRef.current = maxPoints;
  const enabledRef = useRef(enabled);
  enabledRef.current = enabled;

  // Track previous config to detect changes
  const prevTimeRange = useRef(timeRange);
  const prevMaxPoints = useRef(maxPoints);

  const clientRef = useRef<ChartDataWorkerClient | null>(null);

  useEffect(() => {
    if (!enabledRef.current) return;

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
    });

    // Send initial configuration
    client.configure(timeRangeRef.current, maxPointsRef.current);

    // Lightweight drain loop: pull from IngestBuffer → toRow() → send to Worker
    let cancelled = false;
    let drainTimer = 0;

    const drain = () => {
      if (cancelled) return;

      const buffer = ingestBufferRef.current;

      // Check for rebuild signal
      const rebuildData = buffer.consumeRebuild();
      if (rebuildData !== null) {
        const rows = rebuildData.map((p) => schemaRef.current.toRow(p));
        client.rebuild(rows, buffer.latestT);
      } else {
        // Normal drain
        const points = buffer.drain();
        if (points.length > 0) {
          const rows = points.map((p) => schemaRef.current.toRow(p));
          client.ingest(rows, buffer.latestT);
        }
      }

      // Check for config changes (timeRange, maxPoints)
      // Note: schema changes are not detected here. In practice, schema only
      // changes when mu/bodyRadius change (server reconnect), which remounts
      // the component tree and re-runs this effect.
      if (
        timeRangeRef.current !== prevTimeRange.current ||
        maxPointsRef.current !== prevMaxPoints.current
      ) {
        client.configure(timeRangeRef.current, maxPointsRef.current);
        prevTimeRange.current = timeRangeRef.current;
        prevMaxPoints.current = maxPointsRef.current;
      }

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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return { data, isLoading };
}
