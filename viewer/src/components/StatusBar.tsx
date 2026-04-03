import type { SatelliteInfo, SimInfo } from "../hooks/useWebSocket.js";
import type { ReferenceFrame } from "../referenceFrame.js";
import type { ServerState } from "../sources/eventDispatcher.js";
import { jd_to_utc_string } from "../wasm/kanameInit.js";
import { FrameSelector } from "./FrameSelector.js";
import styles from "./StatusBar.module.css";

interface StatusBarProps {
  // Connection
  isConnected: boolean;
  serverState: ServerState;
  wsUrl: string;
  onWsUrlChange: (url: string) => void;
  onConnect: () => void;
  onDisconnect: () => void;
  // Sim control (inline when running/paused)
  onPause: () => void;
  onResume: () => void;
  onTerminate: () => void;
  // Sim info
  simInfo: SimInfo | null;
  totalPoints: number;
  activePerturbations: string[];
  epochJd?: number;
  // Actions
  onLoadFileClick: () => void;
  /** File load info text (e.g. "Loaded: file.csv | 100 points") */
  fileInfo?: string;
  // SimConfigForm modal trigger
  onOpenSimConfig: () => void;
  // Frame
  referenceFrame: ReferenceFrame;
  onFrameChange: (frame: ReferenceFrame) => void;
  satellites?: SatelliteInfo[];
  hasEpoch?: boolean;
  centralBody?: string;
}

export function StatusBar({
  isConnected,
  serverState,
  wsUrl,
  onWsUrlChange,
  onConnect,
  onDisconnect,
  onPause,
  onResume,
  onTerminate,
  simInfo,
  totalPoints,
  activePerturbations,
  epochJd,
  onLoadFileClick,
  fileInfo,
  onOpenSimConfig,
  referenceFrame,
  onFrameChange,
  satellites,
  hasEpoch,
  centralBody,
}: StatusBarProps) {
  const statusLabel = isConnected
    ? serverState === "idle"
      ? "Connected (Idle)"
      : serverState === "paused"
        ? "Connected (Paused)"
        : "Connected"
    : "Disconnected";

  const satNames = simInfo
    ? simInfo.satellites.map((sat) => sat.name ?? sat.id).join(" | ")
    : null;

  return (
    <div className={styles.statusBar} data-testid="ui-overlay">
      {/* Left: Status + WS URL */}
      <div className={styles.left}>
        <span
          className={`${styles.statusDot} ${isConnected ? styles.connected : styles.disconnected}`}
        />
        <span className={styles.statusText} data-testid="ws-status-text">
          {statusLabel}
        </span>

        {!isConnected && (
          <div className={styles.wsUrlRow}>
            <input
              type="text"
              className={styles.wsUrlInput}
              data-testid="ws-url-input"
              value={wsUrl}
              onChange={(e) => onWsUrlChange(e.target.value)}
              placeholder="ws://localhost:9001/ws"
            />
            <button
              className={`${styles.btn} ${styles.connectBtn}`}
              data-testid="ws-connect-btn"
              onClick={onConnect}
            >
              Connect
            </button>
          </div>
        )}

        {isConnected && (
          <button
            className={`${styles.btn} ${styles.disconnectBtn}`}
            data-testid="ws-disconnect-btn"
            onClick={onDisconnect}
          >
            Disconnect
          </button>
        )}
      </div>

      {/* Center: SimInfo + File info */}
      <div className={styles.center}>
        {fileInfo && (
          <span className={styles.fileInfo} data-testid="orbit-info-file">
            {fileInfo}
          </span>
        )}
        {simInfo && (
          <>
            <span className={styles.simInfo} data-testid="orbit-info-sim">
              {satNames && (
                <>
                  <strong>{satNames}</strong> |{" "}
                </>
              )}
              {epochJd != null && <>{jd_to_utc_string(epochJd, 0)} | </>}
              mu={simInfo.mu.toFixed(2)} km^3/s^2 | dt={simInfo.dt.toFixed(1)} s | stream=
              {simInfo.stream_interval.toFixed(1)} s
              {activePerturbations.length > 0 && (
                <span>
                  {" | "}
                  {activePerturbations.map((p) => (
                    <span key={p} className={styles.pertTag}>
                      {p}
                    </span>
                  ))}
                </span>
              )}
            </span>

            {totalPoints > 0 && (
              <>
                <span className={styles.separator}>|</span>
                <span className={styles.pointCount} data-testid="orbit-info-points">
                  {totalPoints} points
                </span>
              </>
            )}
          </>
        )}
      </div>

      {/* Right: Controls + Frame + Load */}
      <div className={styles.right}>
        {/* Sim control buttons when running/paused */}
        {isConnected && (serverState === "running" || serverState === "paused") && (
          <>
            {serverState === "running" ? (
              <button
                className={`${styles.btn} ${styles.pauseBtn}`}
                data-testid="sim-pause-btn"
                onClick={onPause}
              >
                Pause
              </button>
            ) : (
              <button
                className={`${styles.btn} ${styles.resumeBtn}`}
                data-testid="sim-resume-btn"
                onClick={onResume}
              >
                Resume
              </button>
            )}
            <button
              className={`${styles.btn} ${styles.terminateBtn}`}
              data-testid="sim-terminate-btn"
              onClick={onTerminate}
            >
              Stop
            </button>
          </>
        )}

        {/* Configure button when idle+connected */}
        {isConnected && serverState === "idle" && (
          <button
            className={`${styles.btn} ${styles.configureBtn}`}
            onClick={onOpenSimConfig}
          >
            Configure
          </button>
        )}

        {/* Frame selector */}
        <FrameSelector
          referenceFrame={referenceFrame}
          onChange={onFrameChange}
          satellites={satellites}
          hasEpoch={hasEpoch}
          centralBody={centralBody}
        />

        {/* Load file button */}
        <button
          className={`${styles.btn} ${styles.loadBtn}`}
          onClick={onLoadFileClick}
        >
          Load File
        </button>
      </div>
    </div>
  );
}
