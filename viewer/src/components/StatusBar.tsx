import type { ServerState } from "../sources/eventDispatcher.js";
import styles from "./StatusBar.module.css";

interface StatusBarProps {
  // Connection
  isConnected: boolean;
  serverState: ServerState;
  wsUrl: string;
  onWsUrlChange: (url: string) => void;
  onConnect: () => void;
  onDisconnect: () => void;
  // Sim control
  onPause: () => void;
  onResume: () => void;
  onTerminate: () => void;
  // Actions
  onLoadFileClick: () => void;
  onOpenSimConfig: () => void;
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
  onLoadFileClick,
  onOpenSimConfig,
}: StatusBarProps) {
  const statusLabel = isConnected
    ? serverState === "idle"
      ? "Connected (Idle)"
      : serverState === "paused"
        ? "Connected (Paused)"
        : "Connected"
    : "Disconnected";

  return (
    <div className={styles.statusBar} data-testid="ui-overlay">
      <span
        className={`${styles.statusDot} ${isConnected ? styles.connected : styles.disconnected}`}
      />
      <span className={styles.statusText} data-testid="ws-status-text">
        {statusLabel}
      </span>

      {!isConnected && (
        <>
          <input
            type="text"
            className={styles.wsUrlInput}
            data-testid="ws-url-input"
            value={wsUrl}
            onChange={(e) => onWsUrlChange(e.target.value)}
            placeholder="ws://localhost:9001/ws"
          />
          <button
            type="button"
            className={`${styles.btn} ${styles.connectBtn}`}
            data-testid="ws-connect-btn"
            onClick={onConnect}
          >
            Connect
          </button>
        </>
      )}

      {isConnected && (
        <button
          type="button"
          className={`${styles.btn} ${styles.disconnectBtn}`}
          data-testid="ws-disconnect-btn"
          onClick={onDisconnect}
        >
          Disconnect
        </button>
      )}

      <div className={styles.spacer} />

      {isConnected && (serverState === "running" || serverState === "paused") && (
        <>
          {serverState === "running" ? (
            <button
              type="button"
              className={`${styles.btn} ${styles.pauseBtn}`}
              data-testid="sim-pause-btn"
              onClick={onPause}
            >
              Pause
            </button>
          ) : (
            <button
              type="button"
              className={`${styles.btn} ${styles.resumeBtn}`}
              data-testid="sim-resume-btn"
              onClick={onResume}
            >
              Resume
            </button>
          )}
          <button
            type="button"
            className={`${styles.btn} ${styles.terminateBtn}`}
            data-testid="sim-terminate-btn"
            onClick={onTerminate}
          >
            Stop
          </button>
        </>
      )}

      {isConnected && serverState === "idle" && (
        <button
          type="button"
          className={`${styles.btn} ${styles.configureBtn}`}
          onClick={onOpenSimConfig}
        >
          Configure
        </button>
      )}

      <button type="button" className={`${styles.btn} ${styles.loadBtn}`} onClick={onLoadFileClick}>
        Load File
      </button>
    </div>
  );
}
