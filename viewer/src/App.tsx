import { useState, useCallback, useRef, useEffect, useMemo } from "react";
import { initKaname } from "./wasm/kanameInit.js";
import { Scene } from "./components/Scene.js";

// Start loading kaname WASM module immediately.
const kanameReady = initKaname();
import { PlaybackBar } from "./components/PlaybackBar.js";
import { GraphPanel } from "./components/GraphPanel.js";
import { usePlayback } from "./hooks/usePlayback.js";
import { useRealtimePlayback } from "./hooks/useRealtimePlayback.js";
import { useWebSocket, SimInfo, SatelliteInfo, QueryRangeResponse } from "./hooks/useWebSocket.js";
import {
  useDuckDB,
  useTimeSeriesStore,
  IngestBuffer,
  sliceArrays,
  quantizeChartTime,
  type TimeRange,
  type ChartDataMap,
} from "uneri";
import { createOrbitSchema } from "./db/orbitSchema.js";
import { TrailBuffer } from "./utils/TrailBuffer.js";
import { parseOrbitCSVWithMetadata, CSVMetadata, OrbitPoint } from "./orbit.js";
import { mergeQueryRangePoints } from "./utils/mergeQueryRange.js";
import { computeReplayDrawStart } from "./utils/trailDrawStart.js";
import { readTimeRangeParam, writeTimeRangeParam } from "./utils/urlParams.js";
import { jd_to_utc_string } from "./wasm/kanameInit.js";
import { useMultiSatelliteStore, type SatelliteConfig } from "./hooks/useMultiSatelliteStore.js";
import { type ReferenceFrame, DEFAULT_FRAME } from "./referenceFrame.js";
import { FrameSelector } from "./components/FrameSelector.js";

/** The two viewer modes. */
type ViewerMode = "replay" | "realtime";

const DEFAULT_WS_URL = "ws://localhost:9001";

/** Stable reference for an empty terminated-satellites set.
 *  Avoids creating a new Set object on each handleConnect call,
 *  which would cascade through useRealtimePlayback's dependency chain. */
const EMPTY_TERMINATED_SET: Set<string> = new Set();

/** Chart color palette matching the 3D scene SATELLITE_COLORS. */
const SATELLITE_CHART_COLORS = ["#00ff88", "#ff4488", "#44aaff", "#ffaa44", "#aa44ff"];

