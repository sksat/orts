import { useRef, useState, useCallback, useEffect } from "react";
import type { OrbitPoint } from "../orbit.js";
import type { TrailBuffer } from "../utils/TrailBuffer.js";
import type { TimeRange } from "uneri";

type RealtimeMode = "live" | "paused" | "playing";

/**
 * Compute the synchronization time for live mode.
 * Returns the minimum `latest.t` across all non-terminated satellite buffers,
 * so surviving satellites drive the time forward when a peer terminates.
 */
export function computeLiveSyncTime(
  trailBuffers: Map<string, TrailBuffer>,
  terminatedSatellites: Set<string>,
): number {
  let syncTime = Infinity;
  for (const [satId, buf] of trailBuffers) {
    if (terminatedSatellites.has(satId)) continue;
    if (buf.latest) syncTime = Math.min(syncTime, buf.latest.t);
  }
  return syncTime;
}

/**
 * Compute per-satellite draw start indices for time-range clipping.
 * Returns a Map from satellite ID to the index at which the trail should start drawing.
 * When timeRange is null, all starts are 0 (no clipping).
 */
export function computeTrailDrawStarts(
  trailBuffers: Map<string, TrailBuffer>,
  currentTime: number,
  timeRange: TimeRange,
): Map<string, number> {
  const starts = new Map<string, number>();
  for (const [satId, buf] of trailBuffers) {
    if (timeRange == null) {
      starts.set(satId, 0);
    } else {
      const startT = currentTime - timeRange;
      const idx = buf.indexBefore(startT);
      // indexBefore returns -1 when all points are after startT
      starts.set(satId, Math.max(0, idx));
    }
  }
  return starts;
}

export interface RealtimePlaybackSnapshot {
  isLive: boolean;
  isPlaying: boolean;
  currentTime: number;
  fraction: number;
  elapsedTime: number;
  totalDuration: number;
  speed: number;
  /** Per-satellite positions (multi-satellite mode). */
  satellitePositions: Map<string, OrbitPoint | null>;
  /** Per-satellite trail visible counts (multi-satellite mode). */
  trailVisibleCounts: Map<string, number>;
  /** Per-satellite draw start indices for time-range clipping. */
  trailDrawStarts: Map<string, number>;
  /** First satellite position for backward compat. */
  satellitePosition: OrbitPoint | null;
  /** First satellite trail visible count for backward compat. */
  trailVisibleCount: number;
}

/**
 * React hook for realtime playback with history scrubbing.
 * Supports multiple TrailBuffers (one per satellite).
 *
 * State machine:
 *   Live ──pause/seek──→ Paused ──play──→ Playing ──catches up──→ Live
 *                        Paused ←──pause── Playing
 *                        Live   ←──goLive── Paused | Playing
 */
