import { useState, useEffect, useRef } from "react";
import type { AsyncDuckDBConnection } from "@duckdb/duckdb-wasm";
import { OrbitPoint } from "../orbit.js";
import { IngestBuffer } from "../db/IngestBuffer.js";
import {
  insertPoints,
  clearTable,
  queryDerivedQuantities,
  ChartData,
} from "../db/orbitStore.js";

const MU_EARTH = 398600.4418;

/** Maximum number of points to display in charts. Query-time downsampling
 *  keeps chart rendering fast regardless of total data in DuckDB. */
export const DISPLAY_MAX_POINTS = 2000;

/** Time range for chart display: null = all history, number = last N seconds. */
export type TimeRange = number | null;

/** Compute the minimum t value for a chart query given a time range window. */
export function computeTMin(
  timeRange: TimeRange,
  latestT: number
): number | undefined {
  if (timeRange == null) return undefined;
  return latestT - timeRange;
}

interface UseOrbitChartsOptions {
  conn: AsyncDuckDBConnection | null;
  mode: "replay" | "realtime";
  replayPoints: OrbitPoint[] | null;
  ingestBufferRef: React.RefObject<IngestBuffer>;
  mu?: number;
  bodyRadius?: number;
  /** Show only last N seconds of data, or null for all history. */
  timeRange?: TimeRange;
}

export interface UseOrbitChartsReturn {
  chartData: ChartData | null;
  isLoading: boolean;
}

export function useOrbitCharts(
  options: UseOrbitChartsOptions
): UseOrbitChartsReturn {
  const {
    conn,
    mode,
    replayPoints,
    ingestBufferRef,
    mu = MU_EARTH,
    bodyRadius = 6378.137,
    timeRange = null,
  } = options;
  const [chartData, setChartData] = useState<ChartData | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const queryTimerRef = useRef<number>(0);
  const hasDataRef = useRef(false);
  // Refs to avoid stale closures in realtime queryTick
  const timeRangeRef = useRef(timeRange);
  timeRangeRef.current = timeRange;
  const muRef = useRef(mu);
  muRef.current = mu;
  const bodyRadiusRef = useRef(bodyRadius);
  bodyRadiusRef.current = bodyRadius;

  // Replay mode: batch insert all points when data or timeRange changes
  useEffect(() => {
    if (mode !== "replay" || !conn || !replayPoints) return;

    let cancelled = false;
    (async () => {
      setIsLoading(true);
      await clearTable(conn);
      await insertPoints(conn, replayPoints);
      // In replay mode, always query all data. Viewport slicing
      // (based on currentTime and timeRange) is handled downstream.
      const data = await queryDerivedQuantities(conn, mu, bodyRadius, undefined, DISPLAY_MAX_POINTS);
      if (!cancelled) {
        setChartData(data);
        setIsLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conn, mode, replayPoints, mu, bodyRadius]);

  // Realtime mode: drain IngestBuffer + periodic query
  useEffect(() => {
    if (mode !== "realtime" || !conn) return;

    let cancelled = false;
    const TICK_INTERVAL = 500;
    const QUERY_EVERY_N = 4; // run chart query every 4th tick (2000ms)
    let tickCount = 0;

    const startPolling = async () => {
      try {
        await clearTable(conn);
        hasDataRef.current = false;
        setChartData(null);
      } catch (e) {
        console.warn("useOrbitCharts: failed to reset table:", e);
      }

      if (cancelled) return;

      // Single sequential tick: insert then (periodically) query.
      // Using one loop avoids concurrent DuckDB access that caused
      // data loss when insertTick and queryTick overlapped.
      const tick = async () => {
        if (cancelled) return;
        try {
          // 1. Drain buffer → DuckDB insert (lightweight)
          const newPoints = ingestBufferRef.current.drain();
          if (newPoints.length > 0) {
            await insertPoints(conn, newPoints);
            hasDataRef.current = true;
          }

          // 2. Periodically compute derived quantities for charts (heavy)
          tickCount++;
          if (hasDataRef.current && tickCount % QUERY_EVERY_N === 0) {
            const tMin = computeTMin(
              timeRangeRef.current,
              ingestBufferRef.current.latestT
            );
            const data = await queryDerivedQuantities(
              conn, muRef.current, bodyRadiusRef.current, tMin, DISPLAY_MAX_POINTS
            );
            if (!cancelled) setChartData(data);
          }
        } catch (e) {
          console.warn("useOrbitCharts tick error:", e);
        }
        if (!cancelled) {
          queryTimerRef.current = window.setTimeout(tick, TICK_INTERVAL);
        }
      };

      queryTimerRef.current = window.setTimeout(tick, TICK_INTERVAL);
    };

    startPolling();

    return () => {
      cancelled = true;
      clearTimeout(queryTimerRef.current);
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conn, mode]);

  return { chartData, isLoading };
}
