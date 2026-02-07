import { useCallback, useState } from "react";
import type { PlaybackController } from "../playback.js";

interface PlaybackBarProps {
  playback: PlaybackController;
  isPlaying: boolean;
  fraction: number;
  elapsedTime: number;
  totalDuration: number;
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
 */
export function PlaybackBar({
  playback,
  isPlaying,
  fraction,
  elapsedTime,
  totalDuration,
}: PlaybackBarProps) {
  const [isScrubbing, setIsScrubbing] = useState(false);

  const handlePlayPause = useCallback(() => {
    playback.togglePlayPause();
  }, [playback]);

  const handleSpeedChange = useCallback(
    (e: React.ChangeEvent<HTMLSelectElement>) => {
      playback.setSpeed(Number(e.target.value));
    },
    [playback]
  );

  const handleSliderInput = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      setIsScrubbing(true);
      const f = Number(e.target.value) / 1000;
      playback.seekToFraction(f);
    },
    [playback]
  );

  const handleSliderChange = useCallback(() => {
    setIsScrubbing(false);
  }, []);

  const sliderValue = isScrubbing ? undefined : Math.round(fraction * 1000);

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
          {isPlaying ? "Pause" : "Play"}
        </button>
        <select
          className="speed-select"
          defaultValue="1"
          onChange={handleSpeedChange}
        >
          {SPEED_OPTIONS.map((s) => (
            <option key={s} value={s}>
              {s}x
            </option>
          ))}
        </select>
        <span className="time-display">
          T+{formatTime(elapsedTime)} / {formatTime(totalDuration)}
        </span>
        <span className="mode-indicator">Replay</span>
      </div>
    </div>
  );
}
