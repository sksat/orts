import { useState, useCallback, useRef } from "react";
import { Scene } from "./components/Scene.js";
import { PlaybackBar } from "./components/PlaybackBar.js";
import { usePlayback } from "./hooks/usePlayback.js";
import { useWebSocket, SimInfo } from "./hooks/useWebSocket.js";
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
  const [mode, setMode] = useState<ViewerMode>("replay");

  // --- Replay mode state ---
  const [replayPoints, setReplayPoints] = useState<OrbitPoint[] | null>(null);
  const [orbitInfo, setOrbitInfo] = useState<string>("");
  const fileInputRef = useRef<HTMLInputElement>(null);
  const { controller, snapshot } = usePlayback(replayPoints);

  // --- Realtime mode state ---
  const [wsUrl, setWsUrl] = useState(DEFAULT_WS_URL);
  const [realtimePoints, setRealtimePoints] = useState<OrbitPoint[]>([]);
  const [simInfo, setSimInfo] = useState<SimInfo | null>(null);
  const realtimePointsRef = useRef<OrbitPoint[]>([]);

  const handleState = useCallback((point: OrbitPoint) => {
    realtimePointsRef.current = [...realtimePointsRef.current, point];
    setRealtimePoints(realtimePointsRef.current);
  }, []);

  const handleInfo = useCallback((info: SimInfo) => {
    setSimInfo(info);
  }, []);

  const { connect, disconnect, isConnected } = useWebSocket({
    url: wsUrl,
    onState: handleState,
    onInfo: handleInfo,
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
    setRealtimePoints([]);
    setSimInfo(null);
    connect();
  }, [connect]);

  const handleDisconnect = useCallback(() => {
    disconnect();
  }, [disconnect]);

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

  // --- Determine what the 3D scene should display ---
  // In replay mode: use replay points with playback snapshot
  // In realtime mode: use accumulated realtime points, always showing
  //   the latest position with the full trail.
  const scenePoints =
    mode === "replay" ? replayPoints : realtimePoints.length > 0 ? realtimePoints : null;
  const satellitePosition =
    mode === "replay"
      ? snapshot.satellitePosition
      : realtimePoints.length > 0
        ? realtimePoints[realtimePoints.length - 1]
        : null;
  const trailVisibleCount =
    mode === "replay" ? snapshot.trailVisibleCount : realtimePoints.length;

  return (
    <>
      {/* 3D Scene */}
      <Scene
        points={scenePoints}
        satellitePosition={satellitePosition}
        trailVisibleCount={trailVisibleCount}
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
            {realtimePoints.length > 0 && (
              <div className="orbit-info">
                {realtimePoints.length} points |
                T+{(realtimePoints[realtimePoints.length - 1].t - realtimePoints[0].t).toFixed(1)} s
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
