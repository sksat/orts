import { useState, useEffect, useRef } from "react";
import type { AsyncDuckDBConnection } from "@duckdb/duckdb-wasm";
import { OrbitPoint } from "../orbit.js";
import { IngestBuffer } from "../db/IngestBuffer.js";
import {
  insertPoints,
  clearTable,
  replaceRange,
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
    const INSERT_INTERVAL = 500;  // drain buffer → DuckDB (lightweight)
    const QUERY_INTERVAL = 2000;  // derived quantity query (heavy)

    const startPolling = async () => {
      try {
        await clearTable(conn);
        hasDataRef.current = false;
        setChartData(null);
      } catch (e) {
        console.warn("useOrbitCharts: failed to reset table:", e);
      }

      if (cancelled) return;

      // Lightweight insert tick: drain buffer into DuckDB
      const insertTick = async () => {
        if (cancelled) return;
        try {
          const buf = ingestBufferRef.current;
          const range = buf.replaceRange;
          if (range) {
            await replaceRange(conn, range.tMin, range.tMax, []);
            buf.replaceRange = null;
          }
          const newPoints = buf.drain();
          if (newPoints.length > 0) {
            await insertPoints(conn, newPoints);
            hasDataRef.current = true;
          }
        } catch (e) {
          console.warn("useOrbitCharts insert error:", e);
        }
        if (!cancelled) {
          window.setTimeout(insertTick, INSERT_INTERVAL);
        }
      };

      // Heavy query tick: compute derived quantities for charts
      const queryTick = async () => {
        if (cancelled) return;
        try {
          if (hasDataRef.current) {
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
          console.warn("useOrbitCharts query error:", e);
        }
        if (!cancelled) {
          queryTimerRef.current = window.setTimeout(queryTick, QUERY_INTERVAL);
        }
      };

      window.setTimeout(insertTick, INSERT_INTERVAL);
      queryTimerRef.current = window.setTimeout(queryTick, QUERY_INTERVAL);
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
