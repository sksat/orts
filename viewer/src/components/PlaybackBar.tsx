import { useCallback, useState } from "react";
import type { Pacing } from "../protocol/generated/Pacing.js";
import { jd_to_utc_string } from "../wasm/arikaInit.js";
import styles from "./PlaybackBar.module.css";
import { playbackModeLabel } from "./playbackModeLabel.js";

interface PlaybackBarProps {
  isPlaying: boolean;
  fraction: number;
  /** Sim time the view shows, in seconds since the epoch. */
  currentTime: number;
  /** Sim time at the slider's right end, in seconds since the epoch. */
  timelineEnd: number;
  onTogglePlayPause: () => void;
  onSeekFraction: (fraction: number) => void;
  /** Replay speed: how fast Play steps through the history already received. */
  speed: number;
  onSpeedChange: (speed: number) => void;
  /** Whether the viewer is following live data (realtime mode only). */
  isLive?: boolean;
  /** Jump to latest data and follow (realtime mode only). */
  onGoLive?: () => void;
  /** Julian Date of the simulation epoch for absolute time display. */
  epochJd?: number | null;
  /** How fast the server runs the simulation, when it says (`info.pacing`). */
  serverPacing?: Pacing;
}

const SPEED_OPTIONS = [1, 2, 5, 10, 100];

/**
 * Format a time value in seconds to a human-readable string.
 * Shows minutes and seconds when >= 60s, otherwise just seconds.
 */
function formatTime(seconds: number): string {
  if (seconds < 60) {
    return `${seconds.toFixed(1)} s`;
  }
  const mins = Math.floor(seconds / 60);
  const secs = seconds % 60;
  return `${mins}m ${secs.toFixed(1)}s`;
}

/**
 * Playback controls bar component: play/pause button, speed selector,
 * time slider (scrubber), time display, and mode indicator.
 *
 * Works in both Replay and Realtime modes via callback props.
 * In Realtime mode, shows a "Live" button to resume following live data.
 */
export function PlaybackBar({
  isPlaying,
  fraction,
  currentTime,
  timelineEnd,
  onTogglePlayPause,
  onSeekFraction,
  speed,
  onSpeedChange,
  isLive,
  onGoLive,
  epochJd,
  serverPacing,
}: PlaybackBarProps) {
  const [isScrubbing, setIsScrubbing] = useState(false);

  const handlePlayPause = useCallback(() => {
    onTogglePlayPause();
  }, [onTogglePlayPause]);

  const handleSpeedChange = useCallback(
    (e: React.ChangeEvent<HTMLSelectElement>) => {
      onSpeedChange(Number(e.target.value));
    },
    [onSpeedChange],
  );

  const handleSliderInput = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      setIsScrubbing(true);
      const f = Number(e.target.value) / 1000;
      onSeekFraction(f);
    },
    [onSeekFraction],
  );

  const handleSliderChange = useCallback(() => {
    setIsScrubbing(false);
  }, []);

  const sliderValue = isScrubbing ? undefined : Math.round(fraction * 1000);

  const isRealtimeMode = onGoLive != null;
  const modeLabel = playbackModeLabel(isRealtimeMode, isLive, isPlaying, serverPacing);
  // Live shows the newest state as it arrives; nothing is replayed, so the
  // replay speed has nothing to apply to.
  const replaySpeedInactive = isRealtimeMode && isLive === true;

  return (
    <div className={styles.playbackBar}>
      <div className={styles.sliderRow}>
        <input
          type="range"
          className={styles.timeSlider}
          data-testid="time-slider"
          min={0}
          max={1000}
          step={1}
          value={sliderValue}
          onChange={handleSliderInput}
          onMouseUp={handleSliderChange}
          onTouchEnd={handleSliderChange}
        />
      </div>
      <div className={styles.controlsRow}>
        <button
          className={styles.playPauseBtn}
          data-testid="play-pause-btn"
          onClick={handlePlayPause}
          title="Pauses or plays this view only; the server keeps simulating"
        >
          {isPlaying || isLive ? "Pause" : "Play"}
        </button>
        <label
          className={styles.speedLabel}
          title={
            replaySpeedInactive
              ? "Replay speed applies after leaving Live (Pause, or drag the slider)"
              : "How fast Play steps through the history already received (1x = 1 sim s per s)"
          }
        >
          Replay
          <select
            className={styles.speedSelect}
            data-testid="replay-speed-select"
            value={speed}
            onChange={handleSpeedChange}
            disabled={replaySpeedInactive}
          >
            {SPEED_OPTIONS.map((s) => (
              <option key={s} value={s}>
                {s}x
              </option>
            ))}
          </select>
        </label>
        <span className={styles.timeDisplay} data-testid="playback-time">
          {epochJd != null && <>{jd_to_utc_string(epochJd, currentTime)} | </>}
          T+{formatTime(currentTime)} / {formatTime(timelineEnd)}
        </span>
        {isRealtimeMode && (
          <button
            className={`${styles.liveBtn} ${isLive ? styles.active : ""}`}
            onClick={onGoLive}
            disabled={isLive}
          >
            Live
          </button>
        )}
        <span
          className={`${styles.modeIndicator} ${isLive ? styles.live : ""}`}
          data-testid="playback-mode"
        >
          {modeLabel}
        </span>
      </div>
    </div>
  );
}
