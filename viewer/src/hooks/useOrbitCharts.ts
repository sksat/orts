import { useState, useEffect, useRef } from "react";
import type { AsyncDuckDBConnection } from "@duckdb/duckdb-wasm";
import { OrbitPoint } from "../orbit.js";
import {
  insertPoints,
  clearTable,
  queryDerivedQuantities,
  ChartData,
} from "../db/orbitStore.js";

const MU_EARTH = 398600.4418;

interface UseOrbitChartsOptions {
  conn: AsyncDuckDBConnection | null;
  mode: "replay" | "realtime";
  replayPoints: OrbitPoint[] | null;
  realtimePointsRef: React.RefObject<OrbitPoint[]>;
  realtimeVersion: number;
  mu?: number;
  bodyRadius?: number;
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
    realtimePointsRef,
    realtimeVersion,
    mu = MU_EARTH,
    bodyRadius = 6378.137,
  } = options;
  const [chartData, setChartData] = useState<ChartData | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const insertedCountRef = useRef(0);
  const queryTimerRef = useRef<number>(0);

  // Replay mode: batch insert all points when CSV changes
  useEffect(() => {
    if (mode !== "replay" || !conn || !replayPoints) return;

    let cancelled = false;
    (async () => {
      setIsLoading(true);
      await clearTable(conn);
      insertedCountRef.current = 0;
      await insertPoints(conn, replayPoints);
      insertedCountRef.current = replayPoints.length;
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

  // Realtime mode: incremental insert + periodic query
  useEffect(() => {
    if (mode !== "realtime" || !conn) return;

    // Reset on mode switch
    (async () => {
      await clearTable(conn);
      insertedCountRef.current = 0;
      setChartData(null);
    })();

    const QUERY_INTERVAL = 500;

    const tick = async () => {
      try {
        const allPoints = realtimePointsRef.current!;
        const newCount = allPoints.length;
        const inserted = insertedCountRef.current;

        if (newCount > inserted) {
          const newPoints = allPoints.slice(inserted);
          await insertPoints(conn, newPoints);
          insertedCountRef.current = newCount;
          const data = await queryDerivedQuantities(conn, mu, bodyRadius);
          setChartData(data);
        }
      } catch (e) {
        console.warn("useOrbitCharts tick error:", e);
      }

      queryTimerRef.current = window.setTimeout(tick, QUERY_INTERVAL);
    };

    queryTimerRef.current = window.setTimeout(tick, QUERY_INTERVAL);

    return () => {
      clearTimeout(queryTimerRef.current);
    };
    // realtimeVersion is intentionally omitted — we poll on a timer instead
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conn, mode, mu, bodyRadius]);

  return { chartData, isLoading };
}
