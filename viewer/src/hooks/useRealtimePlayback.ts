import { useRef, useState, useCallback, useEffect } from "react";
import type { OrbitPoint } from "../orbit.js";
import type { TrailBuffer } from "../utils/TrailBuffer.js";

type RealtimeMode = "live" | "paused" | "playing";

export interface RealtimePlaybackSnapshot {
  isLive: boolean;
  isPlaying: boolean;
  currentTime: number;
  fraction: number;
  elapsedTime: number;
  totalDuration: number;
  speed: number;
  satellitePosition: OrbitPoint | null;
  trailVisibleCount: number;
}

/**
 * React hook for realtime playback with history scrubbing.
 *
 * State machine:
 *   Live ──pause/seek──→ Paused ──play──→ Playing ──catches up──→ Live
 *                        Paused ←──pause── Playing
 *                        Live   ←──goLive── Paused | Playing
 */
export function useRealtimePlayback(trailBuffer: TrailBuffer) {
  const modeRef = useRef<RealtimeMode>("live");
  const currentTimeRef = useRef(0);
  const speedRef = useRef(1);
  const rafRef = useRef(0);
  const prevTimeRef = useRef(0);

  const [snapshot, setSnapshot] = useState<RealtimePlaybackSnapshot>({
    isLive: true,
    isPlaying: false,
    currentTime: 0,
    fraction: 1,
    elapsedTime: 0,
    totalDuration: 0,
    speed: 1,
    satellitePosition: null,
    trailVisibleCount: 0,
  });

  const syncState = useCallback(() => {
    const pts = trailBuffer.getAll();
    const tMin = pts.length > 0 ? pts[0].t : 0;
    const tMax = trailBuffer.latest?.t ?? 0;
    const duration = tMax - tMin;
    const mode = modeRef.current;

    let currentTime: number;
    let position: OrbitPoint | null;
    let visibleCount: number;

    if (mode === "live") {
      currentTime = tMax;
      position = trailBuffer.latest;
      visibleCount = pts.length;
    } else {
      currentTime = currentTimeRef.current;
      position = trailBuffer.interpolateAt(currentTime);
      const idx = trailBuffer.indexBefore(currentTime);
      visibleCount = idx + 2; // +2 matches replay mode convention
    }

    const fraction = duration > 0
      ? Math.min(1, Math.max(0, (currentTime - tMin) / duration))
      : 1;

    setSnapshot({
      isLive: mode === "live",
      isPlaying: mode === "playing",
      currentTime,
      fraction,
      elapsedTime: currentTime - tMin,
      totalDuration: duration,
      speed: speedRef.current,
      satellitePosition: position,
      trailVisibleCount: visibleCount,
    });
  }, [trailBuffer]);

  // Animation loop
  useEffect(() => {
    const tick = (time: number) => {
      const dt = prevTimeRef.current ? (time - prevTimeRef.current) / 1000 : 0;
      prevTimeRef.current = time;

      if (modeRef.current === "playing") {
        const tMax = trailBuffer.latest?.t ?? 0;
        currentTimeRef.current += dt * speedRef.current;

        // Auto-transition to live when catching up
        if (currentTimeRef.current >= tMax) {
          currentTimeRef.current = tMax;
          modeRef.current = "live";
        }
      }

      // Always sync state in RAF to keep slider/position updated
      // (in live mode, tMax grows as new data arrives)
      if (trailBuffer.length > 0) {
        syncState();
      }

      rafRef.current = requestAnimationFrame(tick);
    };

    rafRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafRef.current);
  }, [trailBuffer, syncState]);

  const togglePlayPause = useCallback(() => {
    const mode = modeRef.current;
    if (mode === "live") {
      // Pause at current (latest) time
      currentTimeRef.current = trailBuffer.latest?.t ?? 0;
      modeRef.current = "paused";
    } else if (mode === "paused") {
      modeRef.current = "playing";
    } else {
      // playing → paused
      modeRef.current = "paused";
    }
    syncState();
  }, [trailBuffer, syncState]);

  const goLive = useCallback(() => {
    modeRef.current = "live";
    syncState();
  }, [syncState]);

  const seekToFraction = useCallback((fraction: number) => {
    const pts = trailBuffer.getAll();
    const tMin = pts.length > 0 ? pts[0].t : 0;
    const tMax = trailBuffer.latest?.t ?? 0;
    const duration = tMax - tMin;

    currentTimeRef.current = tMin + fraction * duration;

    // Seeking always pauses (breaks out of live/playing)
    if (modeRef.current === "live" || modeRef.current === "playing") {
      modeRef.current = "paused";
    }
    syncState();
  }, [trailBuffer, syncState]);

  const setSpeed = useCallback((speed: number) => {
    speedRef.current = Math.max(0.1, speed);
    syncState();
  }, [syncState]);

  return {
    snapshot,
    togglePlayPause,
    goLive,
    seekToFraction,
    setSpeed,
  };
}
