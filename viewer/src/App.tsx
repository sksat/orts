import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Scene } from "./components/Scene.js";
import { initKaname } from "./wasm/kanameInit.js";

// Start loading kaname WASM module immediately.
const kanameReady = initKaname();

import {
  type ChartDataMap,
  IngestBuffer,
  quantizeChartTime,
  queryDerived,
  sliceArrays,
  type TimeRange,
  useDuckDB,
  useTimeSeriesStore,
} from "uneri";
import { FrameSelector } from "./components/FrameSelector.js";
import { GraphPanel } from "./components/GraphPanel.js";
import { PlaybackBar } from "./components/PlaybackBar.js";
import { SimConfigForm, type SimConfigPayload } from "./components/SimConfigForm.js";
import { SimControlBar } from "./components/SimControlBar.js";
import { createOrbitSchema } from "./db/orbitSchema.js";
import { type SatelliteConfig, useMultiSatelliteStore } from "./hooks/useMultiSatelliteStore.js";
import { useRealtimePlayback } from "./hooks/useRealtimePlayback.js";
import {
  type QueryRangeResponse,
  type SatelliteInfo,
  type SimInfo,
  useWebSocket,
} from "./hooks/useWebSocket.js";
import { type OrbitPoint, parseOrbitCSVWithMetadata } from "./orbit.js";
import { DEFAULT_FRAME, type ReferenceFrame } from "./referenceFrame.js";
import { RrdFileAdapter } from "./sources/RrdFileAdapter.js";
import { useSourceRuntime } from "./sources/useSourceRuntime.js";
import { mergeQueryRangePoints } from "./utils/mergeQueryRange.js";
import { readTimeRangeParam, writeTimeRangeParam } from "./utils/urlParams.js";
import { jd_to_utc_string } from "./wasm/kanameInit.js";

const DEFAULT_WS_URL: string =
  import.meta.env.VITE_WS_URL ??
  `${window.location.protocol === "https:" ? "wss:" : "ws:"}//${window.location.host}/ws`;

/** Stable reference for an empty terminated-satellites set.
 *  Avoids creating a new Set object on each handleConnect call,
 *  which would cascade through useRealtimePlayback's dependency chain. */

/** Chart color palette matching the 3D scene SATELLITE_COLORS. */
const SATELLITE_CHART_COLORS = ["#00ff88", "#ff4488", "#44aaff", "#ffaa44", "#aa44ff"];

import { METRIC_NAMES } from "./chartMetrics.js";

// Chart helpers and buffer factories moved to sources/eventDispatcher.ts

