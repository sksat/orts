/**
 * Pure event dispatch logic for routing SourceEvents into buffers.
 *
 * No React dependency. No Worker dependency. Fully testable.
 *
 * The dispatcher mutates the RuntimeBuffers and RuntimeState objects
 * passed to it, which the React hook (useSourceRuntime) syncs back
 * to React state after each event.
 */

import type { OrbitPoint } from "../orbit.js";
import type { TrailBuffer } from "../utils/TrailBuffer.js";
import type { SimInfo, SourceConnectionState, SourceEvent, SourceId } from "./types.js";

// ---------------------------------------------------------------------------
// Chart row conversion
// ---------------------------------------------------------------------------

const RAD_TO_DEG = 180.0 / Math.PI;

export function orbitPointToChartRow(p: OrbitPoint): Record<string, number> {
  const accelDrag = p.accel_drag ?? 0;
  const accelSrp = p.accel_srp ?? 0;
  const accelSun = p.accel_third_body_sun ?? 0;
  const accelMoon = p.accel_third_body_moon ?? 0;
  return {
    t: p.t,
    altitude: p.altitude ?? 0,
    energy: p.specific_energy ?? 0,
    angular_momentum: p.angular_momentum ?? 0,
    velocity: p.velocity_mag ?? 0,
    a: p.a,
    e: p.e,
    inc_deg: p.inc * RAD_TO_DEG,
    raan_deg: p.raan * RAD_TO_DEG,
    accel_gravity: p.accel_gravity ?? 0,
    accel_drag: accelDrag,
    accel_srp: accelSrp,
    accel_third_body_sun: accelSun,
    accel_third_body_moon: accelMoon,
    accel_perturbation_total: accelDrag + accelSrp + accelSun + accelMoon,
  };
}

// ---------------------------------------------------------------------------
// Buffer / state interfaces
// ---------------------------------------------------------------------------

/** Minimal ChartBuffer interface used by the dispatcher. */
export interface ChartBufferLike {
  push(values: Record<string, number>): void;
  clear(): void;
}

/** Minimal IngestBuffer interface used by the dispatcher. */
export interface IngestBufferLike<T> {
  push(point: T): void;
  markRebuild(points: T[]): void;
  readonly latestT: number;
}

export interface RuntimeBuffers {
  trailBuffers: Map<string, TrailBuffer>;
  ingestBuffers: Map<string, IngestBufferLike<OrbitPoint>>;
  chartBuffer: ChartBufferLike;
  detailBuffer: OrbitPoint[];
  streamingCount: number;
  /** Tracks whether a chunked load has started (to clear stale data on first chunk). */
  chunkLoadStarted: boolean;
}

export type ServerState = "unknown" | "idle" | "running" | "paused";

export interface RuntimeState {
  simInfo: SimInfo | null;
  serverState: ServerState;
  terminatedSatellites: Set<string>;
  connectionState: SourceConnectionState;
  textureRevision: number;
}

// ---------------------------------------------------------------------------
// Buffer helpers
// ---------------------------------------------------------------------------

function getOrCreate<T>(map: Map<string, T>, id: string, factory: () => T): T {
  let item = map.get(id);
  if (!item) {
    item = factory();
    map.set(id, item);
  }
  return item;
}

/** Factory for TrailBuffer — imported dynamically to avoid circular deps. */
let trailBufferFactory: (id: string) => TrailBuffer;

export function setTrailBufferFactory(factory: (id: string) => TrailBuffer): void {
  trailBufferFactory = factory;
}

/** Factory for IngestBuffer — injected to avoid uneri Worker dependency in tests. */
let ingestBufferFactory: (id: string) => IngestBufferLike<OrbitPoint>;

export function setIngestBufferFactory(
  factory: (id: string) => IngestBufferLike<OrbitPoint>,
): void {
  ingestBufferFactory = factory;
}

function getOrCreateTrailBuffer(map: Map<string, TrailBuffer>, id: string): TrailBuffer {
  return getOrCreate(map, id, () => trailBufferFactory(id));
}

function getOrCreateIngestBuffer(
  map: Map<string, IngestBufferLike<OrbitPoint>>,
  id: string,
): IngestBufferLike<OrbitPoint> {
  return getOrCreate(map, id, () => ingestBufferFactory(id));
}

// ---------------------------------------------------------------------------
// Event dispatcher
// ---------------------------------------------------------------------------

/**
 * Create an event dispatcher that routes SourceEvents into buffers/state.
 * Ignores events from non-active sources (stale event discard).
 */
