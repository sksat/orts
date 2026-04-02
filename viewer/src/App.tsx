import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Scene } from "./components/Scene.js";
import { initKaname } from "./wasm/kanameInit.js";

// Start loading kaname WASM module immediately.
const kanameReady = initKaname();

import {
  ChartBuffer,
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
import { usePlayback } from "./hooks/usePlayback.js";
import { useRealtimePlayback } from "./hooks/useRealtimePlayback.js";
import {
  type QueryRangeResponse,
  type SatelliteInfo,
  type SimInfo,
  useWebSocket,
} from "./hooks/useWebSocket.js";
import { type CSVMetadata, type OrbitPoint, parseOrbitCSVWithMetadata } from "./orbit.js";
import { DEFAULT_FRAME, type ReferenceFrame } from "./referenceFrame.js";
import { mergeQueryRangePoints } from "./utils/mergeQueryRange.js";
import { TrailBuffer } from "./utils/TrailBuffer.js";
import { computeReplayDrawStart } from "./utils/trailDrawStart.js";
import { readTimeRangeParam, writeTimeRangeParam } from "./utils/urlParams.js";
import { jd_to_utc_string } from "./wasm/kanameInit.js";

/** The two viewer modes. */
type ViewerMode = "replay" | "realtime";

const DEFAULT_WS_URL: string =
  import.meta.env.VITE_WS_URL ??
  `${window.location.protocol === "https:" ? "wss:" : "ws:"}//${window.location.host}/ws`;

/** Stable reference for an empty terminated-satellites set.
 *  Avoids creating a new Set object on each handleConnect call,
 *  which would cascade through useRealtimePlayback's dependency chain. */
const EMPTY_TERMINATED_SET: Set<string> = new Set();

/** Chart color palette matching the 3D scene SATELLITE_COLORS. */
const SATELLITE_CHART_COLORS = ["#00ff88", "#ff4488", "#44aaff", "#ffaa44", "#aa44ff"];

import { METRIC_NAMES } from "./chartMetrics.js";

/** Chart column names matching the derived column names in orbitSchema. */
const CHART_COLUMNS = [
  "t",
  "altitude",
  "energy",
  "angular_momentum",
  "velocity",
  "a",
  "e",
  "inc_deg",
  "raan_deg",
  "accel_gravity",
  "accel_drag",
  "accel_srp",
  "accel_third_body_sun",
  "accel_third_body_moon",
  "accel_perturbation_total",
];

const RAD_TO_DEG = 180.0 / Math.PI;

/** Convert an OrbitPoint (with server-computed derived values) to a chart row. */
function orbitPointToChartRow(p: OrbitPoint): Record<string, number> {
  const accelDrag = p.accel_drag ?? 0;
  const accelSrp = p.accel_srp ?? 0;
  const accelSun = p.accel_third_body_sun ?? 0;
  const accelMoon = p.accel_third_body_moon ?? 0;
  return {
    t: p.t,
    altitude: p.altitude ?? 0,
    energy: p.specific_energy ?? 0,
    angular_momentum: p.angular_momentum ?? 0,
    velocity: p.velocity_mag ?? 0,
    a: p.a,
    e: p.e,
    inc_deg: p.inc * RAD_TO_DEG,
    raan_deg: p.raan * RAD_TO_DEG,
    accel_gravity: p.accel_gravity ?? 0,
    accel_drag: accelDrag,
    accel_srp: accelSrp,
    accel_third_body_sun: accelSun,
    accel_third_body_moon: accelMoon,
    accel_perturbation_total: accelDrag + accelSrp + accelSun + accelMoon,
  };
}

/** Helper: get or create a TrailBuffer in a Map. */
function getOrCreateTrailBuffer(map: Map<string, TrailBuffer>, id: string): TrailBuffer {
  let buf = map.get(id);
  if (!buf) {
    buf = new TrailBuffer(50000);
    map.set(id, buf);
  }
  return buf;
}

/** Helper: get or create an IngestBuffer in a Map. */
function getOrCreateIngestBuffer(
  map: Map<string, IngestBuffer<OrbitPoint>>,
  id: string,
): IngestBuffer<OrbitPoint> {
  let buf = map.get(id);
  if (!buf) {
    buf = new IngestBuffer<OrbitPoint>();
    map.set(id, buf);
  }
  return buf;
}

export function App() {
  // --- WASM initialization (must complete before rendering ECEF transforms) ---
  const [wasmReady, setWasmReady] = useState(false);
  useEffect(() => {
    kanameReady.then(() => setWasmReady(true));
  }, []);

  // --- Mode toggle ---
  const [mode, setMode] = useState<ViewerMode>("realtime");

  // --- Reference frame ---
  const [referenceFrame, setReferenceFrame] = useState<ReferenceFrame>(DEFAULT_FRAME);

  const _isSatCentered = referenceFrame.center.type === "satellite";

  // --- Replay mode state ---
  const [replayPoints, setReplayPoints] = useState<OrbitPoint[] | null>(null);
  const [csvMetadata, setCsvMetadata] = useState<CSVMetadata | null>(null);
  const [orbitInfo, setOrbitInfo] = useState<string>("");
  const fileInputRef = useRef<HTMLInputElement>(null);
  const { snapshot, togglePlayPause, setSpeed, seekToFraction } = usePlayback(replayPoints);

  // --- Chart time range ---
  const [timeRange, setTimeRange] = useState<TimeRange>(() => readTimeRangeParam());

  // Sync timeRange to URL query parameter
  useEffect(() => {
    writeTimeRangeParam(timeRange);
  }, [timeRange]);

  // --- Realtime mode state ---
  const [wsUrl, setWsUrl] = useState(DEFAULT_WS_URL);
  const [simInfo, setSimInfo] = useState<SimInfo | null>(null);
  const [terminatedSatellites, setTerminatedSatellites] = useState<Set<string>>(new Set());

  type ServerState = "unknown" | "idle" | "running" | "paused";
  const [serverState, setServerState] = useState<ServerState>("unknown");

  // --- DuckDB + Charts ---
  const mu = mode === "realtime" ? simInfo?.mu : (csvMetadata?.mu ?? undefined);
  const bodyRadius =
    mode === "realtime"
      ? simInfo?.central_body_radius
      : (csvMetadata?.centralBodyRadius ?? undefined);
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

  const detailBufferRef = useRef<OrbitPoint[]>([]);
  const streamingCountRef = useRef(0);

  // --- Per-satellite buffers ---
  const trailBuffersRef = useRef(new Map<string, TrailBuffer>());
  const ingestBuffersRef = useRef(new Map<string, IngestBuffer<OrbitPoint>>());

  // --- Single-satellite IngestBuffer ref (for replay / single-sat mode) ---
  const singleIngestBufferRef = useRef(new IngestBuffer<OrbitPoint>());

  // --- ChartBuffer for live DuckDB bypass ---
  // Holds the most recent 50k points for instant chart rendering without DuckDB.
  // At dt=10s this covers ~139 hours. When the user views a range exceeding the
  // buffer (e.g. "All" on a very long run), the DuckDB path takes over via
  // the time-range / zoom fallback logic below.
  const chartBufferRef = useRef(new ChartBuffer(CHART_COLUMNS, 50000));
  const [chartBufferVersion, setChartBufferVersion] = useState(0);
  const chartDirtyRef = useRef(false);
  // Result of local DuckDB zoom query (replaces server query_range).
  const [localZoomData, setLocalZoomData] = useState<ChartDataMap | null>(null);
  // Bumped when server notifies that new high-res textures are available.
  const [textureRevision, setTextureRevision] = useState(0);

  // --- Multi-satellite detection ---
  const isMultiSatellite = mode === "realtime" && simInfo != null && simInfo.satellites.length > 1;

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
      singleIngestBufferRef.current = getOrCreateIngestBuffer(
        ingestBuffersRef.current,
        simInfo.satellites[0].id,
      );
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

  const handleState = useCallback((point: OrbitPoint) => {
    const id = point.entityPath ?? "default";
    getOrCreateIngestBuffer(ingestBuffersRef.current, id).push(point);
    getOrCreateTrailBuffer(trailBuffersRef.current, id).push(point);
    chartBufferRef.current.push(orbitPointToChartRow(point));
    streamingCountRef.current++;
    // Batch version bumps to at most once per animation frame to avoid
    // excessive React re-renders at high message rates.
    if (!chartDirtyRef.current) {
      chartDirtyRef.current = true;
      requestAnimationFrame(() => {
        chartDirtyRef.current = false;
        setChartBufferVersion((v) => v + 1);
      });
    }
  }, []);

  const handleInfo = useCallback((info: SimInfo) => {
    setSimInfo(info);
    setServerState("running");
  }, []);

  const handleStatus = useCallback((state: string) => {
    if (state === "idle") {
      setServerState("idle");
      setSimInfo(null);
    } else if (state === "paused") {
      setServerState("paused");
    } else if (state === "running") {
      setServerState("running");
    }
  }, []);

  const handleError = useCallback((message: string) => {
    console.error("Server error:", message);
  }, []);

  const handleSimulationTerminated = useCallback(
    (entityPath: string, t: number, reason: string) => {
      console.log(`Satellite ${entityPath} terminated at t=${t.toFixed(2)}s: ${reason}`);
      setTerminatedSatellites((prev) => {
        const next = new Set(prev);
        next.add(entityPath);
        return next;
      });
    },
    [],
  );

  const handleHistory = useCallback((points: OrbitPoint[]) => {
    // Group by satellite, then markRebuild so DuckDB tables are fully
    // replaced.  This clears stale data left over from a prior connection.
    const byId = new Map<string, OrbitPoint[]>();
    // Seed ChartBuffer with history data (clear first for fresh session)
    chartBufferRef.current.clear();
    for (const point of points) {
      const id = point.entityPath ?? "default";
      let arr = byId.get(id);
      if (!arr) {
        arr = [];
        byId.set(id, arr);
      }
      arr.push(point);
      getOrCreateTrailBuffer(trailBuffersRef.current, id).push(point);
      chartBufferRef.current.push(orbitPointToChartRow(point));
    }
    for (const [id, pts] of byId) {
      getOrCreateIngestBuffer(ingestBuffersRef.current, id).markRebuild(pts);
    }
    // Dev-only: expose history arrival diagnostic for E2E tests
    if (import.meta.env.DEV) {
      const byIdCounts: Record<string, number> = {};
      for (const [id, pts] of byId) byIdCounts[id] = pts.length;
      (window as unknown as Record<string, unknown>).__debug_last_history = {
        historyLen: points.length,
        byIdCounts,
      };
    }
    streamingCountRef.current = 0;
    // Notify live chart that history data is available
    setChartBufferVersion((v) => v + 1);
  }, []);

  const handleHistoryDetail = useCallback((points: OrbitPoint[]) => {
    for (const point of points) {
      detailBufferRef.current.push(point);
    }
  }, []);

  const handleHistoryDetailComplete = useCallback(() => {
    if (detailBufferRef.current.length === 0) return;

    const detailPoints = detailBufferRef.current;
    detailBufferRef.current = [];

    const streamingPoints: OrbitPoint[] = [];
    for (const buf of trailBuffersRef.current.values()) {
      const allPts = buf.getAll();
      const safeCount = Math.min(streamingCountRef.current, allPts.length);
      streamingPoints.push(...allPts.slice(allPts.length - safeCount));
    }

    const combined = [...detailPoints, ...streamingPoints];
    combined.sort((a, b) => a.t - b.t);

    const bySatellite = new Map<string, OrbitPoint[]>();
    for (const p of combined) {
      const id = p.entityPath ?? "default";
      let arr = bySatellite.get(id);
      if (!arr) {
        arr = [];
        bySatellite.set(id, arr);
      }
      arr.push(p);
    }

    for (const [id, pts] of bySatellite) {
      getOrCreateTrailBuffer(trailBuffersRef.current, id).clear();
      getOrCreateTrailBuffer(trailBuffersRef.current, id).pushMany(pts);
      getOrCreateIngestBuffer(ingestBuffersRef.current, id).markRebuild(pts);
    }

    // Sync ChartBuffer with the full-resolution data
    chartBufferRef.current.clear();
    for (const point of combined) {
      chartBufferRef.current.push(orbitPointToChartRow(point));
    }
    setChartBufferVersion((v) => v + 1);
  }, []);

  const handleQueryRangeResponse = useCallback(
    (response: QueryRangeResponse) => {
      // Discard stale responses: if a newer query_range was requested, ignore
      // responses that don't match the latest requested range.
      const latest = latestRequestedRangeRef.current;
      if (latest && (response.tMin !== latest.tMin || response.tMax !== latest.tMax)) {
        return;
      }

      const satId = simInfo?.satellites[0]?.id ?? "default";
      const allTrailPoints = getOrCreateTrailBuffer(trailBuffersRef.current, satId).getAll();
      const combined = mergeQueryRangePoints(response.points, allTrailPoints);

      getOrCreateIngestBuffer(ingestBuffersRef.current, satId).markRebuild(combined);
      getOrCreateTrailBuffer(trailBuffersRef.current, satId).clear();
      getOrCreateTrailBuffer(trailBuffersRef.current, satId).pushMany(combined);
    },
    [simInfo],
  );

  const handleTexturesReady = useCallback((_body: string) => {
    setTextureRevision((v) => v + 1);
  }, []);

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
            setChartBufferVersion((v) => v + 1);
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
          setReplayPoints(null);
          setCsvMetadata(null);
          return;
        }

        setReplayPoints(parsed);
        setCsvMetadata(metadata);

        // Feed IngestBuffer so DuckDB charts work without the old replay path
        singleIngestBufferRef.current = new IngestBuffer<OrbitPoint>();
        singleIngestBufferRef.current.markRebuild(parsed);

        if (mode === "realtime" && isConnected) {
          disconnect();
        }
        setMode("replay");

        const duration = parsed[parsed.length - 1].t - parsed[0].t;
        setOrbitInfo(
          `Loaded: ${file.name} | ${parsed.length} points | Duration: ${duration.toFixed(1)} s`,
        );
      };
      reader.readAsText(file);
    },
    [mode, isConnected, disconnect],
  );

  const handleLoadClick = useCallback(() => {
    fileInputRef.current?.click();
  }, []);

  const handleFileChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;
      loadCSVFile(file);
      e.target.value = "";
    },
    [loadCSVFile],
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
      if (file) loadCSVFile(file);
    },
    [loadCSVFile],
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
    detailBufferRef.current = [];
    streamingCountRef.current = 0;
    for (const buf of trailBuffersRef.current.values()) buf.clear();
    trailBuffersRef.current.clear();
    ingestBuffersRef.current.clear();
    singleIngestBufferRef.current = new IngestBuffer<OrbitPoint>();
    chartBufferRef.current.clear();
    setChartBufferVersion((v) => v + 1);
    lastSentRangeRef.current = null;
    latestRequestedRangeRef.current = null;
    setLocalZoomData(null);
    if (chartZoomTimerRef.current != null) {
      clearTimeout(chartZoomTimerRef.current);
      chartZoomTimerRef.current = null;
    }
    setSimInfo(null);
    setServerState("unknown");
    setTerminatedSatellites(EMPTY_TERMINATED_SET);
    goLiveRef.current();
    connect();
  }, [connect]);

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
    if (mode === "realtime" && !isConnected && !manualDisconnectRef.current && !noAutoConnect) {
      handleConnectRef.current();
    }
  }, [mode, isConnected, noAutoConnect]);

  // --- Mode switching ---
  const handleModeChange = useCallback(
    (newMode: ViewerMode) => {
      if (newMode === mode) return;
      if (mode === "realtime" && isConnected) disconnect();
      // Reset manual disconnect flag so auto-connect works when switching back to realtime.
      if (newMode === "realtime") manualDisconnectRef.current = false;
      setLocalZoomData(null);
      lastSentRangeRef.current = null;
      setMode(newMode);
    },
    [mode, isConnected, disconnect],
  );

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
    if (mode === "replay") {
      if (!replayPoints || replayPoints.length === 0) return undefined;
      return quantizeChartTime(replayPoints[0].t + snapshot.elapsedTime);
    }
    if (realtimePlayback.snapshot.isLive) return undefined;
    return quantizeChartTime(realtimePlayback.snapshot.currentTime);
  }, [
    mode,
    replayPoints,
    snapshot.elapsedTime,
    realtimePlayback.snapshot.isLive,
    realtimePlayback.snapshot.currentTime,
  ]);

  // --- Live chart data: bypass DuckDB, read directly from ChartBuffer ---
  const isLive = mode === "realtime" && realtimePlayback.snapshot.isLive;
  const liveChartData = useMemo((): ChartDataMap | null => {
    if (!isLive || isMultiSatellite) return null;
    // chartBufferVersion triggers re-read from the buffer
    void chartBufferVersion;
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
  }, [isLive, isMultiSatellite, chartBufferVersion, timeRange]);

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
  const satellitePosition =
    mode === "replay" ? snapshot.satellitePosition : realtimePlayback.snapshot.satellitePosition;

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

  const centralBody =
    mode === "realtime"
      ? (simInfo?.central_body ?? "earth")
      : (csvMetadata?.centralBody ?? "earth");
  const centralBodyRadius =
    mode === "realtime"
      ? (simInfo?.central_body_radius ?? 6378.137)
      : (csvMetadata?.centralBodyRadius ?? 6378.137);

  const epochJd =
    mode === "realtime" ? (simInfo?.epoch_jd ?? undefined) : (csvMetadata?.epochJd ?? undefined);

  const trailVisibleCount =
    mode === "replay"
      ? snapshot.trailVisibleCount
      : realtimePlayback.snapshot.isLive
        ? undefined
        : realtimePlayback.snapshot.trailVisibleCount;

  // Draw start for replay mode time-range clipping
  const replayTrailDrawStart = useMemo(() => {
    if (mode !== "replay" || !replayPoints || replayPoints.length === 0) return 0;
    const currentT = replayPoints[0].t + snapshot.elapsedTime;
    return computeReplayDrawStart(replayPoints, currentT, timeRange);
  }, [mode, timeRange, replayPoints, snapshot.elapsedTime]);

  // Total points across all satellite buffers
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const totalPoints = useMemo(() => {
    let count = 0;
    for (const buf of trailBuffersRef.current.values()) count += buf.length;
    return count;
  }, [realtimePlayback.snapshot.currentTime]);

  const showPlaybackBar =
    mode === "realtime" ? totalPoints > 0 : replayPoints != null && replayPoints.length > 0;

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
        points={mode === "replay" ? replayPoints : undefined}
        satellitePosition={mode === "replay" ? satellitePosition : undefined}
        trailVisibleCount={mode === "replay" ? trailVisibleCount : undefined}
        trailDrawStart={mode === "replay" ? replayTrailDrawStart : undefined}
        trailBuffers={mode === "realtime" ? trailBuffersRef.current : undefined}
        satellitePositions={
          mode === "realtime" ? realtimePlayback.snapshot.satellitePositions : undefined
        }
        trailVisibleCounts={
          mode === "realtime" && !realtimePlayback.snapshot.isLive
            ? realtimePlayback.snapshot.trailVisibleCounts
            : undefined
        }
        trailDrawStarts={
          mode === "realtime" && timeRange != null
            ? realtimePlayback.snapshot.trailDrawStarts
            : undefined
        }
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
        <div className="mode-toggle">
          <button
            className={`mode-toggle-btn ${mode === "replay" ? "active" : ""}`}
            onClick={() => handleModeChange("replay")}
          >
            Replay
          </button>
          <button
            className={`mode-toggle-btn ${mode === "realtime" ? "active" : ""}`}
            onClick={() => handleModeChange("realtime")}
          >
            Realtime
          </button>
        </div>

        <FrameSelector
          referenceFrame={referenceFrame}
          onChange={setReferenceFrame}
          satellites={simInfo?.satellites}
          hasEpoch={epochJd != null}
          centralBody={centralBody}
        />

        {mode === "replay" && (
          <>
            <button className="load-csv-btn" onClick={handleLoadClick}>
              Load Orbit CSV
            </button>
            {orbitInfo && <div className="orbit-info">{orbitInfo}</div>}
          </>
        )}

        {mode === "realtime" && (
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
        )}
      </div>

      <input
        ref={fileInputRef}
        type="file"
        accept=".csv,.txt"
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

      {showPlaybackBar &&
        (mode === "realtime" ? (
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
        ) : (
          <PlaybackBar
            isPlaying={snapshot.isPlaying}
            fraction={snapshot.fraction}
            elapsedTime={snapshot.elapsedTime}
            totalDuration={snapshot.totalDuration}
            onTogglePlayPause={togglePlayPause}
            onSeekFraction={seekToFraction}
            onSpeedChange={setSpeed}
            epochJd={epochJd}
          />
        ))}
    </div>
  );
}
