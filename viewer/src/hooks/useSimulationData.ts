import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ChartDataWorkerClient, IngestBuffer as IngestBufferType } from "@sksat/uneri";
import {
  type ChartBuffer,
  type ChartDataMap,
  IngestBuffer,
  quantizeChartTime,
  sliceArrays,
  type TimeRange,
  useTimeSeriesStoreWorker,
} from "@sksat/uneri";
import type {
  MultiChartDataResult,
  MultiChartDataWorkerClient,
} from "@sksat/uneri/multiWorkerClient";
import { METRIC_NAMES } from "../chartMetrics.js";
import { createOrbitSchema } from "../db/orbitSchema.js";
import type { OrbitPoint } from "../orbit.js";
import type { MultiChartDataMap } from "./buildMultiChartData.js";
import type { SatelliteConfig } from "./useMultiSatelliteStore.js";
import { useMultiSatelliteStoreWorker } from "./useMultiSatelliteStoreWorker.js";
import type { SatelliteInfo, SimInfo } from "./useWebSocket.js";

/** Chart color palette matching the 3D scene SATELLITE_COLORS. */
const SATELLITE_CHART_COLORS = ["#00ff88", "#ff4488", "#44aaff", "#ffaa44", "#aa44ff"];

export interface UseSimulationDataOptions {
  simInfo: SimInfo | null;
  ingestBuffers: Map<string, IngestBufferType<OrbitPoint>>;
  chartBuffer: ChartBuffer;
  chartBufferVersion: number;
  playback: {
    isLive: boolean;
    currentTime: number;
  };
  timeRange: TimeRange;
  /** Fallback for DuckDB query failure — sends query_range to server */
  queryRange: (satId: string, tMin: number, tMax: number, maxPoints: number) => void;
}

export interface SimulationDataResult {
  dbReady: boolean;
  visibleChartData: ChartDataMap | null;
  multiChartData: MultiChartDataMap | null;
  chartsLoading: boolean;
  isMultiSatellite: boolean;
  satelliteConfigs: SatelliteConfig[];
  handleChartZoom: (tMin: number, tMax: number) => void;
  /** Clear zoom/query state (call on source switch to avoid stale data). */
  resetZoomState: () => void;
  /** Expose the latestRequestedRangeRef for WS staleness check */
  latestRequestedRangeRef: React.RefObject<{ tMin: number; tMax: number } | null>;
}

