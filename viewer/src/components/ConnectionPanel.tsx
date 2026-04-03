import styles from "../App.module.css";
import type { ServerState } from "../sources/eventDispatcher.js";
import type { SimConfigPayload } from "./SimConfigForm.js";
import { SimConfigForm } from "./SimConfigForm.js";
import { SimControlBar } from "./SimControlBar.js";

interface ConnectionPanelProps {
  wsUrl: string;
  onWsUrlChange: (url: string) => void;
  isConnected: boolean;
  serverState: ServerState; // "unknown" | "idle" | "running" | "paused"
  onConnect: () => void;
  onDisconnect: () => void;
  onStartSimulation: (config: SimConfigPayload) => void;
  onPause: () => void;
  onResume: () => void;
  onTerminate: () => void;
}

export function ConnectionPanel({
  wsUrl,
  onWsUrlChange,
  isConnected,
  serverState,
  onConnect,
  onDisconnect,
  onStartSimulation,
  onPause,
  onResume,
  onTerminate,
}: ConnectionPanelProps) {
  return (
    <div className={styles.realtimeControls}>
      <div className={styles.wsUrlRow}>
        <input
          type="text"
          className={styles.wsUrlInput}
          data-testid="ws-url-input"
          value={wsUrl}
          onChange={(e) => onWsUrlChange(e.target.value)}
          placeholder="ws://localhost:9001/ws"
          disabled={isConnected}
        />
        {isConnected ? (
          <button
            type="button"
            className={`${styles.wsBtn} ${styles.wsDisconnectBtn}`}
            data-testid="ws-disconnect-btn"
            onClick={onDisconnect}
          >
            Disconnect
          </button>
        ) : (
          <button
            type="button"
            className={`${styles.wsBtn} ${styles.wsConnectBtn}`}
            data-testid="ws-connect-btn"
            onClick={onConnect}
          >
            Connect
          </button>
        )}
      </div>

      <div className={styles.wsStatus}>
        <span
          className={`${styles.wsStatusDot} ${isConnected ? styles.connected : styles.disconnected}`}
        />
        <span className={styles.wsStatusText} data-testid="ws-status-text">
          {isConnected
            ? serverState === "idle"
              ? "Connected (Idle)"
              : serverState === "paused"
                ? "Connected (Paused)"
                : "Connected"
            : "Disconnected"}
        </span>
      </div>

      {isConnected && serverState === "idle" && <SimConfigForm onStart={onStartSimulation} />}

      {isConnected && (serverState === "running" || serverState === "paused") && (
        <SimControlBar
          serverState={serverState}
          onPause={onPause}
          onResume={onResume}
          onTerminate={onTerminate}
        />
      )}
    </div>
  );
}
