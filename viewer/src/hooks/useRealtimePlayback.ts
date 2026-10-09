import type { TimeRange } from "@sksat/uneri";
import { useCallback, useEffect, useRef, useState } from "react";
import type { OrbitPoint } from "../orbit.js";
import type { TrailBuffer } from "../utils/TrailBuffer.js";

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

/** The span of sim time the slider covers, and where on it the view is. */
export interface PlaybackTimeline {
  start: number;
  end: number;
  fraction: number;
}

/**
 * The slider's span and the view's place on it.
 *
 * `frozenEnd` is null while Live: the span then runs to the newest state.
 * Outside Live it is the newest state's time when the view left Live, so the
 * thumb is measured against a fixed span: the server keeps sending states
 * while the view is paused or replaying, and measured against those the thumb
 * slid back on its own. Playback past the frozen end stretches it, never
 * beyond what the buffers hold. A frozen end the buffers have dropped (older
 * than their oldest point) is let go of, so the span never runs backwards.
 */
export function computePlaybackTimeline(
  tMin: number,
  tMax: number,
  currentTime: number,
  frozenEnd: number | null,
): PlaybackTimeline {
  const frozen = frozenEnd !== null && frozenEnd >= tMin ? frozenEnd : null;
  const end = frozen === null ? tMax : Math.min(tMax, Math.max(frozen, currentTime));
  const span = end - tMin;
  const fraction = span > 0 ? Math.min(1, Math.max(0, (currentTime - tMin) / span)) : 1;
  return { start: tMin, end, fraction };
}

/**
 * The newest time any buffer holds, or 0 when none holds a point. Starts below
 * every time, so a recording whose times are all negative ends where it does.
 */
export function newestTime(trailBuffers: Map<string, TrailBuffer>): number {
  let tMax = Number.NEGATIVE_INFINITY;
  for (const buf of trailBuffers.values()) {
    if (buf.latest) tMax = Math.max(tMax, buf.latest.t);
  }
  return tMax === Number.NEGATIVE_INFINITY ? 0 : tMax;
}

/** The part of the playback state one animation frame can change. */
export interface PlaybackStep {
  mode: RealtimeMode;
  currentTime: number;
  /** See {@link computePlaybackTimeline}. */
  frozenEnd: number | null;
}

/**
 * Advance playback by one animation frame.
 *
 * Only a playing view moves: by `elapsed` wall seconds times `speed`. Reaching
 * `tMax` (the newest point the buffers hold) ends the replay. A stream then
 * follows live data; a source that cannot be followed live (a loaded file,
 * which has nothing newer) pauses at its end with the span frozen there.
 */
export function stepPlayback(
  state: PlaybackStep,
  elapsed: number,
  speed: number,
  tMax: number,
  canFollowLive: boolean,
): PlaybackStep {
  if (state.mode !== "playing") return state;
  const currentTime = state.currentTime + elapsed * speed;
  if (currentTime >= tMax) {
    return canFollowLive
      ? { mode: "live", currentTime: tMax, frozenEnd: null }
      : { mode: "paused", currentTime: tMax, frozenEnd: tMax };
  }
  // Ratchet, so seeking back after this keeps the span it showed.
  const frozenEnd =
    state.frozenEnd !== null && currentTime > state.frozenEnd ? currentTime : state.frozenEnd;
  return { mode: "playing", currentTime, frozenEnd };
}

/** What the buffers hold, as far as a frame's sync is concerned. */
export interface BufferRevision {
  /** Points across every buffer. */
  totalLength: number;
  /** `generation` summed across every buffer: bumped by a clear or a trim, so
   * a buffer replaced by one of the same length still counts as a change. */
  totalGeneration: number;
}

/** The revision of the buffers as they are now. */
export function bufferRevision(trailBuffers: Map<string, TrailBuffer>): BufferRevision {
  let totalLength = 0;
  let totalGeneration = 0;
  for (const buf of trailBuffers.values()) {
    totalLength += buf.length;
    totalGeneration += buf.generation;
  }
  return { totalLength, totalGeneration };
}

/**
 * Whether an animation frame recomputes the playback snapshot.
 *
 * A playing view's time moves, so it syncs every frame, and so does the frame
 * that changed the mode (`modeBefore` → `mode`: playback reaching the end pauses
 * a file and makes a stream live, with no new point). A live or paused view
 * otherwise changes only when the buffers do (`buffers` differs from the
 * revision the last sync saw, or there was none); a seek, a play/pause or a
 * speed change syncs on its own. A loaded file rests paused, and syncing it
 * every frame re-rendered the app at the display's refresh rate.
 */
export function shouldSyncFrame(
  modeBefore: RealtimeMode,
  mode: RealtimeMode,
  buffers: BufferRevision,
  lastSynced: BufferRevision | null,
): boolean {
  if (buffers.totalLength === 0) return false;
  return (
    mode === "playing" ||
    mode !== modeBefore ||
    lastSynced === null ||
    buffers.totalLength !== lastSynced.totalLength ||
    buffers.totalGeneration !== lastSynced.totalGeneration
  );
}

