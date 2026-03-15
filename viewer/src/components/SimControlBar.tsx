export interface SimControlBarProps {
  serverState: "running" | "paused";
  onPause: () => void;
  onResume: () => void;
  onTerminate: () => void;
}

export function SimControlBar({ serverState, onPause, onResume, onTerminate }: SimControlBarProps) {
  return (
    <div className="sim-control-bar">
      {serverState === "running" ? (
        <button className="sim-control-btn sim-pause-btn" onClick={onPause}>
          Pause
        </button>
      ) : (
        <button className="sim-control-btn sim-resume-btn" onClick={onResume}>
          Resume
        </button>
      )}
      <button className="sim-control-btn sim-terminate-btn" onClick={onTerminate}>
        Stop
      </button>
    </div>
  );
}
