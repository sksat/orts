import { useState, useCallback, useRef, useEffect, useMemo } from "react";
import { Scene } from "./components/Scene.js";
import { PlaybackBar } from "./components/PlaybackBar.js";
import { GraphPanel } from "./components/GraphPanel.js";
import { usePlayback } from "./hooks/usePlayback.js";
import { useRealtimePlayback } from "./hooks/useRealtimePlayback.js";
import { useWebSocket, SimInfo, QueryRangeResponse } from "./hooks/useWebSocket.js";
import {
  useDuckDB,
  useTimeSeriesStore,
  IngestBuffer,
  sliceArrays,
  quantizeChartTime,
  type TimeRange,
  type ChartDataMap,
} from "@orts/uneri";
import { createOrbitSchema } from "./db/orbitSchema.js";
import { TrailBuffer } from "./utils/TrailBuffer.js";
import { parseOrbitCSVWithMetadata, CSVMetadata, OrbitPoint } from "./orbit.js";
import { mergeQueryRangePoints } from "./utils/mergeQueryRange.js";

/** The two viewer modes. */
type ViewerMode = "replay" | "realtime";

const DEFAULT_WS_URL = "ws://localhost:9001";

/**
 * Main application component.
 *
 * Supports two modes:
 *   - "Replay": Load CSV orbit data and play it back with time controls.
 *   - "Realtime": Connect to a WebSocket server and display orbit data
 *     as it streams in from a running simulation, with history scrubbing.
 *
 * Both modes share a unified PlaybackBar for timeline control.
 */