export interface RealtimePlaybackSnapshot {
  isLive: boolean;
  isPlaying: boolean;
  /** Sim time the view shows, in seconds since the epoch. */
  currentTime: number;
  fraction: number;
  /** Sim time at the slider's right end, in seconds since the epoch. */
  timelineEnd: number;
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
export interface RealtimePlaybackOptions {
  /** Initial mode: "live" for streaming sources, "paused" for file sources. */
  defaultMode?: "live" | "paused";
  /**
   * Whether the view can follow live data (default true). False for a loaded
   * file: playback that reaches its end pauses there instead of going live.
   * Read on every frame, so it can change while the hook is mounted.
   */
  canFollowLive?: boolean;
}

export function useRealtimePlayback(
  trailBuffers: Map<string, TrailBuffer>,
  terminatedSatellites: Set<string> = new Set(),
  timeRange: TimeRange = null,
  options?: RealtimePlaybackOptions,
) {
  const defaultMode = options?.defaultMode ?? "live";
  const canFollowLiveRef = useRef(true);
  canFollowLiveRef.current = options?.canFollowLive ?? true;
  const modeRef = useRef<RealtimeMode>(defaultMode);
  const currentTimeRef = useRef(0);
  // The slider's end while the view is out of Live; see computePlaybackTimeline.
  const frozenEndRef = useRef<number | null>(null);
  const speedRef = useRef(1);
  const rafRef = useRef(0);
  const prevTimeRef = useRef(0);

  const [snapshot, setSnapshot] = useState<RealtimePlaybackSnapshot>({
    isLive: defaultMode === "live",
    isPlaying: false,
    currentTime: 0,
    fraction: 1,
    timelineEnd: 0,
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

    const timeline = computePlaybackTimeline(
      tMin,
      tMax,
      currentTime,
      mode === "live" ? null : frozenEndRef.current,
    );

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
      fraction: timeline.fraction,
      timelineEnd: timeline.end,
      speed: speedRef.current,
      satellitePositions: positions,
      trailVisibleCounts: visibleCounts,
      trailDrawStarts: drawStarts,
      satellitePosition: firstPos,
      trailVisibleCount: firstVc,
    });
  }, [trailBuffers, terminatedSatellites, timeRange]);

  // The buffers as the last frame sync saw them, to skip a sync nothing calls
  // for. Reset when inputs (timeRange, terminatedSatellites) change so that
  // stale snapshot values are refreshed even without new data arriving.
  const lastSyncedRef = useRef<BufferRevision | null>(null);
  useEffect(() => {
    lastSyncedRef.current = null;
  }, [timeRange, terminatedSatellites]);

  // Animation loop
  useEffect(() => {
    const tick = (time: number) => {
      const dt = prevTimeRef.current ? (time - prevTimeRef.current) / 1000 : 0;
      prevTimeRef.current = time;

      let tMax = -Infinity;
      for (const buf of trailBuffers.values()) {
        if (buf.latest) tMax = Math.max(tMax, buf.latest.t);
      }
      if (tMax === -Infinity) tMax = 0;
      const buffers = bufferRevision(trailBuffers);

      const modeBefore = modeRef.current;
      const next = stepPlayback(
        {
          mode: modeRef.current,
          currentTime: currentTimeRef.current,
          frozenEnd: frozenEndRef.current,
        },
        dt,
        speedRef.current,
        tMax,
        canFollowLiveRef.current,
      );
      modeRef.current = next.mode;
      currentTimeRef.current = next.currentTime;
      frozenEndRef.current = next.frozenEnd;

      // The point count catches multi-satellite updates where a lagging
      // satellite advances without changing the global tMax.
      if (shouldSyncFrame(modeBefore, modeRef.current, buffers, lastSyncedRef.current)) {
        lastSyncedRef.current = buffers;
        syncState();
      }

      rafRef.current = requestAnimationFrame(tick);
    };

    rafRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafRef.current);
  }, [trailBuffers, syncState]);

  /** Pause at the newest point the buffers hold, with the span frozen there. */
  const holdAtEnd = useCallback(() => {
    const tMax = newestTime(trailBuffers);
    currentTimeRef.current = tMax;
    frozenEndRef.current = tMax;
    modeRef.current = "paused";
  }, [trailBuffers]);

  /** Pause at the end of what the buffers hold, as a loaded file rests. */
  const pauseAtEnd = useCallback(() => {
    holdAtEnd();
    syncState();
  }, [holdAtEnd, syncState]);

  const togglePlayPause = useCallback(() => {
    const mode = modeRef.current;
    if (mode === "live") {
      holdAtEnd();
    } else if (mode === "paused") {
      modeRef.current = "playing";
    } else {
      modeRef.current = "paused";
    }
    syncState();
  }, [holdAtEnd, syncState]);

  const goLive = useCallback(() => {
    modeRef.current = "live";
    frozenEndRef.current = null;
    syncState();
  }, [syncState]);

  const seekToFraction = useCallback(
    (fraction: number) => {
      let tMin = Infinity;
      let tMax = -Infinity;
      for (const buf of trailBuffers.values()) {
        const pts = buf.getAll();
        if (pts.length > 0) tMin = Math.min(tMin, pts[0].t);
        if (buf.latest) tMax = Math.max(tMax, buf.latest.t);
      }
      if (tMin === Infinity) tMin = 0;
      if (tMax === -Infinity) tMax = 0;

      // A seek from Live freezes the span it was made on; one made outside
      // Live maps onto the span the slider is showing.
      if (modeRef.current === "live") frozenEndRef.current = tMax;
      const { start, end } = computePlaybackTimeline(
        tMin,
        tMax,
        currentTimeRef.current,
        frozenEndRef.current,
      );
      currentTimeRef.current = start + fraction * (end - start);
      // Freeze the span the seek was made on, which replaces a frozen end the
      // buffers have dropped since.
      frozenEndRef.current = end;

      if (modeRef.current === "live" || modeRef.current === "playing") {
        modeRef.current = "paused";
      }
      syncState();
    },
    [trailBuffers, syncState],
  );

  const setSpeed = useCallback(
    (speed: number) => {
      speedRef.current = Math.max(0.1, speed);
      syncState();
    },
    [syncState],
  );

  return {
    snapshot,
    togglePlayPause,
    goLive,
    pauseAtEnd,
    seekToFraction,
    setSpeed,
  };
}