export function useRealtimePlayback(
  trailBuffers: Map<string, TrailBuffer>,
  terminatedSatellites: Set<string> = new Set(),
  timeRange: TimeRange = null,
) {
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
    satellitePositions: new Map(),
    trailVisibleCounts: new Map(),
    trailDrawStarts: new Map(),
    satellitePosition: null,
    trailVisibleCount: 0,
  });

  const syncState = useCallback(() => {
    // Compute unified timeline across all satellite buffers
    let tMin = Infinity;
    let tMax = -Infinity;
    let totalLength = 0;

    for (const buf of trailBuffers.values()) {
      if (buf.length === 0) continue;
      const pts = buf.getAll();
      if (pts.length > 0) {
        tMin = Math.min(tMin, pts[0].t);
      }
      if (buf.latest) {
        tMax = Math.max(tMax, buf.latest.t);
      }
      totalLength += buf.length;
    }

    if (totalLength === 0) return;
    if (tMin === Infinity) tMin = 0;
    if (tMax === -Infinity) tMax = 0;

    const duration = tMax - tMin;
    const mode = modeRef.current;

    let currentTime: number;
    if (mode === "live") {
      // Synchronize: use min of all active (non-terminated) satellites' latest t.
      // Terminated satellites are excluded so the surviving ones keep advancing.
      const syncTime = computeLiveSyncTime(trailBuffers, terminatedSatellites);
      currentTime = syncTime === Infinity ? tMax : syncTime;
    } else {
      currentTime = currentTimeRef.current;
    }

    const fraction = duration > 0
      ? Math.min(1, Math.max(0, (currentTime - tMin) / duration))
      : 1;

    // Compute per-satellite positions and visible counts
    const positions = new Map<string, OrbitPoint | null>();
    const visibleCounts = new Map<string, number>();

    for (const [satId, buf] of trailBuffers) {
      positions.set(satId, buf.interpolateAt(currentTime));
      if (mode === "live") {
        visibleCounts.set(satId, buf.length);
      } else {
        const idx = buf.indexBefore(currentTime);
        visibleCounts.set(satId, idx + 2);
      }
    }

    // Compute per-satellite draw start indices for time-range clipping
    const drawStarts = computeTrailDrawStarts(trailBuffers, currentTime, timeRange);

    // Backward compat: first satellite
    const firstId = trailBuffers.keys().next().value;
    const firstPos = firstId != null ? (positions.get(firstId) ?? null) : null;
    const firstVc = firstId != null ? (visibleCounts.get(firstId) ?? 0) : 0;

    setSnapshot({
      isLive: mode === "live",
      isPlaying: mode === "playing",
      currentTime,
      fraction,
      elapsedTime: currentTime - tMin,
      totalDuration: duration,
      speed: speedRef.current,
      satellitePositions: positions,
      trailVisibleCounts: visibleCounts,
      trailDrawStarts: drawStarts,
      satellitePosition: firstPos,
      trailVisibleCount: firstVc,
    });
  }, [trailBuffers, terminatedSatellites, timeRange]);

  // Animation loop
  useEffect(() => {
    const tick = (time: number) => {
      const dt = prevTimeRef.current ? (time - prevTimeRef.current) / 1000 : 0;
      prevTimeRef.current = time;

      if (modeRef.current === "playing") {
        let tMax = -Infinity;
        for (const buf of trailBuffers.values()) {
          if (buf.latest) tMax = Math.max(tMax, buf.latest.t);
        }
        if (tMax === -Infinity) tMax = 0;

        currentTimeRef.current += dt * speedRef.current;
        if (currentTimeRef.current >= tMax) {
          currentTimeRef.current = tMax;
          modeRef.current = "live";
        }
      }

      // Always sync in RAF to keep slider/position updated
      let totalLength = 0;
      for (const buf of trailBuffers.values()) {
        totalLength += buf.length;
      }
      if (totalLength > 0) {
        syncState();
      }

      rafRef.current = requestAnimationFrame(tick);
    };

    rafRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafRef.current);
  }, [trailBuffers, syncState]);

  const togglePlayPause = useCallback(() => {
    const mode = modeRef.current;
    if (mode === "live") {
      // Find max t across all buffers
      let tMax = 0;
      for (const buf of trailBuffers.values()) {
        if (buf.latest) tMax = Math.max(tMax, buf.latest.t);
      }
      currentTimeRef.current = tMax;
      modeRef.current = "paused";
    } else if (mode === "paused") {
      modeRef.current = "playing";
    } else {
      modeRef.current = "paused";
    }
    syncState();
  }, [trailBuffers, syncState]);

  const goLive = useCallback(() => {
    modeRef.current = "live";
    syncState();
  }, [syncState]);

  const seekToFraction = useCallback((fraction: number) => {
    let tMin = Infinity;
    let tMax = -Infinity;
    for (const buf of trailBuffers.values()) {
      const pts = buf.getAll();
      if (pts.length > 0) tMin = Math.min(tMin, pts[0].t);
      if (buf.latest) tMax = Math.max(tMax, buf.latest.t);
    }
    if (tMin === Infinity) tMin = 0;
    if (tMax === -Infinity) tMax = 0;
    const duration = tMax - tMin;

    currentTimeRef.current = tMin + fraction * duration;

    if (modeRef.current === "live" || modeRef.current === "playing") {
      modeRef.current = "paused";
    }
    syncState();
  }, [trailBuffers, syncState]);

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
