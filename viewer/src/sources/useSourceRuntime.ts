/**
 * Central hook for managing data sources and routing SourceEvents into buffers.
 *
 * The core event dispatch logic lives in `eventDispatcher.ts` —
 * a pure module testable without React. This hook wraps it with
 * React state management.
 */

import { useCallback, useRef, useState } from "react";
import { ChartBuffer, IngestBuffer } from "uneri";
import type { OrbitPoint } from "../orbit.js";
import { TrailBuffer } from "../utils/TrailBuffer.js";
import {
  createEventDispatcher,
  isDataBumpEvent,
  type RuntimeBuffers,
  type RuntimeState,
  type ServerState,
  setIngestBufferFactory,
  setTrailBufferFactory,
} from "./eventDispatcher.js";
import type { SimInfo, SourceConnectionState, SourceEvent, SourceId } from "./types.js";

// Re-export for convenience
export type { ServerState } from "./eventDispatcher.js";

// ---------------------------------------------------------------------------
// Chart column names
// ---------------------------------------------------------------------------

const CHART_COLUMNS = [
  "t",
  "altitude",
  "energy",
  "angular_momentum",
  "velocity",
  "a",
  "e",
  "inc_deg",
  "raan_deg",
  "accel_gravity",
  "accel_drag",
  "accel_srp",
  "accel_third_body_sun",
  "accel_third_body_moon",
  "accel_perturbation_total",
];

// ---------------------------------------------------------------------------
// Initialize factories for eventDispatcher
// ---------------------------------------------------------------------------

setTrailBufferFactory(() => new TrailBuffer(50000));
setIngestBufferFactory(() => new IngestBuffer<OrbitPoint>());

// ---------------------------------------------------------------------------
// React hook
// ---------------------------------------------------------------------------

export function useSourceRuntime() {
  const trailBuffersRef = useRef(new Map<string, TrailBuffer>());
  const ingestBuffersRef = useRef(new Map<string, IngestBuffer<OrbitPoint>>());
  const chartBufferRef = useRef(new ChartBuffer(CHART_COLUMNS, 50000));
  const streamingCountRef = useRef(0);
  const chunkLoadStartedRef = useRef(false);
  const chartDirtyRef = useRef(false);

  const [simInfo, setSimInfo] = useState<SimInfo | null>(null);
  const [serverState, setServerState] = useState<ServerState>("unknown");
  const [terminatedSatellites, setTerminatedSatellites] = useState(() => new Set<string>());
  const [connectionState, setConnectionState] = useState<SourceConnectionState>("disconnected");
  const connectionStateRef = useRef<SourceConnectionState>("disconnected");
  connectionStateRef.current = connectionState;
  const [textureRevision, setTextureRevision] = useState(0);
  const [chartBufferVersion, setChartBufferVersion] = useState(0);

  const activeSourceIdRef = useRef<SourceId | null>(null);

  const setActiveSourceId = useCallback((id: SourceId | null) => {
    activeSourceIdRef.current = id;
    if (id) {
      setConnectionState("connecting");
    } else {
      setConnectionState("disconnected");
    }
  }, []);

  const bumpChartVersion = useCallback(() => {
    if (!chartDirtyRef.current) {
      chartDirtyRef.current = true;
      requestAnimationFrame(() => {
        chartDirtyRef.current = false;
        setChartBufferVersion((v) => v + 1);
      });
    }
  }, []);

  const handleEvent = useCallback(
    (sourceId: SourceId, event: SourceEvent) => {
      if (activeSourceIdRef.current !== null && sourceId !== activeSourceIdRef.current) {
        return;
      }

      // Build mutable containers for the pure dispatcher
      const buffers: RuntimeBuffers = {
        trailBuffers: trailBuffersRef.current,
        ingestBuffers: ingestBuffersRef.current,
        chartBuffer: chartBufferRef.current,
        streamingCount: streamingCountRef.current,
        chunkLoadStarted: chunkLoadStartedRef.current,
      };
      // Seed mutableState with current connectionState so the dispatcher
      // can preserve "loading" for file sources (instead of defaulting to "disconnected").
      const mutableState: RuntimeState = {
        simInfo: null,
        serverState: "unknown",
        terminatedSatellites: new Set(),
        connectionState: connectionStateRef.current,
        textureRevision: 0,
      };

      // Dispatch via the pure event dispatcher (eventDispatcher.ts)
      const dispatch = createEventDispatcher(buffers, mutableState, activeSourceIdRef.current);
      dispatch(sourceId, event);

      // Sync back mutable buffer state
      streamingCountRef.current = buffers.streamingCount;
      chunkLoadStartedRef.current = buffers.chunkLoadStarted;

      // Sync React state (only for events that change it)
      switch (event.kind) {
        case "info":
          setSimInfo(mutableState.simInfo);
          setServerState(mutableState.serverState);
          setConnectionState(mutableState.connectionState);
          break;
        case "terminated":
          // Use updater to preserve previously terminated satellites
          setTerminatedSatellites((prev) => {
            const next = new Set(prev);
            if (event.kind === "terminated") next.add(event.entityPath);
            return next;
          });
          break;
        case "server-state":
          setServerState(mutableState.serverState);
          if (event.state === "idle") setSimInfo(null);
          break;
        case "textures-ready":
          setTextureRevision((v) => v + 1);
          break;
        case "complete":
          setConnectionState("complete");
          break;
        case "error":
          setConnectionState("error");
          break;
      }

      // Bump chart version for events that modify buffers.
      if (isDataBumpEvent(event)) {
        bumpChartVersion();
      }
    },
    [bumpChartVersion],
  );

  const resetBuffers = useCallback(() => {
    trailBuffersRef.current.clear();
    ingestBuffersRef.current.clear();
    chartBufferRef.current.clear();
    streamingCountRef.current = 0;
    chunkLoadStartedRef.current = false;
    chartDirtyRef.current = false;
    setChartBufferVersion((v) => v + 1);
    setSimInfo(null);
    setServerState("unknown");
    setTerminatedSatellites(new Set());
    setConnectionState("disconnected");
    activeSourceIdRef.current = null;
  }, []);

  return {
    trailBuffers: trailBuffersRef.current,
    ingestBuffers: ingestBuffersRef.current,
    chartBuffer: chartBufferRef.current,
    simInfo,
    serverState,
    terminatedSatellites,
    connectionState,
    textureRevision,
    chartBufferVersion,
    isLive:
      connectionState === "connected" && (serverState === "running" || serverState === "idle"),
    handleEvent,
    setActiveSourceId,
    resetBuffers,
  };
}
