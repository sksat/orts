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

/** Time range for chart display: null = all history, number = last N seconds. */
export type TimeRange = number | null;

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

  // Replay mode: batch insert all points when CSV changes
  useEffect(() => {
    if (mode !== "replay" || !conn || !replayPoints) return;

    let cancelled = false;
    (async () => {
      setIsLoading(true);
      await clearTable(conn);
      await insertPoints(conn, replayPoints);
      const data = await queryDerivedQuantities(conn, mu, bodyRadius);
      if (!cancelled) {
        setChartData(data);
        setIsLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [conn, mode, replayPoints, mu, bodyRadius]);

  // Realtime mode: drain IngestBuffer + periodic query
  useEffect(() => {
    if (mode !== "realtime" || !conn) return;

    let cancelled = false;
    const QUERY_INTERVAL = 500;

    const startPolling = async () => {
      try {
        await clearTable(conn);
        hasDataRef.current = false;
        setChartData(null);
      } catch (e) {
        console.warn("useOrbitCharts: failed to reset table:", e);
      }

      if (cancelled) return;

      const tick = async () => {
        if (cancelled) return;
        try {
          const newPoints = ingestBufferRef.current.drain();

          if (newPoints.length > 0) {
            await insertPoints(conn, newPoints);
            hasDataRef.current = true;
          }

          if (hasDataRef.current) {
            const tMin = timeRange != null
              ? ingestBufferRef.current.latestT - timeRange
              : undefined;
            const data = await queryDerivedQuantities(conn, mu, bodyRadius, tMin);
            if (!cancelled) setChartData(data);
          }
        } catch (e) {
          console.warn("useOrbitCharts tick error:", e);
        }

        if (!cancelled) {
          queryTimerRef.current = window.setTimeout(tick, QUERY_INTERVAL);
        }
      };

      queryTimerRef.current = window.setTimeout(tick, QUERY_INTERVAL);
    };

    startPolling();

    return () => {
      cancelled = true;
      clearTimeout(queryTimerRef.current);
    };
  }, [conn, mode, mu, bodyRadius]);

  return { chartData, isLoading };
}