export function useSimulationData(options: UseSimulationDataOptions): SimulationDataResult {
  const {
    simInfo,
    ingestBuffers,
    chartBuffer,
    chartBufferVersion,
    playback,
    timeRange,
    queryRange,
  } = options;

  // --- Orbit schema (shared by single & multi-satellite Workers) ---
  const mu = simInfo?.mu;
  const bodyRadius = simInfo?.central_body_radius;
  const orbitSchema = useMemo(
    () => createOrbitSchema(mu ?? 398600.4418, bodyRadius ?? 6378.137),
    [mu, bodyRadius],
  );

  // DuckDB is fully managed inside Workers (no main-thread instance).
  const dbReady = true;

  // Expose debug state for E2E testing (dev mode only)
  const isMultiSatellite = simInfo != null && simInfo.satellites.length > 1;
  useEffect(() => {
    if (import.meta.env.DEV) {
      (window as unknown as Record<string, unknown>).__debug_ingest_buffers = ingestBuffers;
      (window as unknown as Record<string, unknown>).__debug_is_multi_satellite = isMultiSatellite;
    }
  }, [isMultiSatellite, ingestBuffers]);

  // --- Single-satellite IngestBuffer ref and Worker client ref ---
  const singleIngestBufferRef = useRef(new IngestBuffer<OrbitPoint>());
  const workerClientRef = useRef<ChartDataWorkerClient | null>(null);
  // Multi-sat worker client ref. Populated by `useMultiSatelliteStoreWorker`
  // once the dynamic import finishes. Declared here (before
  // `handleChartZoom`) so the zoom handler's closure can read it.
  const multiWorkerClientRef = useRef<MultiChartDataWorkerClient | null>(null);

  // Keep singleIngestBufferRef pointing to the first satellite's buffer for single-sat mode
  useEffect(() => {
    if (simInfo?.satellites.length === 1) {
      const buf = ingestBuffers.get(simInfo.satellites[0].id);
      if (buf) singleIngestBufferRef.current = buf as IngestBuffer<OrbitPoint>;
    }
  }, [simInfo, ingestBuffers]);

  // --- Zoom state ---
  const [localZoomData, setLocalZoomData] = useState<ChartDataMap | null>(null);
  const [localMultiZoomData, setLocalMultiZoomData] = useState<MultiChartDataMap | null>(null);
  const [localChartBump, setLocalChartBump] = useState(0);
  const effectiveChartVersion = chartBufferVersion + localChartBump;

  const lastSentRangeRef = useRef<{ tMin: number; tMax: number } | null>(null);
  const latestRequestedRangeRef = useRef<{ tMin: number; tMax: number } | null>(null);
  const chartZoomTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Cleanup timer on unmount
  useEffect(() => {
    return () => {
      if (chartZoomTimerRef.current != null) {
        clearTimeout(chartZoomTimerRef.current);
      }
    };
  }, []);

  // --- Multi-satellite configs ---
  const satelliteConfigs = useMemo((): SatelliteConfig[] => {
    if (!simInfo) return [];
    return simInfo.satellites.map((sat: SatelliteInfo, i: number) => ({
      id: sat.id,
      label: sat.name ?? sat.id,
      color: SATELLITE_CHART_COLORS[i % SATELLITE_CHART_COLORS.length],
    }));
  }, [simInfo]);

  // --- Chart zoom handler ---
  const isLive = playback.isLive;
  const isLiveRef = useRef(isLive);
  isLiveRef.current = isLive;

  const handleChartZoom = useCallback(
    (tMin: number, tMax: number) => {
      // Dedupe: skip if same range as last sent request
      const last = lastSentRangeRef.current;
      if (last && last.tMin === tMin && last.tMax === tMax) return;

      // Coalesce: always update the latest desired range
      latestRequestedRangeRef.current = { tMin, tMax };

      // Trailing debounce: only send after 200ms of quiet
      if (chartZoomTimerRef.current != null) {
        clearTimeout(chartZoomTimerRef.current);
      }
      chartZoomTimerRef.current = setTimeout(() => {
        chartZoomTimerRef.current = null;
        const range = latestRequestedRangeRef.current;
        if (!range) return;
        // Always record the zoom range (used by liveChartData for getWindow).
        lastSentRangeRef.current = range;

        /** Server fallback: fire one `query_range` per satellite so every
         * sat's trail/chart buffers get enriched for the window. Mirrors
         * the M3 proactive-initial-query pattern. */
        const serverFallback = () => {
          if (!simInfo) return;
          for (const sat of simInfo.satellites) {
            queryRange(sat.id, range.tMin, range.tMax, 2000);
          }
        };

        if (isMultiSatellite) {
          // Multi-sat: ask the multi-sat worker for an aligned zoom
          // window across every satellite's DuckDB. If the worker has
          // the data, the result renders immediately; otherwise fall
          // back to pulling detail from the server.
          const multiClient = multiWorkerClientRef.current;
          if (multiClient) {
            multiClient
              .zoomQuery(range.tMin, range.tMax, 2000)
              .then((data: MultiChartDataResult) => {
                const current = latestRequestedRangeRef.current;
                if (!current || current.tMin !== range.tMin || current.tMax !== range.tMax) {
                  return;
                }
                // Accept the result if any metric has data; otherwise
                // fall back to a server pull per sat.
                const hasData = Object.values(data).some(
                  (series) => series != null && series.t.length > 0,
                );
                if (hasData) {
                  setLocalMultiZoomData(data);
                } else {
                  setLocalMultiZoomData(null);
                  serverFallback();
                }
              })
              .catch((e: unknown) => {
                console.warn("Multi-sat zoom query failed, falling back to server:", e);
                serverFallback();
              });
          } else {
            serverFallback();
          }
          return;
        }

        // Single-sat path: live ChartBuffer fast path → worker DuckDB
        // zoom query → server query_range fallback.
        if (isLiveRef.current) {
          if (
            chartBuffer.length > 0 &&
            range.tMin >= chartBuffer.earliestT &&
            range.tMax <= chartBuffer.latestT
          ) {
            setLocalChartBump((v) => v + 1);
            return;
          }
        }

        const client = workerClientRef.current;
        if (client) {
          client
            .zoomQuery(range.tMin, range.tMax, 2000)
            .then((data) => {
              const current = latestRequestedRangeRef.current;
              if (current && current.tMin === range.tMin && current.tMax === range.tMax) {
                setLocalZoomData(data.t.length > 0 ? data : null);
              }
            })
            .catch((e) => {
              console.warn("Worker zoom query failed, falling back to server:", e);
              const satId = simInfo?.satellites[0]?.id ?? "default";
              queryRange(satId, range.tMin, range.tMax, 2000);
            });
        } else {
          // No Worker client — fall back to server query_range.
          const satId = simInfo?.satellites[0]?.id ?? "default";
          queryRange(satId, range.tMin, range.tMax, 2000);
        }
      }, 200);
    },
    // multiWorkerClientRef / workerClientRef are stable `useRef` objects,
    // read via `.current` at call time.
    [isMultiSatellite, simInfo, queryRange, chartBuffer],
  );

  // --- Charts: single-satellite mode (Worker-based) ---
  // DuckDB tick loop runs entirely in a Web Worker, keeping the main thread free.
  // Disabled in multi-satellite mode (uses useMultiSatelliteStoreWorker instead).
  const { data: singleChartData, isLoading: singleChartsLoading } = useTimeSeriesStoreWorker({
    schema: orbitSchema,
    ingestBufferRef: singleIngestBufferRef,
    timeRange,
    enabled: !isMultiSatellite,
    clientRef: workerClientRef,
  });

  // --- Charts: multi-satellite mode (Worker-based) ---
  const { data: multiChartDataRaw, isLoading: multiChartsLoading } = useMultiSatelliteStoreWorker({
    baseSchema: orbitSchema,
    satelliteConfigs,
    ingestBuffers,
    metricNames: METRIC_NAMES,
    timeRange,
    enabled: isMultiSatellite,
    clientRef: multiWorkerClientRef,
  });

  // When the user zooms, the one-shot multi-zoom-query result takes
  // precedence over the tick-broadcast data. Clearing falls back to the
  // normal timeRange view.
  const multiChartData: MultiChartDataMap | null = localMultiZoomData ?? multiChartDataRaw;

  // Expose the latest deserialized multi-sat chart data for E2E tests
  // (dev mode only). This is the post-`alignTimeSeries` output that the
  // charts actually render, so tests can assert properties like
  // NaN counts, per-series length consistency, and timestamp span
  // without reaching into the Worker's DuckDB directly.
  useEffect(() => {
    if (import.meta.env.DEV) {
      (window as unknown as Record<string, unknown>).__debug_multi_chart_data = multiChartData;
    }
  }, [multiChartData]);

  const chartsLoading = isMultiSatellite ? multiChartsLoading : singleChartsLoading;

  // --- Chart current time ---
  const chartCurrentTime = useMemo(() => {
    if (isLive) return undefined;
    return quantizeChartTime(playback.currentTime);
  }, [isLive, playback.currentTime]);

  // --- Live chart data: bypass DuckDB, read directly from ChartBuffer ---
  const liveChartData = useMemo((): ChartDataMap | null => {
    if (!isLive || isMultiSatellite) return null;
    // effectiveChartVersion triggers re-read from the buffer
    void effectiveChartVersion;
    if (chartBuffer.length === 0) return null;

    // If user has zoomed, check if ChartBuffer covers the range.
    // If yes, serve from buffer. If no, return null to fall through to DuckDB.
    const zoomRange = lastSentRangeRef.current;
    if (zoomRange) {
      if (zoomRange.tMin >= chartBuffer.earliestT && zoomRange.tMax <= chartBuffer.latestT) {
        return chartBuffer.getWindow(zoomRange.tMin, zoomRange.tMax);
      }
      return null; // Fall through to DuckDB for out-of-range zoom
    }

    if (timeRange != null) {
      const tMax = chartBuffer.latestT;
      const tMin = tMax - timeRange;
      return chartBuffer.getWindow(tMin, tMax);
    }
    return chartBuffer.toChartData();
  }, [isLive, isMultiSatellite, effectiveChartVersion, timeRange, chartBuffer]);

  // --- DuckDB chart data: used for replay, zoom outside ChartBuffer, and non-live scrubbing ---
  const chartArrays = useMemo(() => {
    if (isMultiSatellite || !singleChartData) return null;
    return [
      singleChartData.t,
      singleChartData.altitude,
      singleChartData.energy,
      singleChartData.angular_momentum,
      singleChartData.velocity,
      singleChartData.a,
      singleChartData.e,
      singleChartData.inc_deg,
      singleChartData.raan_deg,
    ];
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isMultiSatellite, singleChartData, isLive]);

  const visibleArrays = useMemo(
    () => sliceArrays(chartArrays, chartCurrentTime, timeRange),
    [chartArrays, chartCurrentTime, timeRange],
  );

  const duckdbChartData = useMemo((): ChartDataMap | null => {
    if (!visibleArrays) return null;
    return {
      t: visibleArrays[0],
      altitude: visibleArrays[1],
      energy: visibleArrays[2],
      angular_momentum: visibleArrays[3],
      velocity: visibleArrays[4],
      a: visibleArrays[5],
      e: visibleArrays[6],
      inc_deg: visibleArrays[7],
      raan_deg: visibleArrays[8],
    };
  }, [visibleArrays]);

  // --- Zoom reset: clear when returning to live or when time range changes ---
  const prevIsLiveRef = useRef(isLive);
  const prevTimeRangeRef = useRef(timeRange);
  useEffect(() => {
    if ((isLive && !prevIsLiveRef.current) || timeRange !== prevTimeRangeRef.current) {
      lastSentRangeRef.current = null;
      setLocalZoomData(null);
      setLocalMultiZoomData(null);
    }
    prevIsLiveRef.current = isLive;
    prevTimeRangeRef.current = timeRange;
  }, [isLive, timeRange]);

  // Choose data source:
  // 1. Live + no zoom → ChartBuffer (instant)
  // 2. Live + zoom covered by ChartBuffer → ChartBuffer.getWindow (instant)
  // 3. Zoom outside ChartBuffer → local DuckDB query result
  // 4. Non-live / fallback → DuckDB useTimeSeriesStore
  const visibleChartData = liveChartData ?? localZoomData ?? duckdbChartData;

  const resetZoomState = useCallback(() => {
    lastSentRangeRef.current = null;
    latestRequestedRangeRef.current = null;
    setLocalZoomData(null);
    setLocalMultiZoomData(null);
    setLocalChartBump((v) => v + 1);
    if (chartZoomTimerRef.current != null) {
      clearTimeout(chartZoomTimerRef.current);
      chartZoomTimerRef.current = null;
    }
    // Reset single-satellite ingest buffer to avoid stale chart data after reconnect
    singleIngestBufferRef.current = new IngestBuffer<OrbitPoint>();
  }, []);

  return {
    dbReady,
    visibleChartData,
    multiChartData,
    chartsLoading,
    isMultiSatellite,
    satelliteConfigs,
    handleChartZoom,
    resetZoomState,
    latestRequestedRangeRef,
  };
}
