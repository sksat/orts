import styles from "./SimControlBar.module.css";

export interface SimControlBarProps {
  serverState: "running" | "paused";
  onPause: () => void;
  onResume: () => void;
  onTerminate: () => void;
}

export function SimControlBar({ serverState, onPause, onResume, onTerminate }: SimControlBarProps) {
  return (
    <div className={styles.controlBar}>
      {serverState === "running" ? (
        <button className={`${styles.controlBtn} ${styles.pauseBtn}`} data-testid="sim-pause-btn" onClick={onPause}>
          Pause
        </button>
      ) : (
        <button className={`${styles.controlBtn} ${styles.resumeBtn}`} data-testid="sim-resume-btn" onClick={onResume}>
          Resume
        </button>
      )}
      <button className={`${styles.controlBtn} ${styles.terminateBtn}`} data-testid="sim-terminate-btn" onClick={onTerminate}>
        Stop
      </button>
    </div>
  );
}
