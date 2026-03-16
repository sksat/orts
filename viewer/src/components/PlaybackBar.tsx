import { useCallback, useState } from "react";
import { jd_to_utc_string } from "../wasm/kanameInit.js";

interface PlaybackBarProps {
  isPlaying: boolean;
  fraction: number;
  elapsedTime: number;
  totalDuration: number;
  onTogglePlayPause: () => void;
  onSeekFraction: (fraction: number) => void;
  onSpeedChange: (speed: number) => void;
  /** Whether the viewer is following live data (realtime mode only). */
  isLive?: boolean;
  /** Jump to latest data and follow (realtime mode only). */
  onGoLive?: () => void;
  /** Julian Date of the simulation epoch for absolute time display. */
  epochJd?: number | null;
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
  elapsedTime,
  totalDuration,
  onTogglePlayPause,
  onSeekFraction,
  onSpeedChange,
  isLive,
  onGoLive,
  epochJd,
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
  const modeLabel = isRealtimeMode
    ? isLive
      ? "Live"
      : isPlaying
        ? "Playing"
        : "Paused"
    : "Replay";

  return (
    <div className="playback-bar visible">
      <div className="playback-slider-row">
        <input
          type="range"
          className="time-slider"
          min={0}
          max={1000}
          step={1}
          value={sliderValue}
          onChange={handleSliderInput}
          onMouseUp={handleSliderChange}
          onTouchEnd={handleSliderChange}
        />
      </div>
      <div className="playback-controls-row">
        <button className="play-pause-btn" onClick={handlePlayPause}>
          {isPlaying || isLive ? "Pause" : "Play"}
        </button>
        <select className="speed-select" defaultValue="1" onChange={handleSpeedChange}>
          {SPEED_OPTIONS.map((s) => (
            <option key={s} value={s}>
              {s}x
            </option>
          ))}
        </select>
        <span className="time-display">
          {epochJd != null && <>{jd_to_utc_string(epochJd, elapsedTime)} | </>}
          T+{formatTime(elapsedTime)} / {formatTime(totalDuration)}
        </span>
        {isRealtimeMode && (
          <button
            className={`live-btn ${isLive ? "active" : ""}`}
            onClick={onGoLive}
            disabled={isLive}
          >
            Live
          </button>
        )}
        <span className={`mode-indicator ${isLive ? "live" : ""}`}>{modeLabel}</span>
      </div>
    </div>
  );
}