export function App() {
  // --- WASM initialization (must complete before rendering ECEF transforms) ---
  const [wasmReady, setWasmReady] = useState(false);
  useEffect(() => {
    kanameReady.then(() => setWasmReady(true));
  }, []);

  // --- Reference frame ---
  const [referenceFrame, setReferenceFrame] = useState<ReferenceFrame>(DEFAULT_FRAME);

  const _isSatCentered = referenceFrame.center.type === "satellite";

  // --- File load state (CSV / RRD) ---
  const [orbitInfo, setOrbitInfo] = useState<string>("");
  const fileInputRef = useRef<HTMLInputElement>(null);
  /** Tracks whether a local file is the active source (vs WS). */
  const [fileSourceActive, setFileSourceActive] = useState(false);

  // --- Chart time range ---
  const [timeRange, setTimeRange] = useState<TimeRange>(() => readTimeRangeParam());

  // Sync timeRange to URL query parameter
  useEffect(() => {
    writeTimeRangeParam(timeRange);
  }, [timeRange]);

  // --- Source Runtime (manages buffers, state, event dispatch) ---
  const runtime = useSourceRuntime();
  const {
    trailBuffers: trailBuffersMap,
    ingestBuffers: ingestBuffersMap,
    chartBuffer: runtimeChartBuffer,
    simInfo,
    serverState,
    terminatedSatellites,
    textureRevision,
    chartBufferVersion,
    handleEvent,
    setActiveSourceId,
    resetBuffers,
  } = runtime;

  // --- Refs for backward compat (some code still uses .current pattern) ---
  const trailBuffersRef = useRef(trailBuffersMap);
  trailBuffersRef.current = trailBuffersMap;
  const ingestBuffersRef = useRef(ingestBuffersMap);
  ingestBuffersRef.current = ingestBuffersMap;
  const chartBufferRef = useRef(runtimeChartBuffer);
  chartBufferRef.current = runtimeChartBuffer;

  // --- WS URL + connection state ---
  const [wsUrl, setWsUrl] = useState(DEFAULT_WS_URL);

  // --- DuckDB + Charts ---
  const mu = simInfo?.mu;
  const bodyRadius = simInfo?.central_body_radius;
  const orbitSchema = useMemo(
    () => createOrbitSchema(mu ?? 398600.4418, bodyRadius ?? 6378.137),
    [mu, bodyRadius],
  );
  const { conn, isReady: dbReady } = useDuckDB(orbitSchema);

  // Expose DuckDB connection and debug state for E2E testing (dev mode only)
  useEffect(() => {
    if (import.meta.env.DEV && conn) {
      (window as unknown as Record<string, unknown>).__duckdb_conn = conn;
    }
  }, [conn]);

  // --- Single-satellite IngestBuffer ref (for single-sat DuckDB mode) ---
  const singleIngestBufferRef = useRef(new IngestBuffer<OrbitPoint>());

  // Result of local DuckDB zoom query (replaces server query_range).
  const [localZoomData, setLocalZoomData] = useState<ChartDataMap | null>(null);
  // Local chart version bump for non-event-driven updates (e.g., zoom within ChartBuffer)
  const [localChartBump, setLocalChartBump] = useState(0);
  const effectiveChartVersion = chartBufferVersion + localChartBump;

  // --- Multi-satellite detection ---
  const isMultiSatellite = simInfo != null && simInfo.satellites.length > 1;

  // Expose debug state for E2E testing (dev mode only)
  useEffect(() => {
    if (import.meta.env.DEV) {
      (window as unknown as Record<string, unknown>).__debug_ingest_buffers =
        ingestBuffersRef.current;
      (window as unknown as Record<string, unknown>).__debug_is_multi_satellite = isMultiSatellite;
    }
  }, [isMultiSatellite]);

  // Keep singleIngestBufferRef pointing to the first satellite's buffer for single-sat mode
  useEffect(() => {
    if (simInfo?.satellites.length === 1) {
      const buf = ingestBuffersRef.current.get(simInfo.satellites[0].id);
      if (buf) singleIngestBufferRef.current = buf as IngestBuffer<OrbitPoint>;
    }
  }, [simInfo]);

  // --- Satellite configs for multi-store ---
  const satelliteConfigs = useMemo((): SatelliteConfig[] => {
    if (!simInfo) return [];
    return simInfo.satellites.map((sat: SatelliteInfo, i: number) => ({
      id: sat.id,
      label: sat.name ?? sat.id,
      color: SATELLITE_CHART_COLORS[i % SATELLITE_CHART_COLORS.length],
    }));
  }, [simInfo]);

  // --- Satellite name map for 3D model lookup ---
  const satelliteNames = useMemo(() => {
    if (!simInfo) return undefined;
    const m = new Map<string, string | null>();
    for (const sat of simInfo.satellites) m.set(sat.id, sat.name);
    return m;
  }, [simInfo]);

  // --- Realtime playback (history scrubbing) ---
  const realtimePlayback = useRealtimePlayback(
    trailBuffersRef.current,
    terminatedSatellites,
    timeRange,
  );

  // --- WS → SourceEvent bridge ---
  // useWebSocket callbacks are bridged to useSourceRuntime.handleEvent.
  // This keeps useWebSocket as the WS lifecycle manager while routing
  // all data through the unified SourceEvent pipeline.
  const WS_SOURCE_ID = "ws-0";

  const handleState = useCallback(
    (point: OrbitPoint) => handleEvent(WS_SOURCE_ID, { kind: "state", point }),
    [handleEvent],
  );
  const handleInfo = useCallback(
    (info: SimInfo) => handleEvent(WS_SOURCE_ID, { kind: "info", info }),
    [handleEvent],
  );
  const handleStatus = useCallback(
    (state: string) => handleEvent(WS_SOURCE_ID, { kind: "server-state", state }),
    [handleEvent],
  );
  const handleError = useCallback(
    (message: string) => handleEvent(WS_SOURCE_ID, { kind: "error", message }),
    [handleEvent],
  );
  const handleSimulationTerminated = useCallback(
    (entityPath: string, t: number, reason: string) =>
      handleEvent(WS_SOURCE_ID, { kind: "terminated", entityPath, t, reason }),
    [handleEvent],
  );
  const handleHistory = useCallback(
    (points: OrbitPoint[]) => {
      handleEvent(WS_SOURCE_ID, { kind: "history", points });
      // Dev-only: expose history arrival diagnostic for E2E tests
      if (import.meta.env.DEV) {
        const byId = new Map<string, number>();
        for (const p of points) {
          const id = p.entityPath ?? "default";
          byId.set(id, (byId.get(id) ?? 0) + 1);
        }
        (window as unknown as Record<string, unknown>).__debug_last_history = {
          historyLen: points.length,
          byIdCounts: Object.fromEntries(byId),
        };
      }
    },
    [handleEvent],
  );
  const handleHistoryDetail = useCallback(
    (points: OrbitPoint[]) => handleEvent(WS_SOURCE_ID, { kind: "history-detail", points }),
    [handleEvent],
  );
  const handleHistoryDetailComplete = useCallback(
    () => handleEvent(WS_SOURCE_ID, { kind: "history-detail-complete" }),
    [handleEvent],
  );
  const handleQueryRangeResponse = useCallback(
    (response: QueryRangeResponse) => {
      // Discard stale responses
      const latest = latestRequestedRangeRef.current;
      if (latest && (response.tMin !== latest.tMin || response.tMax !== latest.tMax)) {
        return;
      }
      // Merge with existing streaming data to avoid position rewind
      const satId = simInfo?.satellites[0]?.id ?? "default";
      const trailBuf = trailBuffersRef.current.get(satId);
      const merged = trailBuf
        ? mergeQueryRangePoints(response.points, trailBuf.getAll())
        : response.points;
      handleEvent(WS_SOURCE_ID, {
        kind: "range-response",
        tMin: response.tMin,
        tMax: response.tMax,
        points: merged,
      });
    },
    [handleEvent, simInfo],
  );
  const handleTexturesReady = useCallback(
    (body: string) => handleEvent(WS_SOURCE_ID, { kind: "textures-ready", body }),
    [handleEvent],
  );

  const { connect, disconnect, isConnected, send } = useWebSocket({
    url: wsUrl,
    onState: handleState,
    onInfo: handleInfo,
    onHistory: handleHistory,
    onHistoryDetail: handleHistoryDetail,
    onHistoryDetailComplete: handleHistoryDetailComplete,
    onQueryRangeResponse: handleQueryRangeResponse,
    onSimulationTerminated: handleSimulationTerminated,
    onStatus: handleStatus,
    onError: handleError,
    onTexturesReady: handleTexturesReady,
  });

  const handleStartSimulation = useCallback(
    (config: SimConfigPayload) => {
      send({ type: "start_simulation", config });
    },
    [send],
  );

  const handlePause = useCallback(() => {
    send({ type: "pause_simulation" });
  }, [send]);

  const handleResume = useCallback(() => {
    send({ type: "resume_simulation" });
  }, [send]);

  const handleTerminate = useCallback(() => {
    send({ type: "terminate_simulation" });
  }, [send]);

  // --- query_range dedupe + coalescing ---
  // Tracks the last sent range to suppress duplicate requests (e.g. from multiple charts
  // sharing the same onZoom callback, or from programmatic setScale firings).
  const lastSentRangeRef = useRef<{ tMin: number; tMax: number } | null>(null);
  // Tracks the latest requested range for in-flight coalescing.
  // If a query_range_response arrives with a different range, it is stale and discarded.
  const latestRequestedRangeRef = useRef<{ tMin: number; tMax: number } | null>(null);
  const chartZoomTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const handleChartZoom = useCallback(
    (tMin: number, tMax: number) => {
      if (isMultiSatellite) return;

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

        // In live mode, try ChartBuffer first (instant, no server round-trip).
        // Note: if the zoomed interval later ages out of the ring buffer,
        // liveChartData returns null and falls back to DuckDB. The user
        // would need to re-zoom to trigger a fresh query_range in that case.
        // This is acceptable since the buffer holds ~139h at dt=10s.
        if (realtimePlayback.snapshot.isLive) {
          const buf = chartBufferRef.current;
          if (buf.length > 0 && range.tMin >= buf.earliestT && range.tMax <= buf.latestT) {
            setLocalChartBump((v) => v + 1);
            return;
          }
        }

        // Query local DuckDB instead of server query_range.
        // Note: DuckDB may lag behind by up to one ingest tick (~250ms)
        // since data is flushed asynchronously. This is acceptable for
        // zoom operations where sub-second freshness isn't critical.
        if (conn) {
          queryDerived(conn, orbitSchema, range.tMin, 2000, range.tMax)
            .then((data) => {
              // Check if this result is still relevant (not superseded by a newer zoom)
              const current = latestRequestedRangeRef.current;
              if (current && current.tMin === range.tMin && current.tMax === range.tMax) {
                setLocalZoomData(data);
              }
            })
            .catch((e) => {
              console.warn("Local DuckDB zoom query failed, falling back to server:", e);
              const satId = simInfo?.satellites[0]?.id ?? "default";
              send({
                type: "query_range",
                t_min: range.tMin,
                t_max: range.tMax,
                max_points: 2000,
                entity_path: satId,
              });
            });
        } else {
          // No DuckDB connection — fall back to server query_range.
          const satId = simInfo?.satellites[0]?.id ?? "default";
          send({
            type: "query_range",
            t_min: range.tMin,
            t_max: range.tMax,
            max_points: 2000,
            entity_path: satId,
          });
        }
      }, 200);
    },
    [conn, orbitSchema, send, isMultiSatellite, simInfo, realtimePlayback.snapshot.isLive],
  );

  // --- Replay: file loading ---
  const loadCSVFile = useCallback(
    (file: File) => {
      const reader = new FileReader();
      reader.onload = () => {
        const text = reader.result as string;
        const { points: parsed, metadata } = parseOrbitCSVWithMetadata(text);

        if (parsed.length === 0) {
          setOrbitInfo("No valid orbit data found in file.");
          return;
        }

        // Disconnect WS if connected
        if (isConnected) {
          disconnect();
        }
        // Stop any active RRD worker
        if (rrdAdapterRef.current) {
          rrdAdapterRef.current.stop();
          rrdAdapterRef.current = null;
        }

        // Route CSV data through useSourceRuntime via SourceEvents
        const CSV_SOURCE_ID = "csv-file";
        resetBuffers();
        setActiveSourceId(CSV_SOURCE_ID);

        // Build SimInfo from CSV metadata
        // For multi-sat, estimate dt from consecutive points of the same entity
        let dt = 10;
        if (metadata.satellites && metadata.satellites.length > 0) {
          // Multi-sat: find dt from same-entity consecutive points
          for (let i = 1; i < parsed.length; i++) {
            if (parsed[i].entityPath === parsed[0].entityPath && parsed[i].t > parsed[0].t) {
              dt = parsed[i].t - parsed[0].t;
              break;
            }
          }
        } else if (parsed.length >= 2) {
          dt = parsed[1].t - parsed[0].t;
        }

        // Build satellites list
        const satellites =
          metadata.satellites && metadata.satellites.length > 0
            ? metadata.satellites.map((id) => ({
                id,
                name: id,
                altitude: 0,
                period: 0,
                perturbations: [] as string[],
              }))
            : [
                {
                  id: "default",
                  name: metadata.satelliteName ?? `${file.name} (1 sat)`,
                  altitude: 0,
                  period: 0,
                  perturbations: [] as string[],
                },
              ];

        handleEvent(CSV_SOURCE_ID, {
          kind: "info",
          info: {
            mu: metadata.mu ?? 398600.4418,
            dt,
            output_interval: dt,
            stream_interval: dt,
            central_body: metadata.centralBody ?? "earth",
            central_body_radius: metadata.centralBodyRadius ?? 6378.137,
            epoch_jd: metadata.epochJd,
            satellites,
          },
        });

        // Push all CSV data as a history event, then mark complete.
        // NOTE: Do NOT dispatch server-state "idle" here — the dispatcher
        // clears simInfo on idle, which would erase the CSV metadata we just set.
        handleEvent(CSV_SOURCE_ID, { kind: "history", points: parsed });
        handleEvent(CSV_SOURCE_ID, { kind: "complete" });

        setFileSourceActive(true);
        // Reset playback to start of file data
        goLiveRef.current();

        const duration = parsed[parsed.length - 1].t - parsed[0].t;
        setOrbitInfo(
          `Loaded: ${file.name} | ${parsed.length} points | Duration: ${duration.toFixed(1)} s`,
        );
      };
      reader.readAsText(file);
    },
    [isConnected, disconnect, handleEvent, resetBuffers, setActiveSourceId],
  );

  const rrdAdapterRef = useRef<RrdFileAdapter | null>(null);
  const loadRrdFile = useCallback(
    (file: File) => {
      if (isConnected) disconnect();
      // Stop any previous RRD adapter to prevent stale worker events
      if (rrdAdapterRef.current) {
        rrdAdapterRef.current.stop();
        rrdAdapterRef.current = null;
      }
      resetBuffers();

      const RRD_SOURCE_ID = "rrd-file";
      setActiveSourceId(RRD_SOURCE_ID);

      let totalPoints = 0;
      const rrdHandleEvent: typeof handleEvent = (sourceId, event) => {
        handleEvent(sourceId, event);
        if (event.kind === "history-chunk") {
          totalPoints += event.points.length;
        }
        if (event.kind === "complete") {
          setOrbitInfo(`Loaded: ${file.name} | ${totalPoints} points`);
        }
      };

      const adapter = new RrdFileAdapter(RRD_SOURCE_ID, file, rrdHandleEvent);
      rrdAdapterRef.current = adapter;
      adapter.start();
      setFileSourceActive(true);
      goLiveRef.current();
      setOrbitInfo(`Loading: ${file.name}...`);
    },
    [isConnected, disconnect, handleEvent, resetBuffers, setActiveSourceId],
  );

  /** Route file to appropriate loader based on extension. */
  const loadFile = useCallback(
    (file: File) => {
      if (file.name.endsWith(".rrd")) {
        loadRrdFile(file);
      } else {
        loadCSVFile(file);
      }
    },
    [loadCSVFile, loadRrdFile],
  );

  const handleLoadClick = useCallback(() => {
    fileInputRef.current?.click();
  }, []);

  const handleFileChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;
      loadFile(file);
      e.target.value = "";
    },
    [loadFile],
  );

  // --- Drag & Drop ---
  const [isDragOver, setIsDragOver] = useState(false);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(true);
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(false);
  }, []);

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setIsDragOver(false);
      const file = e.dataTransfer.files[0];
      if (file) loadFile(file);
    },
    [loadFile],
  );

  // --- Realtime: connect / disconnect ---
  // Use ref for goLive to avoid including it in handleConnect deps.
  // This breaks the circular dependency: handleConnect → goLive → syncState → terminatedSatellites.
  const goLiveRef = useRef(realtimePlayback.goLive);
  goLiveRef.current = realtimePlayback.goLive;

  // Track explicit user disconnect to suppress auto-reconnect.
  const manualDisconnectRef = useRef(false);

  const handleConnect = useCallback(() => {
    manualDisconnectRef.current = false;
    setFileSourceActive(false);
    // Stop any active RRD worker
    if (rrdAdapterRef.current) {
      rrdAdapterRef.current.stop();
      rrdAdapterRef.current = null;
    }
    resetBuffers();
    setActiveSourceId(WS_SOURCE_ID);
    singleIngestBufferRef.current = new IngestBuffer<OrbitPoint>();
    setLocalChartBump((v) => v + 1); // Invalidate cached chart data
    lastSentRangeRef.current = null;
    latestRequestedRangeRef.current = null;
    setLocalZoomData(null);
    if (chartZoomTimerRef.current != null) {
      clearTimeout(chartZoomTimerRef.current);
      chartZoomTimerRef.current = null;
    }
    goLiveRef.current();
    connect();
  }, [connect, resetBuffers, setActiveSourceId]);

  const handleDisconnect = useCallback(() => {
    manualDisconnectRef.current = true;
    disconnect();
  }, [disconnect]);

  // --- Auto-connect ---
  // Use ref for handleConnect to make the effect immune to callback identity changes.
  // The effect only re-fires on actual mode/connection changes.
  // Suppressed when ?noAutoConnect=1 is in the URL (used by E2E tests with mock servers).
  const handleConnectRef = useRef(handleConnect);
  handleConnectRef.current = handleConnect;
  const noAutoConnect = new URLSearchParams(window.location.search).has("noAutoConnect");

  useEffect(() => {
    if (!fileSourceActive && !isConnected && !manualDisconnectRef.current && !noAutoConnect) {
      handleConnectRef.current();
    }
  }, [fileSourceActive, isConnected, noAutoConnect]);

  // --- Charts: single-satellite mode (replay or single satellite) ---
  const { data: singleChartData, isLoading: singleChartsLoading } = useTimeSeriesStore({
    conn: isMultiSatellite ? null : conn, // disable when multi-sat (uses multi-store instead)
    schema: orbitSchema,
    ingestBufferRef: singleIngestBufferRef,
    timeRange,
  });

  // --- Charts: multi-satellite mode ---
  const { data: multiChartData, isLoading: multiChartsLoading } = useMultiSatelliteStore({
    conn: isMultiSatellite ? conn : null, // only active in multi-sat mode
    baseSchema: orbitSchema,
    satelliteConfigs,
    ingestBuffers: ingestBuffersRef.current,
    metricNames: METRIC_NAMES,
    timeRange,
  });

  const chartsLoading = isMultiSatellite ? multiChartsLoading : singleChartsLoading;

  const chartCurrentTime = useMemo(() => {
    if (realtimePlayback.snapshot.isLive) return undefined;
    return quantizeChartTime(realtimePlayback.snapshot.currentTime);
  }, [realtimePlayback.snapshot.isLive, realtimePlayback.snapshot.currentTime]);

  // --- Live chart data: bypass DuckDB, read directly from ChartBuffer ---
  const isLive = realtimePlayback.snapshot.isLive;
  const liveChartData = useMemo((): ChartDataMap | null => {
    if (!isLive || isMultiSatellite) return null;
    // effectiveChartVersion triggers re-read from the buffer
    void effectiveChartVersion;
    const buf = chartBufferRef.current;
    if (buf.length === 0) return null;

    // If user has zoomed, check if ChartBuffer covers the range.
    // If yes, serve from buffer. If no, return null to fall through to DuckDB.
    const zoomRange = lastSentRangeRef.current;
    if (zoomRange) {
      if (zoomRange.tMin >= buf.earliestT && zoomRange.tMax <= buf.latestT) {
        return buf.getWindow(zoomRange.tMin, zoomRange.tMax);
      }
      return null; // Fall through to DuckDB for out-of-range zoom
    }

    if (timeRange != null) {
      const tMax = buf.latestT;
      const tMin = tMax - timeRange;
      return buf.getWindow(tMin, tMax);
    }
    return buf.toChartData();
  }, [isLive, isMultiSatellite, effectiveChartVersion, timeRange]);

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

  // Clear zoom state when returning to live mode or when the user changes
  // the time range preset (e.g. "5 min" → "30 min"), so the chart follows
  // the rolling window instead of staying stuck on a fixed zoom range.
  const prevIsLiveRef = useRef(isLive);
  const prevTimeRangeRef = useRef(timeRange);
  if ((isLive && !prevIsLiveRef.current) || timeRange !== prevTimeRangeRef.current) {
    lastSentRangeRef.current = null;
    setLocalZoomData(null);
  }
  prevIsLiveRef.current = isLive;
  prevTimeRangeRef.current = timeRange;

  // Choose data source:
  // 1. Live + no zoom → ChartBuffer (instant)
  // 2. Live + zoom covered by ChartBuffer → ChartBuffer.getWindow (instant)
  // 3. Zoom outside ChartBuffer → local DuckDB query result
  // 4. Non-live / fallback → DuckDB useTimeSeriesStore
  const visibleChartData = liveChartData ?? localZoomData ?? duckdbChartData;

  // --- Derived values ---
  const satellitePosition = realtimePlayback.snapshot.satellitePosition;

  // Derive texture base URL from the WebSocket URL so that in dev mode
  // (Vite on a different port) high-res textures are fetched from the orts server.
  const textureBaseUrl = useMemo(() => {
    try {
      const u = new URL(wsUrl.replace(/^ws/, "http"));
      return `${u.origin}/textures/`;
    } catch {
      return `${import.meta.env.BASE_URL}textures/`;
    }
  }, [wsUrl]);

  const centralBody = simInfo?.central_body ?? "earth";
  const centralBodyRadius = simInfo?.central_body_radius ?? 6378.137;
  const epochJd = simInfo?.epoch_jd ?? undefined;

  const trailVisibleCount = realtimePlayback.snapshot.isLive
    ? undefined
    : realtimePlayback.snapshot.trailVisibleCount;

  // Total points across all satellite buffers
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const totalPoints = useMemo(() => {
    let count = 0;
    for (const buf of trailBuffersRef.current.values()) count += buf.length;
    return count;
  }, [realtimePlayback.snapshot.currentTime]);

  const showPlaybackBar = totalPoints > 0;

  // Satellite info display
  const satInfoText = useMemo(() => {
    if (!simInfo) return null;
    const parts: string[] = [];
    for (const sat of simInfo.satellites) {
      parts.push(sat.name ?? sat.id);
    }
    return parts.join(" | ");
  }, [simInfo]);

  // Union of active perturbation names across all satellites
  const activePerturbations = useMemo(() => {
    if (!simInfo) return [];
    const set = new Set<string>();
    for (const sat of simInfo.satellites) {
      for (const p of sat.perturbations) set.add(p);
    }
    return [...set];
  }, [simInfo]);

  if (!wasmReady) return null;

  return (
    <div
      className="app-root"
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      {isDragOver && (
        <div className="drop-overlay">
          <div className="drop-overlay-text">Drop CSV file to load</div>
        </div>
      )}

      {/* 3D Scene */}
      <Scene
        trailBuffers={trailBuffersRef.current}
        satellitePositions={realtimePlayback.snapshot.satellitePositions}
        trailVisibleCounts={
          !realtimePlayback.snapshot.isLive
            ? realtimePlayback.snapshot.trailVisibleCounts
            : undefined
        }
        trailDrawStarts={timeRange != null ? realtimePlayback.snapshot.trailDrawStarts : undefined}
        centralBody={centralBody}
        centralBodyRadius={centralBodyRadius}
        epochJd={epochJd ?? null}
        referenceFrame={referenceFrame}
        satelliteNames={satelliteNames}
        physicalScale={false}
        textureRevision={textureRevision}
        textureBaseUrl={textureBaseUrl}
      />

      {/* UI overlay */}
      <div className="ui-overlay">
        <FrameSelector
          referenceFrame={referenceFrame}
          onChange={setReferenceFrame}
          satellites={simInfo?.satellites}
          hasEpoch={epochJd != null}
          centralBody={centralBody}
        />

        <button className="load-csv-btn" onClick={handleLoadClick}>
          Load File
        </button>
        {orbitInfo && <div className="orbit-info">{orbitInfo}</div>}

        <div className="realtime-controls">
          <div className="ws-url-row">
            <input
              type="text"
              className="ws-url-input"
              value={wsUrl}
              onChange={(e) => setWsUrl(e.target.value)}
              placeholder="ws://localhost:9001/ws"
              disabled={isConnected}
            />
            {isConnected ? (
              <button className="ws-btn ws-disconnect-btn" onClick={handleDisconnect}>
                Disconnect
              </button>
            ) : (
              <button className="ws-btn ws-connect-btn" onClick={handleConnect}>
                Connect
              </button>
            )}
          </div>

          <div className="ws-status">
            <span className={`ws-status-dot ${isConnected ? "connected" : "disconnected"}`} />
            <span className="ws-status-text">
              {isConnected
                ? serverState === "idle"
                  ? "Connected (Idle)"
                  : serverState === "paused"
                    ? "Connected (Paused)"
                    : "Connected"
                : "Disconnected"}
            </span>
          </div>

          {isConnected && serverState === "idle" && (
            <SimConfigForm onStart={handleStartSimulation} />
          )}

          {isConnected && (serverState === "running" || serverState === "paused") && (
            <SimControlBar
              serverState={serverState}
              onPause={handlePause}
              onResume={handleResume}
              onTerminate={handleTerminate}
            />
          )}

          {simInfo && (
            <div className="orbit-info">
              {satInfoText && (
                <>
                  <strong>{satInfoText}</strong> |{" "}
                </>
              )}
              {simInfo.epoch_jd != null && <>{jd_to_utc_string(simInfo.epoch_jd, 0)} | </>}
              mu={simInfo.mu.toFixed(2)} km^3/s^2 | dt={simInfo.dt.toFixed(1)} s | stream=
              {simInfo.stream_interval.toFixed(1)} s
              {activePerturbations.length > 0 && (
                <span className="pert-tags">
                  {" | "}
                  {activePerturbations.map((p) => (
                    <span key={p} className="pert-tag">
                      {p}
                    </span>
                  ))}
                </span>
              )}
            </div>
          )}

          {totalPoints > 0 && <div className="orbit-info">{totalPoints} points</div>}
        </div>
      </div>

      <input
        ref={fileInputRef}
        type="file"
        accept=".csv,.txt,.rrd"
        style={{ display: "none" }}
        onChange={handleFileChange}
      />

      {dbReady && (
        <GraphPanel
          chartData={isMultiSatellite ? undefined : visibleChartData}
          multiChartData={isMultiSatellite ? multiChartData : undefined}
          isLoading={chartsLoading}
          timeRange={timeRange}
          onTimeRangeChange={setTimeRange}
          onZoom={handleChartZoom}
          activePerturbations={activePerturbations}
        />
      )}

      {showPlaybackBar && (
        <PlaybackBar
          isPlaying={realtimePlayback.snapshot.isPlaying}
          fraction={realtimePlayback.snapshot.fraction}
          elapsedTime={realtimePlayback.snapshot.elapsedTime}
          totalDuration={realtimePlayback.snapshot.totalDuration}
          onTogglePlayPause={realtimePlayback.togglePlayPause}
          onSeekFraction={realtimePlayback.seekToFraction}
          onSpeedChange={realtimePlayback.setSpeed}
          isLive={realtimePlayback.snapshot.isLive}
          onGoLive={realtimePlayback.goLive}
          epochJd={epochJd}
        />
      )}
    </div>
  );
}