/** Derived metric names for multi-satellite alignment. */
const METRIC_NAMES = [
  "altitude", "energy", "angular_momentum", "velocity",
  "a", "e", "inc_deg", "raan_deg",
];

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
function getOrCreateIngestBuffer(map: Map<string, IngestBuffer<OrbitPoint>>, id: string): IngestBuffer<OrbitPoint> {
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
  useEffect(() => { kanameReady.then(() => setWasmReady(true)); }, []);

  // --- Mode toggle ---
  const [mode, setMode] = useState<ViewerMode>("realtime");

  // --- Reference frame ---
  const [referenceFrame, setReferenceFrame] = useState<ReferenceFrame>(DEFAULT_FRAME);

  const isSatCentered = referenceFrame.center.type === "satellite";

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

  // --- DuckDB + Charts ---
  const mu = mode === "realtime" ? simInfo?.mu : (csvMetadata?.mu ?? undefined);
  const bodyRadius = mode === "realtime" ? simInfo?.central_body_radius : (csvMetadata?.centralBodyRadius ?? undefined);
  const orbitSchema = useMemo(() => createOrbitSchema(mu ?? 398600.4418, bodyRadius ?? 6378.137), [mu, bodyRadius]);
  const { conn, isReady: dbReady } = useDuckDB(orbitSchema);

  // Expose DuckDB connection for E2E testing (dev mode only)
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

  // --- Multi-satellite detection ---
  const isMultiSatellite = mode === "realtime" && simInfo != null && simInfo.satellites.length > 1;

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
  const realtimePlayback = useRealtimePlayback(trailBuffersRef.current, terminatedSatellites, timeRange);

  const handleState = useCallback((point: OrbitPoint) => {
    const id = point.satelliteId ?? "default";
    getOrCreateIngestBuffer(ingestBuffersRef.current, id).push(point);
    getOrCreateTrailBuffer(trailBuffersRef.current, id).push(point);
    streamingCountRef.current++;
  }, []);

  const handleInfo = useCallback((info: SimInfo) => {
    setSimInfo(info);
  }, []);

  const handleSimulationTerminated = useCallback((satelliteId: string, t: number, reason: string) => {
    console.log(`Satellite ${satelliteId} terminated at t=${t.toFixed(2)}s: ${reason}`);
    setTerminatedSatellites((prev) => {
      const next = new Set(prev);
      next.add(satelliteId);
      return next;
    });
  }, []);

  const handleHistory = useCallback((points: OrbitPoint[]) => {
    for (const point of points) {
      const id = point.satelliteId ?? "default";
      getOrCreateIngestBuffer(ingestBuffersRef.current, id).push(point);
      getOrCreateTrailBuffer(trailBuffersRef.current, id).push(point);
    }
    streamingCountRef.current = 0;
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
      const id = p.satelliteId ?? "default";
      let arr = bySatellite.get(id);
      if (!arr) { arr = []; bySatellite.set(id, arr); }
      arr.push(p);
    }

    for (const [id, pts] of bySatellite) {
      getOrCreateTrailBuffer(trailBuffersRef.current, id).clear();
      getOrCreateTrailBuffer(trailBuffersRef.current, id).pushMany(pts);
      getOrCreateIngestBuffer(ingestBuffersRef.current, id).markRebuild(pts);
    }
  }, []);

  const handleQueryRangeResponse = useCallback((response: QueryRangeResponse) => {
    const satId = simInfo?.satellites[0]?.id ?? "default";
    const allTrailPoints = getOrCreateTrailBuffer(trailBuffersRef.current, satId).getAll();
    const combined = mergeQueryRangePoints(response.points, allTrailPoints);

    getOrCreateIngestBuffer(ingestBuffersRef.current, satId).markRebuild(combined);
    getOrCreateTrailBuffer(trailBuffersRef.current, satId).clear();
    getOrCreateTrailBuffer(trailBuffersRef.current, satId).pushMany(combined);
  }, [simInfo]);

  const { connect, disconnect, isConnected, send } = useWebSocket({
    url: wsUrl,
    onState: handleState,
    onInfo: handleInfo,
    onHistory: handleHistory,
    onHistoryDetail: handleHistoryDetail,
    onHistoryDetailComplete: handleHistoryDetailComplete,
    onQueryRangeResponse: handleQueryRangeResponse,
    onSimulationTerminated: handleSimulationTerminated,
  });

  const handleChartZoom = useCallback((tMin: number, tMax: number) => {
    if (!isMultiSatellite) {
      const satId = simInfo?.satellites[0]?.id ?? "default";
      send({ type: "query_range", t_min: tMin, t_max: tMax, max_points: 2000, satellite_id: satId });
    }
  }, [send, isMultiSatellite, simInfo]);

  // --- Replay: file loading ---
  const loadCSVFile = useCallback((file: File) => {
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

      if (mode === "realtime" && isConnected) {
        disconnect();
      }
      setMode("replay");

      const duration = parsed[parsed.length - 1].t - parsed[0].t;
      setOrbitInfo(
        `Loaded: ${file.name} | ${parsed.length} points | Duration: ${duration.toFixed(1)} s`
      );
    };
    reader.readAsText(file);
  }, [mode, isConnected, disconnect]);

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
    [loadCSVFile]
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

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(false);
    const file = e.dataTransfer.files[0];
    if (file) loadCSVFile(file);
  }, [loadCSVFile]);

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
    setSimInfo(null);
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
  const handleConnectRef = useRef(handleConnect);
  handleConnectRef.current = handleConnect;

  useEffect(() => {
    if (mode === "realtime" && !isConnected && !manualDisconnectRef.current) {
      handleConnectRef.current();
    }
  }, [mode, isConnected]);

  // --- Mode switching ---
  const handleModeChange = useCallback(
    (newMode: ViewerMode) => {
      if (newMode === mode) return;
      if (mode === "realtime" && isConnected) disconnect();
      // Reset manual disconnect flag so auto-connect works when switching back to realtime.
      if (newMode === "realtime") manualDisconnectRef.current = false;
      setMode(newMode);
    },
    [mode, isConnected, disconnect]
  );

  // --- Charts: single-satellite mode (replay or single satellite) ---
  const { data: singleChartData, isLoading: singleChartsLoading } = useTimeSeriesStore({
    conn,
    schema: orbitSchema,
    mode: isMultiSatellite ? "replay" : mode, // disable realtime tick loop when multi-sat
    replayPoints: isMultiSatellite ? null : replayPoints,
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
  }, [mode, replayPoints, snapshot.elapsedTime, realtimePlayback.snapshot.isLive, realtimePlayback.snapshot.currentTime]);

  // Single-satellite chart data slicing (replay / single sat)
  const chartArrays = useMemo(() => {
    if (isMultiSatellite || !singleChartData) return null;
    return [
      singleChartData.t, singleChartData.altitude, singleChartData.energy,
      singleChartData.angular_momentum, singleChartData.velocity,
      singleChartData.a, singleChartData.e, singleChartData.inc_deg, singleChartData.raan_deg,
    ];
  }, [isMultiSatellite, singleChartData]);

  const visibleArrays = useMemo(
    () => sliceArrays(chartArrays, chartCurrentTime, timeRange),
    [chartArrays, chartCurrentTime, timeRange],
  );

  const visibleChartData = useMemo((): ChartDataMap | null => {
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

  // --- Derived values ---
  const satellitePosition =
    mode === "replay"
      ? snapshot.satellitePosition
      : realtimePlayback.snapshot.satellitePosition;

  const centralBody =
    mode === "realtime"
      ? (simInfo?.central_body ?? "earth")
      : (csvMetadata?.centralBody ?? "earth");
  const centralBodyRadius =
    mode === "realtime"
      ? (simInfo?.central_body_radius ?? 6378.137)
      : (csvMetadata?.centralBodyRadius ?? 6378.137);

  const epochJd =
    mode === "realtime"
      ? (simInfo?.epoch_jd ?? undefined)
      : (csvMetadata?.epochJd ?? undefined);

  const trailVisibleCount =
    mode === "replay"
      ? snapshot.trailVisibleCount
      : (realtimePlayback.snapshot.isLive ? undefined : realtimePlayback.snapshot.trailVisibleCount);

  // Draw start for replay mode time-range clipping
  const replayTrailDrawStart = useMemo(() => {
    if (mode !== "replay" || !replayPoints || replayPoints.length === 0) return 0;
    const currentT = replayPoints[0].t + snapshot.elapsedTime;
    return computeReplayDrawStart(replayPoints, currentT, timeRange);
  }, [mode, timeRange, replayPoints, snapshot.elapsedTime]);

  // Total points across all satellite buffers
  const totalPoints = useMemo(() => {
    let count = 0;
    for (const buf of trailBuffersRef.current.values()) count += buf.length;
    return count;
  }, [realtimePlayback.snapshot.currentTime]);

  const showPlaybackBar =
    mode === "realtime"
      ? totalPoints > 0
      : replayPoints != null && replayPoints.length > 0;

  // Satellite info display
  const satInfoText = useMemo(() => {
    if (!simInfo) return null;
    const parts: string[] = [];
    for (const sat of simInfo.satellites) {
      parts.push(sat.name ?? sat.id);
    }
    return parts.join(" | ");
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
        satellitePositions={mode === "realtime" ? realtimePlayback.snapshot.satellitePositions : undefined}
        trailVisibleCounts={mode === "realtime" && !realtimePlayback.snapshot.isLive ? realtimePlayback.snapshot.trailVisibleCounts : undefined}
        trailDrawStarts={mode === "realtime" && timeRange != null ? realtimePlayback.snapshot.trailDrawStarts : undefined}
        centralBody={centralBody}
        centralBodyRadius={centralBodyRadius}
        epochJd={epochJd ?? null}
        referenceFrame={referenceFrame}
        satelliteNames={satelliteNames}
        physicalScale={isSatCentered}
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
                placeholder="ws://localhost:9001"
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
                {isConnected ? "Connected" : "Disconnected"}
              </span>
            </div>

            {simInfo && (
              <div className="orbit-info">
                {satInfoText && <><strong>{satInfoText}</strong> | </>}
                {simInfo.epoch_jd != null && <>{jd_to_utc_string(simInfo.epoch_jd, 0)} | </>}
                mu={simInfo.mu.toFixed(2)} km^3/s^2 | dt={simInfo.dt.toFixed(1)} s | stream={simInfo.stream_interval.toFixed(1)} s
              </div>
            )}

            {totalPoints > 0 && (
              <div className="orbit-info">
                {totalPoints} points
              </div>
            )}
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
        />
      )}

      {showPlaybackBar && (
        mode === "realtime" ? (
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
        )
      )}
    </div>
  );
}
