import { useState, useCallback, useRef, useEffect } from "react";
import { Scene } from "./components/Scene.js";
import { PlaybackBar } from "./components/PlaybackBar.js";
import { GraphPanel } from "./components/GraphPanel.js";
import { usePlayback } from "./hooks/usePlayback.js";
import { useWebSocket, SimInfo } from "./hooks/useWebSocket.js";
import { useDuckDB } from "./hooks/useDuckDB.js";
import { useOrbitCharts, TimeRange } from "./hooks/useOrbitCharts.js";
import { IngestBuffer } from "./db/IngestBuffer.js";
import { TrailBuffer } from "./utils/TrailBuffer.js";
import { parseOrbitCSV, OrbitPoint } from "./orbit.js";

/** The two viewer modes. */
type ViewerMode = "replay" | "realtime";

const DEFAULT_WS_URL = "ws://localhost:9001";

/**
 * Main application component.
 *
 * Supports two modes:
 *   - "Replay": Load CSV orbit data and play it back with time controls.
 *   - "Realtime": Connect to a WebSocket server and display orbit data
 *     as it streams in from a running simulation.
 */
export function App() {
  // --- Mode toggle ---
  const [mode, setMode] = useState<ViewerMode>("realtime");

  // --- Replay mode state ---
  const [replayPoints, setReplayPoints] = useState<OrbitPoint[] | null>(null);
  const [orbitInfo, setOrbitInfo] = useState<string>("");
  const fileInputRef = useRef<HTMLInputElement>(null);
  const { controller, snapshot } = usePlayback(replayPoints);

  // --- DuckDB + Charts ---
  const { conn, isReady: dbReady } = useDuckDB();

  // --- Chart time range ---
  const [timeRange, setTimeRange] = useState<TimeRange>(null);

  // --- Realtime mode state ---
  const [wsUrl, setWsUrl] = useState(DEFAULT_WS_URL);
  const [simInfo, setSimInfo] = useState<SimInfo | null>(null);
  const realtimePointsRef = useRef<OrbitPoint[]>([]);
  const rafScheduledRef = useRef(false);
  // Cumulative time offset: when the server loops t back to 0,
  // add the previous max t so charts show monotonically increasing time.
  const tOffsetRef = useRef(0);
  const lastRawTRef = useRef(-1);
  const overviewEndIndexRef = useRef(0);
  const detailBufferRef = useRef<OrbitPoint[]>([]);
  // Version counter triggers React re-renders at RAF rate without
  // creating a new array copy on every WebSocket message.
  const [realtimeVersion, setRealtimeVersion] = useState(0);

  // --- IngestBuffer for DuckDB (drain pattern) ---
  const ingestBufferRef = useRef(new IngestBuffer());

  // --- TrailBuffer for 3D rendering (bounded) ---
  const trailBufferRef = useRef(new TrailBuffer(50000));

  const handleState = useCallback((point: OrbitPoint) => {
    // Detect orbit restart: server loops t back to 0 after one period.
    if (point.t < lastRawTRef.current) {
      tOffsetRef.current += lastRawTRef.current;
    }
    lastRawTRef.current = point.t;

    const adjusted = { ...point, t: point.t + tOffsetRef.current };
    realtimePointsRef.current.push(adjusted);
    ingestBufferRef.current.push(adjusted);
    trailBufferRef.current.push(adjusted);
    // Batch state updates to at most once per animation frame to avoid
    // overwhelming React with re-renders (messages arrive every ~100ms).
    if (!rafScheduledRef.current) {
      rafScheduledRef.current = true;
      requestAnimationFrame(() => {
        rafScheduledRef.current = false;
        setRealtimeVersion((v) => v + 1);
      });
    }
  }, []);

  const handleInfo = useCallback((info: SimInfo) => {
    setSimInfo(info);
  }, []);

  const handleHistory = useCallback((points: OrbitPoint[]) => {
    const adjusted: OrbitPoint[] = [];
    for (const point of points) {
      if (point.t < lastRawTRef.current) {
        tOffsetRef.current += lastRawTRef.current;
      }
      lastRawTRef.current = point.t;
      const p = { ...point, t: point.t + tOffsetRef.current };
      realtimePointsRef.current.push(p);
      adjusted.push(p);
    }
    ingestBufferRef.current.pushMany(adjusted);
    trailBufferRef.current.pushMany(adjusted);
    overviewEndIndexRef.current = realtimePointsRef.current.length;
    // Trigger single re-render for entire batch
    if (!rafScheduledRef.current) {
      rafScheduledRef.current = true;
      requestAnimationFrame(() => {
        rafScheduledRef.current = false;
        setRealtimeVersion((v) => v + 1);
      });
    }
  }, []);

  const handleHistoryDetail = useCallback((points: OrbitPoint[]) => {
    // Accumulate detail points with t-offset processing
    for (const point of points) {
      detailBufferRef.current.push(point);
    }
  }, []);

  const handleHistoryDetailComplete = useCallback(() => {
    if (detailBufferRef.current.length === 0) return;

    // Process detail buffer through t-offset logic independently
    const detailPoints: OrbitPoint[] = [];
    let detailOffset = 0;
    let detailLastRawT = -1;
    for (const point of detailBufferRef.current) {
      if (point.t < detailLastRawT) {
        detailOffset += detailLastRawT;
      }
      detailLastRawT = point.t;
      detailPoints.push({
        ...point,
        t: point.t + detailOffset,
      });
    }
    detailBufferRef.current = [];

    // Replace overview portion with detail, keep streaming portion
    const streamingPoints = realtimePointsRef.current.slice(overviewEndIndexRef.current);
    realtimePointsRef.current = [...detailPoints, ...streamingPoints];
    overviewEndIndexRef.current = detailPoints.length;

    // Rebuild TrailBuffer with detail + streaming
    trailBufferRef.current.clear();
    trailBufferRef.current.pushMany([...detailPoints, ...streamingPoints]);

    // Re-ingest all data into DuckDB (detail replaced overview)
    ingestBufferRef.current = new IngestBuffer();
    ingestBufferRef.current.pushMany([...detailPoints, ...streamingPoints]);

    // Trigger re-render
    if (!rafScheduledRef.current) {
      rafScheduledRef.current = true;
      requestAnimationFrame(() => {
        rafScheduledRef.current = false;
        setRealtimeVersion((v) => v + 1);
      });
    }
  }, []);

  const { connect, disconnect, isConnected } = useWebSocket({
    url: wsUrl,
    onState: handleState,
    onInfo: handleInfo,
    onHistory: handleHistory,
    onHistoryDetail: handleHistoryDetail,
    onHistoryDetailComplete: handleHistoryDetailComplete,
  });

  // --- Replay: CSV loading ---
  const handleLoadClick = useCallback(() => {
    fileInputRef.current?.click();
  }, []);

  const handleFileChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;

      const reader = new FileReader();
      reader.onload = () => {
        const text = reader.result as string;
        const parsed = parseOrbitCSV(text);

        if (parsed.length === 0) {
          setOrbitInfo("No valid orbit data found in file.");
          setReplayPoints(null);
          return;
        }

        setReplayPoints(parsed);

        const duration = parsed[parsed.length - 1].t - parsed[0].t;
        setOrbitInfo(
          `Loaded: ${file.name} | ${parsed.length} points | Duration: ${duration.toFixed(1)} s`
        );
      };

      reader.readAsText(file);

      // Reset file input so the same file can be re-loaded
      e.target.value = "";
    },
    []
  );

  // --- Realtime: connect / disconnect ---
  const handleConnect = useCallback(() => {
    // Clear previous realtime data when starting a new connection
    realtimePointsRef.current = [];
    tOffsetRef.current = 0;
    lastRawTRef.current = -1;
    overviewEndIndexRef.current = 0;
    detailBufferRef.current = [];
    ingestBufferRef.current = new IngestBuffer();
    trailBufferRef.current.clear();
    setRealtimeVersion(0);
    setSimInfo(null);
    connect();
  }, [connect]);

  const handleDisconnect = useCallback(() => {
    disconnect();
  }, [disconnect]);

  // --- Auto-connect in realtime mode ---
  // Deps include isConnected so we reconnect after HMR or unexpected drops.
  // No infinite loop: failed connects don't toggle isConnected again.
  useEffect(() => {
    if (mode === "realtime" && !isConnected) {
      handleConnect();
    }
  }, [mode, isConnected, handleConnect]);

  // --- Mode switching ---
  const handleModeChange = useCallback(
    (newMode: ViewerMode) => {
      if (newMode === mode) return;

      // Disconnect WebSocket when leaving realtime mode
      if (mode === "realtime" && isConnected) {
        disconnect();
      }

      setMode(newMode);
    },
    [mode, isConnected, disconnect]
  );

  // --- Charts ---
  const { chartData, isLoading: chartsLoading } = useOrbitCharts({
    conn,
    mode,
    replayPoints,
    ingestBufferRef,
    mu: simInfo?.mu,
    bodyRadius: simInfo?.central_body_radius,
    timeRange,
  });

  // --- Determine what the 3D scene should display ---
  // In replay mode: use replay points with playback snapshot
  // In realtime mode: use accumulated realtime points, always showing
  //   the latest position with the full trail.
  // `realtimeVersion` is read to ensure React re-renders when new points arrive.
  const rtPoints = realtimePointsRef.current;
  void realtimeVersion; // consumed for reactivity
  const satellitePosition =
    mode === "replay"
      ? snapshot.satellitePosition
      : trailBufferRef.current.latest;

  const centralBody = simInfo?.central_body ?? "earth";
  const centralBodyRadius = simInfo?.central_body_radius ?? 6378.137;

  return (
    <>
      {/* 3D Scene */}
      <Scene
        points={mode === "replay" ? replayPoints : undefined}
        satellitePosition={satellitePosition}
        trailVisibleCount={mode === "replay" ? snapshot.trailVisibleCount : undefined}
        trailBuffer={mode === "realtime" ? trailBufferRef.current : undefined}
        centralBody={centralBody}
        centralBodyRadius={centralBodyRadius}
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
                T={simInfo.period.toFixed(1)} s | dt={simInfo.dt.toFixed(1)} s
              </div>
            )}

            {/* Realtime data stats */}
            {trailBufferRef.current.length > 0 && (
              <div className="orbit-info">
                {trailBufferRef.current.length} points |
                T+{((trailBufferRef.current.latest?.t ?? 0) - (trailBufferRef.current.getAll()[0]?.t ?? 0)).toFixed(1)} s
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
          chartData={chartData}
          isLoading={chartsLoading}
          timeRange={timeRange}
          onTimeRangeChange={setTimeRange}
        />
      )}

      {/* Playback bar (only shown in replay mode when data is loaded) */}
      {mode === "replay" && controller && (
        <PlaybackBar
          playback={controller}
          isPlaying={snapshot.isPlaying}
          fraction={snapshot.fraction}
          elapsedTime={snapshot.elapsedTime}
          totalDuration={snapshot.totalDuration}
        />
      )}
    </>
  );
}
