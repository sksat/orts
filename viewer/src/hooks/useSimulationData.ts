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
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { METRIC_NAMES, TORQUE_CHART_METRICS } from "../chartMetrics.js";
import { createOrbitSchema } from "../db/orbitSchema.js";
import { duckdbBundles } from "../duckdbBundles.js";
import type { OrbitPoint } from "../orbit.js";
import type { SourceKind } from "../sources/types.js";
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
  /** The active source's kind: a file's DuckDB tables are never compacted. */
  sourceKind: SourceKind | null;
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

/** The chart columns served from DuckDB, in the order `chartArrays` holds them. */
const DUCKDB_CHART_COLUMNS = [
  "altitude",
  "energy",
  "angular_momentum",
  "velocity",
  "a",
  "e",
  "inc_deg",
  "raan_deg",
  ...TORQUE_CHART_METRICS,
];

export function useSimulationData(options: UseSimulationDataOptions): SimulationDataResult {
  const {
    simInfo,
    ingestBuffers,
    chartBuffer,
    chartBufferVersion,
    playback,
    timeRange,
    sourceKind,
    queryRange,
  } = options;
  // A file keeps every row it loads (DESIGN.md, the file source policy).
  const compaction = sourceKind !== "file";

  // Orbit schema (shared by single & multi-satellite Workers). A `SimInfo` has
  // been resolved against a body already, so its `mu` and radius are taken as
  // they are. The schema's own Earth defaults stand in only while no source has
  // reported one, where there is no body yet to be wrong about.
  const mu = simInfo?.mu;
  const bodyRadius = simInfo?.central_body_radius;
  const orbitSchema = useMemo(
    () =>
      mu == null || bodyRadius == null ? createOrbitSchema() : createOrbitSchema(mu, bodyRadius),
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

  // Single-satellite IngestBuffer ref and Worker client ref
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

  // Zoom state
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

  // Multi-satellite configs
  const satelliteConfigs = useMemo((): SatelliteConfig[] => {
    if (!simInfo) return [];
    return simInfo.satellites.map((sat: SatelliteInfo, i: number) => ({
      id: sat.id,
      label: sat.name ?? sat.id,
      color: SATELLITE_CHART_COLORS[i % SATELLITE_CHART_COLORS.length],
    }));
  }, [simInfo]);

  // Chart zoom handler
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

  // Charts: single-satellite mode (Worker-based)
  // DuckDB tick loop runs entirely in a Web Worker, keeping the main thread free.
  // Disabled in multi-satellite mode (uses useMultiSatelliteStoreWorker instead).
  const { data: singleChartData, isLoading: singleChartsLoading } = useTimeSeriesStoreWorker({
    schema: orbitSchema,
    ingestBufferRef: singleIngestBufferRef,
    timeRange,
    enabled: !isMultiSatellite,
    clientRef: workerClientRef,
    duckDB: { bundles: duckdbBundles },
    compaction,
  });

  // Charts: multi-satellite mode (Worker-based)
  const { data: multiChartDataRaw, isLoading: multiChartsLoading } = useMultiSatelliteStoreWorker({
    baseSchema: orbitSchema,
    satelliteConfigs,
    ingestBuffers,
    metricNames: METRIC_NAMES,
    timeRange,
    enabled: isMultiSatellite,
    clientRef: multiWorkerClientRef,
    duckDB: { bundles: duckdbBundles },
    compaction,
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

  // Chart current time
  const chartCurrentTime = useMemo(() => {
    if (isLive) return undefined;
    return quantizeChartTime(playback.currentTime);
  }, [isLive, playback.currentTime]);

  // Live chart data: bypass DuckDB, read directly from ChartBuffer
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

  // DuckDB chart data: used for replay, zoom outside ChartBuffer, and non-live scrubbing.
  // `chartArrays` and `duckdbChartData` are a positional pair, so both read the
  // same column list. A column the query did not return is left out of both.
  // TODO: the acceleration columns are still absent here, so those charts have
  //   no data outside the live path either — a gap that predates the torques.
  const duckdbColumns = useMemo(
    () =>
      singleChartData ? DUCKDB_CHART_COLUMNS.filter((name) => singleChartData[name] != null) : [],
    [singleChartData],
  );

  const chartArrays = useMemo(() => {
    if (isMultiSatellite || !singleChartData) return null;
    return [singleChartData.t, ...duckdbColumns.map((name) => singleChartData[name])];
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isMultiSatellite, singleChartData, duckdbColumns, isLive]);

  const visibleArrays = useMemo(
    () => sliceArrays(chartArrays, chartCurrentTime, timeRange),
    [chartArrays, chartCurrentTime, timeRange],
  );

  const duckdbChartData = useMemo((): ChartDataMap | null => {
    if (!visibleArrays) return null;
    const data: ChartDataMap = { t: visibleArrays[0] };
    duckdbColumns.forEach((name, i) => {
      const values = visibleArrays[i + 1];
      if (values) data[name] = values;
    });
    return data;
  }, [visibleArrays, duckdbColumns]);

  // Zoom reset: clear when returning to live or when time range changes
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

  // The same for one satellite: what the charts render, after the live buffer
  // or DuckDB has answered. A test can read a column's values here, where the
  // chart component draws its title and legend whether or not any value
  // arrived.
  useEffect(() => {
    if (import.meta.env.DEV) {
      (window as unknown as Record<string, unknown>).__debug_chart_data = visibleChartData;
    }
  }, [visibleChartData]);

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