export function createEventDispatcher(
  buffers: RuntimeBuffers,
  state: RuntimeState,
  activeSourceId: SourceId | null,
): (sourceId: SourceId, event: SourceEvent) => void {
  return (sourceId: SourceId, event: SourceEvent) => {
    if (activeSourceId !== null && sourceId !== activeSourceId) {
      return;
    }

    switch (event.kind) {
      case "info":
        state.simInfo = event.info;
        state.serverState = "running";
        // Don't override "loading" state (file sources stay loading until complete)
        if (state.connectionState !== "loading") {
          state.connectionState = "connected";
        }
        break;

      case "state": {
        const id = event.point.entityPath ?? "default";
        getOrCreateIngestBuffer(buffers.ingestBuffers, id).push(event.point);
        getOrCreateTrailBuffer(buffers.trailBuffers, id).push(event.point);
        buffers.chartBuffer.push(orbitPointToChartRow(event.point));
        buffers.streamingCount++;
        break;
      }

      case "history": {
        const byId = new Map<string, OrbitPoint[]>();
        // Clear existing trail data to avoid stale points from prior sessions
        for (const buf of buffers.trailBuffers.values()) {
          buf.clear();
        }
        buffers.chartBuffer.clear();
        for (const point of event.points) {
          const id = point.entityPath ?? "default";
          let arr = byId.get(id);
          if (!arr) {
            arr = [];
            byId.set(id, arr);
          }
          arr.push(point);
          getOrCreateTrailBuffer(buffers.trailBuffers, id).push(point);
          buffers.chartBuffer.push(orbitPointToChartRow(point));
        }
        for (const [id, pts] of byId) {
          getOrCreateIngestBuffer(buffers.ingestBuffers, id).markRebuild(pts);
        }
        buffers.streamingCount = 0;
        break;
      }

      case "history-chunk": {
        // Clear stale data on the first chunk of a new load
        if (!buffers.chunkLoadStarted && event.points.length > 0) {
          for (const buf of buffers.trailBuffers.values()) buf.clear();
          buffers.chartBuffer.clear();
          buffers.chunkLoadStarted = true;
        }
        for (const point of event.points) {
          const id = point.entityPath ?? "default";
          getOrCreateTrailBuffer(buffers.trailBuffers, id).push(point);
          getOrCreateIngestBuffer(buffers.ingestBuffers, id).push(point);
          buffers.chartBuffer.push(orbitPointToChartRow(point));
        }
        if (event.done) {
          // If no chunks arrived (empty/invalid file), clear stale data
          if (!buffers.chunkLoadStarted) {
            for (const buf of buffers.trailBuffers.values()) buf.clear();
            buffers.chartBuffer.clear();
          }
          for (const [id, buf] of buffers.trailBuffers) {
            getOrCreateIngestBuffer(buffers.ingestBuffers, id).markRebuild(buf.getAll());
          }
          buffers.chunkLoadStarted = false; // reset for next load
        }
        break;
      }

      case "history-detail":
        for (const point of event.points) {
          buffers.detailBuffer.push(point);
        }
        break;

      case "history-detail-complete": {
        if (buffers.detailBuffer.length === 0) break;

        const detailPoints = buffers.detailBuffer;
        buffers.detailBuffer = [];

        // Collect recent streaming points from TrailBuffers.
        // NOTE: streamingCount is a global counter, not per-satellite.
        // In multi-sat sessions this may include tail points from satellites
        // that had no new streamed data. A per-satellite counter would fix
        // this, but matches the existing App.tsx behavior.
        const streamingPoints: OrbitPoint[] = [];
        for (const buf of buffers.trailBuffers.values()) {
          const allPts = buf.getAll();
          const safeCount = Math.min(buffers.streamingCount, allPts.length);
          streamingPoints.push(...allPts.slice(allPts.length - safeCount));
        }

        const combined = [...detailPoints, ...streamingPoints];
        combined.sort((a, b) => a.t - b.t);

        const bySatellite = new Map<string, OrbitPoint[]>();
        for (const p of combined) {
          const id = p.entityPath ?? "default";
          let arr = bySatellite.get(id);
          if (!arr) {
            arr = [];
            bySatellite.set(id, arr);
          }
          arr.push(p);
        }

        for (const [id, pts] of bySatellite) {
          getOrCreateTrailBuffer(buffers.trailBuffers, id).clear();
          getOrCreateTrailBuffer(buffers.trailBuffers, id).pushMany(pts);
          getOrCreateIngestBuffer(buffers.ingestBuffers, id).markRebuild(pts);
        }

        buffers.chartBuffer.clear();
        for (const point of combined) {
          buffers.chartBuffer.push(orbitPointToChartRow(point));
        }
        break;
      }

      case "terminated":
        state.terminatedSatellites = new Set(state.terminatedSatellites);
        state.terminatedSatellites.add(event.entityPath);
        break;

      case "server-state":
        if (event.state === "idle") {
          state.serverState = "idle";
          state.simInfo = null;
        } else if (event.state === "paused") {
          state.serverState = "paused";
        } else if (event.state === "running") {
          state.serverState = "running";
        }
        break;

      case "textures-ready":
        state.textureRevision++;
        break;

      case "complete":
        state.connectionState = "complete";
        break;

      case "error":
        state.connectionState = "error";
        break;

      case "range-response":
        // Store for App.tsx zoom logic to consume. Basic handling:
        // merge response points into trail/ingest buffers.
        if (event.points.length > 0) {
          const rangeId = event.points[0].entityPath ?? "default";
          const trailBuf = getOrCreateTrailBuffer(buffers.trailBuffers, rangeId);
          trailBuf.clear();
          trailBuf.pushMany(event.points);
          getOrCreateIngestBuffer(buffers.ingestBuffers, rangeId).markRebuild(event.points);
        }
        break;

      case "progress":
        break;
    }
  };
}
