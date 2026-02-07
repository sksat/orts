import { useRef, useState, useCallback, useEffect } from "react";
import { PlaybackController } from "../playback.js";
import { OrbitPoint } from "../orbit.js";

/** Snapshot of playback state exposed to React components. */
export interface PlaybackSnapshot {
  isPlaying: boolean;
  fraction: number;
  elapsedTime: number;
  totalDuration: number;
  satellitePosition: OrbitPoint | null;
  trailVisibleCount: number;
}

/**
 * React hook that wraps a PlaybackController instance.
 *
 * Creates and manages the controller for a set of orbit points.
 * Uses requestAnimationFrame for smooth per-frame updates and
 * exposes play/pause/seek/speed controls plus current state.
 */
export function usePlayback(points: OrbitPoint[] | null) {
  const controllerRef = useRef<PlaybackController | null>(null);
  const rafRef = useRef<number>(0);
  const prevTimeRef = useRef<number>(0);

  const [snapshot, setSnapshot] = useState<PlaybackSnapshot>({
    isPlaying: false,
    fraction: 0,
    elapsedTime: 0,
    totalDuration: 0,
    satellitePosition: null,
    trailVisibleCount: 1,
  });

  // Sync React state from the controller
  const syncState = useCallback(() => {
    const ctrl = controllerRef.current;
    if (!ctrl) return;
    setSnapshot({
      isPlaying: ctrl.isPlaying,
      fraction: ctrl.fraction,
      elapsedTime: ctrl.elapsedTime,
      totalDuration: ctrl.totalDuration,
      satellitePosition: ctrl.getCurrentState(),
      trailVisibleCount: ctrl.getCurrentTrailIndex() + 2,
    });
  }, []);

  // (Re)create controller when points change
  useEffect(() => {
    if (!points || points.length === 0) {
      controllerRef.current = null;
      setSnapshot({
        isPlaying: false,
        fraction: 0,
        elapsedTime: 0,
        totalDuration: 0,
        satellitePosition: null,
        trailVisibleCount: 1,
      });
      return;
    }

    const ctrl = new PlaybackController(points);
    ctrl.onChange = syncState;
    controllerRef.current = ctrl;
    syncState();
  }, [points, syncState]);

  // Animation loop: advance playback each frame
  useEffect(() => {
    const tick = (time: number) => {
      const ctrl = controllerRef.current;
      if (ctrl) {
        const dt = prevTimeRef.current ? (time - prevTimeRef.current) / 1000 : 0;
        prevTimeRef.current = time;

        const changed = ctrl.update(dt);
        if (changed) {
          syncState();
        }
      } else {
        prevTimeRef.current = time;
      }

      rafRef.current = requestAnimationFrame(tick);
    };

    rafRef.current = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(rafRef.current);
    };
  }, [syncState]);

  // Exposed control functions
  const togglePlayPause = useCallback(() => {
    controllerRef.current?.togglePlayPause();
  }, []);

  const setSpeed = useCallback((speed: number) => {
    controllerRef.current?.setSpeed(speed);
  }, []);

  const seekToFraction = useCallback((fraction: number) => {
    controllerRef.current?.seekToFraction(fraction);
  }, []);

  return {
    controller: controllerRef.current,
    snapshot,
    togglePlayPause,
    setSpeed,
    seekToFraction,
  };
}
