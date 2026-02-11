import { useState, useEffect, useRef } from "react";
import type { AsyncDuckDBConnection } from "@duckdb/duckdb-wasm";
import type { TimePoint, TableSchema, ChartDataMap } from "../types.js";
import type { IngestBuffer } from "../db/IngestBuffer.js";
import { insertPoints, clearTable, queryDerived } from "../db/store.js";

/** Maximum number of points to display in charts. Query-time downsampling
 *  keeps chart rendering fast regardless of total data in DuckDB. */
export const DISPLAY_MAX_POINTS = 2000;

/** Time range for chart display: null = all history, number = last N seconds. */
export type TimeRange = number | null;

/** Compute the minimum t value for a chart query given a time range window. */
export function computeTMin(
  timeRange: TimeRange,
  latestT: number,
): number | undefined {
  if (timeRange == null) return undefined;
  return latestT - timeRange;
}

export interface UseTimeSeriesStoreOptions<T extends TimePoint> {
  conn: AsyncDuckDBConnection | null;
  schema: TableSchema<T>;
  mode: "replay" | "realtime";
  replayPoints: T[] | null;
  ingestBufferRef: React.RefObject<IngestBuffer<T>>;
  /** Show only last N seconds of data, or null for all history. */
  timeRange?: TimeRange;
  /** Maximum number of points to display (default: DISPLAY_MAX_POINTS). */
  maxPoints?: number;
  /** Polling interval in ms for realtime mode (default: 500). */
  tickInterval?: number;
  /** Run chart query every Nth tick (default: 4, i.e. every 2000ms at 500ms tick). */
  queryEveryN?: number;
}

export interface UseTimeSeriesStoreReturn {
  data: ChartDataMap | null;
  isLoading: boolean;
}

export function useTimeSeriesStore<T extends TimePoint>(
  options: UseTimeSeriesStoreOptions<T>,
): UseTimeSeriesStoreReturn {
  const {
    conn,
    schema,
    mode,
    replayPoints,
    ingestBufferRef,
    timeRange = null,
    maxPoints = DISPLAY_MAX_POINTS,
    tickInterval = 500,
    queryEveryN = 4,
  } = options;

  const [data, setData] = useState<ChartDataMap | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const queryTimerRef = useRef<number>(0);
  const hasDataRef = useRef(false);

  // Refs to avoid stale closures in realtime queryTick
  const timeRangeRef = useRef(timeRange);
  timeRangeRef.current = timeRange;
  const schemaRef = useRef(schema);
  schemaRef.current = schema;
  const maxPointsRef = useRef(maxPoints);
  maxPointsRef.current = maxPoints;

  // Replay mode: batch insert all points when data or timeRange changes
  useEffect(() => {
    if (mode !== "replay" || !conn || !replayPoints) return;

    let cancelled = false;
    (async () => {
      setIsLoading(true);
      await clearTable(conn, schema);
      await insertPoints(conn, schema, replayPoints);
      // In replay mode, always query all data. Viewport slicing
      // (based on currentTime and timeRange) is handled downstream.
      const result = await queryDerived(conn, schema, undefined, maxPoints);
      if (!cancelled) {
        setData(result);
        setIsLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conn, mode, replayPoints, schema, maxPoints]);

  // Realtime mode: drain IngestBuffer + periodic query
  useEffect(() => {
    if (mode !== "realtime" || !conn) return;

    let cancelled = false;
    let tickCount = 0;

    const startPolling = async () => {
      try {
        await clearTable(conn, schemaRef.current);
        hasDataRef.current = false;
        setData(null);
      } catch (e) {
        console.warn("useTimeSeriesStore: failed to reset table:", e);
      }

      if (cancelled) return;

      // Single sequential tick: insert then (periodically) query.
      // Using one loop avoids concurrent DuckDB access that caused
      // data loss when insertTick and queryTick overlapped.
      const tick = async () => {
        if (cancelled) return;
        try {
          // 1. Drain buffer -> DuckDB insert (lightweight)
          const newPoints = ingestBufferRef.current.drain();
          if (newPoints.length > 0) {
            await insertPoints(conn, schemaRef.current, newPoints);
            hasDataRef.current = true;
          }

          // 2. Periodically compute derived quantities for charts (heavy)
          tickCount++;
          if (hasDataRef.current && tickCount % queryEveryN === 0) {
            const tMin = computeTMin(
              timeRangeRef.current,
              ingestBufferRef.current.latestT,
            );
            const result = await queryDerived(
              conn,
              schemaRef.current,
              tMin,
              maxPointsRef.current,
            );
            if (!cancelled) setData(result);
          }
        } catch (e) {
          console.warn("useTimeSeriesStore tick error:", e);
        }
        if (!cancelled) {
          queryTimerRef.current = window.setTimeout(
            tick,
            tickInterval,
          ) as unknown as number;
        }
      };

      queryTimerRef.current = window.setTimeout(
        tick,
        tickInterval,
      ) as unknown as number;
    };

    startPolling();

    return () => {
      cancelled = true;
      clearTimeout(queryTimerRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conn, mode]);

  return { data, isLoading };
}
