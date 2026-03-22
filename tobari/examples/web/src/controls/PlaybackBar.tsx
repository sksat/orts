import type { ReactNode } from "react";

interface PlaybackBarProps {
  playing: boolean;
  onTogglePlay: () => void;
  speed: number;
  onSpeedChange: (speed: number) => void;
  epochJd: number;
  children?: ReactNode;
}

const SPEED_OPTIONS = [
  { label: "1 min/s", value: 1 / 1440 },
  { label: "10 min/s", value: 10 / 1440 },
  { label: "1 hr/s", value: 1 / 24 },
  { label: "6 hr/s", value: 0.25 },
  { label: "1 day/s", value: 1 },
  { label: "7 days/s", value: 7 },
  { label: "30 days/s", value: 30 },
  { label: "365 days/s", value: 365 },
];

function jdToDisplay(jd: number): string {
  const ms = (jd - 2440587.5) * 86400000;
  const d = new Date(ms);
  return d.toISOString().slice(0, 19).replace("T", " ") + " UTC";
}

const styles = {
  bar: {
    display: "flex",
    alignItems: "center",
    gap: "12px",
    padding: "6px 16px",
    background: "#111118",
    borderBottom: "1px solid #2a2a35",
    fontSize: "13px",
  },
  playBtn: (playing: boolean) => ({
    width: "32px",
    height: "28px",
    background: playing ? "#445" : "#2a4a6a",
    border: "1px solid #555",
    borderRadius: "4px",
    color: "#e0e0e0",
    fontSize: "14px",
    cursor: "pointer",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
  }),
  select: {
    background: "#1e1e28",
    border: "1px solid #333",
    borderRadius: "4px",
    color: "#e0e0e0",
    padding: "4px 6px",
    fontSize: "13px",
  },
  date: {
    color: "#8ab4f8",
    fontFamily: "monospace",
    fontSize: "14px",
    minWidth: "100px",
  },
};

export function PlaybackBar({
  playing,
  onTogglePlay,
  speed,
  onSpeedChange,
  epochJd,
  children,
}: PlaybackBarProps) {
  return (
    <div style={styles.bar}>
      <button
        type="button"
        style={styles.playBtn(playing)}
        onClick={onTogglePlay}
        title={playing ? "Pause" : "Play"}
      >
        {playing ? "\u275A\u275A" : "\u25B6"}
      </button>

      <span style={{ color: "#888" }}>Speed:</span>
      <select
        style={styles.select}
        value={speed}
        onChange={(e) => onSpeedChange(Number(e.target.value))}
      >
        {SPEED_OPTIONS.map((opt) => (
          <option key={opt.value} value={opt.value}>
            {opt.label}
          </option>
        ))}
      </select>

      <span style={styles.date}>{jdToDisplay(epochJd)}</span>
      {children}
    </div>
  );
}