export function App() {
  // --- Mode toggle ---
  const [mode, setMode] = useState<ViewerMode>("realtime");

  // --- Replay mode state ---
  const [replayPoints, setReplayPoints] = useState<OrbitPoint[] | null>(null);
  const [csvMetadata, setCsvMetadata] = useState<CSVMetadata | null>(null);
  const [orbitInfo, setOrbitInfo] = useState<string>("");
  const fileInputRef = useRef<HTMLInputElement>(null);
  const { snapshot, togglePlayPause, setSpeed, seekToFraction } = usePlayback(replayPoints);

  // --- Chart time range ---
  const [timeRange, setTimeRange] = useState<TimeRange>(null);

  // --- Realtime mode state ---
  const [wsUrl, setWsUrl] = useState(DEFAULT_WS_URL);
  const [simInfo, setSimInfo] = useState<SimInfo | null>(null);

  // --- DuckDB + Charts ---
  const mu = mode === "realtime" ? simInfo?.mu : (csvMetadata?.mu ?? undefined);
  const bodyRadius = mode === "realtime" ? simInfo?.central_body_radius : (csvMetadata?.centralBodyRadius ?? undefined);
  const orbitSchema = useMemo(() => createOrbitSchema(mu ?? 398600.4418, bodyRadius ?? 6378.137), [mu, bodyRadius]);
  const { conn, isReady: dbReady } = useDuckDB(orbitSchema);
  const detailBufferRef = useRef<OrbitPoint[]>([]);
  // Count of points received as streaming (after history overview).
  // Used to track how many trailing points to preserve on detail complete.
  const streamingCountRef = useRef(0);

  // --- IngestBuffer for DuckDB (drain pattern) ---
  const ingestBufferRef = useRef(new IngestBuffer<OrbitPoint>());

  // --- TrailBuffer for 3D rendering (bounded) ---
  const trailBufferRef = useRef(new TrailBuffer(50000));

  // --- Realtime playback (history scrubbing) ---
  const realtimePlayback = useRealtimePlayback(trailBufferRef.current);

  const handleState = useCallback((point: OrbitPoint) => {
    ingestBufferRef.current.push(point);
    trailBufferRef.current.push(point);
    streamingCountRef.current++;
  }, []);

  const handleInfo = useCallback((info: SimInfo) => {
    setSimInfo(info);
  }, []);

  const handleHistory = useCallback((points: OrbitPoint[]) => {
    ingestBufferRef.current.pushMany(points);
    trailBufferRef.current.pushMany(points);
    streamingCountRef.current = 0;
  }, []);

  const handleHistoryDetail = useCallback((points: OrbitPoint[]) => {
    for (const point of points) {
      detailBufferRef.current.push(point);
    }
  }, []);

  const handleHistoryDetailComplete = useCallback(() => {
    if (detailBufferRef.current.length === 0) return;

    // Server sends monotonically increasing t, so detail points are ready as-is.
    const detailPoints = detailBufferRef.current;
    detailBufferRef.current = [];

    // Get streaming points that arrived after the overview.
    // Clamp streamingCountRef to avoid referencing evicted TrailBuffer entries.
    const allTrailPoints = trailBufferRef.current.getAll();
    const safeStreamingCount = Math.min(
      streamingCountRef.current,
      allTrailPoints.length,
    );
    const streamingPoints = allTrailPoints.slice(
      allTrailPoints.length - safeStreamingCount,
    );

    const combined = [...detailPoints, ...streamingPoints];
    // Guard: ensure monotonic time order even if streamingCountRef drifted
    // (e.g., due to TrailBuffer trimming). Out-of-order data breaks interpolateAt.
    combined.sort((a, b) => a.t - b.t);

    // Rebuild TrailBuffer with detail + streaming for 3D rendering.
    trailBufferRef.current.clear();
    trailBufferRef.current.pushMany(combined);

    // Rebuild DuckDB via IngestBuffer (tick loop will clear + insert).
    // This replaces the overview data with full-resolution detail.
    ingestBufferRef.current.markRebuild(combined);
  }, []);

  const handleQueryRangeResponse = useCallback((response: QueryRangeResponse) => {
    // Merge response with any streaming points newer than the response range
    // to prevent the 3D satellite position from rewinding on chart zoom.
    const allTrailPoints = trailBufferRef.current.getAll();
    const combined = mergeQueryRangePoints(response.points, allTrailPoints);

    ingestBufferRef.current.markRebuild(combined);
    trailBufferRef.current.clear();
    trailBufferRef.current.pushMany(combined);
  }, []);

  const { connect, disconnect, isConnected, send } = useWebSocket({
    url: wsUrl,
    onState: handleState,
    onInfo: handleInfo,
    onHistory: handleHistory,
    onHistoryDetail: handleHistoryDetail,
    onHistoryDetailComplete: handleHistoryDetailComplete,
    onQueryRangeResponse: handleQueryRangeResponse,
  });

  // Chart zoom -> query_range is disabled until t-offset mismatch is fixed.
  const handleChartZoom = useCallback((tMin: number, tMax: number) => {
    send({ type: "query_range", t_min: tMin, t_max: tMax, max_points: 2000 });
  }, [send]);

  // --- Replay: file loading (shared by file input and D&D) ---
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

      // Switch to replay mode
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
      // Reset file input so the same file can be re-loaded
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
    if (file) {
      loadCSVFile(file);
    }
  }, [loadCSVFile]);

  // --- Realtime: connect / disconnect ---
  const handleConnect = useCallback(() => {
    // Clear previous realtime data when starting a new connection
    detailBufferRef.current = [];
    streamingCountRef.current = 0;
    ingestBufferRef.current = new IngestBuffer<OrbitPoint>();
    trailBufferRef.current.clear();
    setSimInfo(null);
    realtimePlayback.goLive();
    connect();
  }, [connect, realtimePlayback.goLive]);

  const handleDisconnect = useCallback(() => {
    disconnect();
  }, [disconnect]);

  // --- Auto-connect in realtime mode ---
  useEffect(() => {
    if (mode === "realtime" && !isConnected) {
      handleConnect();
    }
  }, [mode, isConnected, handleConnect]);

  // --- Mode switching ---
  const handleModeChange = useCallback(
    (newMode: ViewerMode) => {
      if (newMode === mode) return;

      if (mode === "realtime" && isConnected) {
        disconnect();
      }

      setMode(newMode);
    },
    [mode, isConnected, disconnect]
  );

  // --- Charts ---
  const { data: chartData, isLoading: chartsLoading } = useTimeSeriesStore({
    conn,
    schema: orbitSchema,
    mode,
    replayPoints,
    ingestBufferRef,
    timeRange,
  });

  // --- Chart viewport slicing: right edge follows current playback time ---
  const chartCurrentTime = useMemo(() => {
    if (mode === "replay") {
      if (!replayPoints || replayPoints.length === 0) return undefined;
      return quantizeChartTime(replayPoints[0].t + snapshot.elapsedTime);
    }
    if (realtimePlayback.snapshot.isLive) return undefined;
    return quantizeChartTime(realtimePlayback.snapshot.currentTime);
  }, [mode, replayPoints, snapshot.elapsedTime, realtimePlayback.snapshot.isLive, realtimePlayback.snapshot.currentTime]);

  const chartArrays = useMemo(() => {
    if (!chartData) return null;
    return [
      chartData.t, chartData.altitude, chartData.energy, chartData.angular_momentum, chartData.velocity,
      chartData.a, chartData.e, chartData.inc_deg, chartData.raan_deg,
    ];
  }, [chartData]);

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

  // TrailVisibleCount: in live mode show all, in scrub mode show up to scrubbed time
  const trailVisibleCount =
    mode === "replay"
      ? snapshot.trailVisibleCount
      : (realtimePlayback.snapshot.isLive ? undefined : realtimePlayback.snapshot.trailVisibleCount);

  // Determine if PlaybackBar should be shown
  const showPlaybackBar =
    mode === "realtime"
      ? trailBufferRef.current.length > 0
      : replayPoints != null && replayPoints.length > 0;

  return (
    <div
      className="app-root"
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      {/* Drop overlay */}
      {isDragOver && (
        <div className="drop-overlay">
          <div className="drop-overlay-text">Drop CSV file to load</div>
        </div>
      )}

      {/* 3D Scene */}
      <Scene
        points={mode === "replay" ? replayPoints : undefined}
        satellitePosition={satellitePosition}
        trailVisibleCount={trailVisibleCount}
        trailBuffer={mode === "realtime" ? trailBufferRef.current : undefined}
        centralBody={centralBody}
        centralBodyRadius={centralBodyRadius}
        epochJd={epochJd ?? null}
      />

      {/* UI overlay */}
      <div className="ui-overlay">
        {/* Mode toggle */}
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

        {/* Replay mode controls */}
        {mode === "replay" && (
          <>
            <button className="load-csv-btn" onClick={handleLoadClick}>
              Load Orbit CSV
            </button>
            {orbitInfo && <div className="orbit-info">{orbitInfo}</div>}
          </>
        )}

        {/* Realtime mode controls */}
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

            {/* Connection status */}
            <div className="ws-status">
              <span className={`ws-status-dot ${isConnected ? "connected" : "disconnected"}`} />
              <span className="ws-status-text">
                {isConnected ? "Connected" : "Disconnected"}
              </span>
            </div>

            {/* Sim info (shown after server sends info message) */}
            {simInfo && (
              <div className="orbit-info">
                mu={simInfo.mu.toFixed(2)} km^3/s^2 | alt={simInfo.altitude.toFixed(1)} km |
                T={simInfo.period.toFixed(1)} s | dt={simInfo.dt.toFixed(1)} s | stream={simInfo.stream_interval.toFixed(1)} s
              </div>
            )}

            {/* Points count */}
            {trailBufferRef.current.length > 0 && (
              <div className="orbit-info">
                {trailBufferRef.current.length} points
              </div>
            )}
          </div>
        )}
      </div>

      {/* Hidden file input (replay mode) */}
      <input
        ref={fileInputRef}
        type="file"
        accept=".csv,.txt"
        style={{ display: "none" }}
        onChange={handleFileChange}
      />

      {/* Graph panel (right side) */}
      {dbReady && (
        <GraphPanel
          chartData={visibleChartData}
          isLoading={chartsLoading}
          timeRange={timeRange}
          onTimeRangeChange={setTimeRange}
          onZoom={handleChartZoom}
        />
      )}

      {/* Unified PlaybackBar (shown in both modes when data is available) */}
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
