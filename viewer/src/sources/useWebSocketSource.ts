/**
 * Adapter hook that encapsulates the WebSocket → SourceEvent bridge.
 *
 * Connects the low-level `useWebSocket` hook to the unified
 * `SourceEvent` pipeline, translating each WS callback into
 * the appropriate event and routing it through `handleEvent`.
 */

import { useCallback, useRef, useState } from "react";
import {
  type QueryRangeResponse,
  type SatelliteInfo,
  type SimInfo,
  useWebSocket,
} from "../hooks/useWebSocket.js";
import type { OrbitPoint } from "../orbit.js";
import type { ClientMessage } from "../protocol/generated/ClientMessage.js";
import type { Pacing } from "../protocol/generated/Pacing.js";
import type { SimConfig } from "../protocol/generated/SimConfig.js";
import { mergeQueryRangePoints, pickTrailBufferForResponse } from "../utils/mergeQueryRange.js";
import type { TrailBuffer } from "../utils/TrailBuffer.js";
import type { SourceEvent } from "./types.js";

export const WS_SOURCE_ID = "ws-0";

// Options & result types

export interface UseWebSocketSourceOptions {
  wsUrl: string;
  handleEvent: (sourceId: string, event: SourceEvent) => void;
  /** For merging query_range responses with existing trail data */
  trailBuffers: Map<string, TrailBuffer>;
  simInfo: SimInfo | null;
  /** Optional: ref to latest requested range for staleness check */
  latestRequestedRangeRef?: React.RefObject<{ tMin: number; tMax: number } | null>;
}

export interface WebSocketSourceResult {
  connect: () => void;
  disconnect: () => void;
  isConnected: boolean;
  send: (msg: ClientMessage) => void;
  /** `pacing` left out runs at the server's default. */
  handleStartSimulation: (config: SimConfig, pacing?: Pacing) => void;
  /**
   * The pacing a start naming none runs at, as the server's last idle status
   * said, or null before one has. Kept across the running status changes,
   * which do not carry it.
   */
  serverDefaultPacing: Pacing | null;
  handlePause: () => void;
  handleResume: () => void;
  handleTerminate: () => void;
}

// Hook

export function useWebSocketSource(options: UseWebSocketSourceOptions): WebSocketSourceResult {
  const { wsUrl, handleEvent, trailBuffers, simInfo, latestRequestedRangeRef } = options;

  // Keep mutable refs for values that change between renders but shouldn't
  // trigger re-creation of the callbacks passed to useWebSocket.
  const trailBuffersRef = useRef(trailBuffers);
  trailBuffersRef.current = trailBuffers;

  const simInfoRef = useRef(simInfo);
  simInfoRef.current = simInfo;

  // WS → SourceEvent bridge callbacks

  const handleState = useCallback(
    (point: OrbitPoint) => handleEvent(WS_SOURCE_ID, { kind: "state", point }),
    [handleEvent],
  );
  const handleInfo = useCallback(
    (info: SimInfo) => handleEvent(WS_SOURCE_ID, { kind: "info", info }),
    [handleEvent],
  );
  const [serverDefaultPacing, setServerDefaultPacing] = useState<Pacing | null>(null);
  const handleStatus = useCallback(
    (state: string, defaultPacing?: Pacing) => {
      if (defaultPacing !== undefined) setServerDefaultPacing(defaultPacing);
      handleEvent(WS_SOURCE_ID, { kind: "server-state", state });
    },
    [handleEvent],
  );
  const handleError = useCallback(
    (message: string) => handleEvent(WS_SOURCE_ID, { kind: "error", message }),
    [handleEvent],
  );
  const handleSimulationTerminated = useCallback(
    (entityPath: string, t: number, reason: string) =>
      handleEvent(WS_SOURCE_ID, { kind: "terminated", entityPath, t, reason }),
    [handleEvent],
  );
  const handleHistory = useCallback(
    (points: OrbitPoint[]) => {
      handleEvent(WS_SOURCE_ID, { kind: "history", points });
      // Dev-only: expose history arrival diagnostic for E2E tests
      if (import.meta.env.DEV) {
        const byId = new Map<string, number>();
        for (const p of points) {
          const id = p.entityPath ?? "default";
          byId.set(id, (byId.get(id) ?? 0) + 1);
        }
        (window as unknown as Record<string, unknown>).__debug_last_history = {
          historyLen: points.length,
          byIdCounts: Object.fromEntries(byId),
        };
      }
    },
    [handleEvent],
  );
  const handleQueryRangeResponse = useCallback(
    (response: QueryRangeResponse) => {
      // Discard stale responses
      if (latestRequestedRangeRef) {
        const latest = latestRequestedRangeRef.current;
        if (latest && (response.tMin !== latest.tMin || response.tMax !== latest.tMax)) {
          return;
        }
      }
      // Merge with existing streaming tail for *the same satellite* so
      // the 3D position does not rewind. Using a hard-coded fallback
      // (satellites[0]) would contaminate sat B's trail with sat A's
      // tail in multi-sat sims.
      const fallbackSatId = simInfoRef.current?.satellites[0]?.id ?? null;
      const trailBuf = pickTrailBufferForResponse(
        response.points,
        trailBuffersRef.current,
        fallbackSatId,
      );
      const merged = trailBuf
        ? mergeQueryRangePoints(response.points, trailBuf.getAll())
        : response.points;
      handleEvent(WS_SOURCE_ID, {
        kind: "range-response",
        tMin: response.tMin,
        tMax: response.tMax,
        points: merged,
      });
    },
    [handleEvent, latestRequestedRangeRef],
  );
  const handleTexturesReady = useCallback(
    (body: string) => handleEvent(WS_SOURCE_ID, { kind: "textures-ready", body }),
    [handleEvent],
  );
  const handleSatelliteAdded = useCallback(
    (satellite: SatelliteInfo, t: number) =>
      handleEvent(WS_SOURCE_ID, { kind: "satellite-added", satellite, t }),
    [handleEvent],
  );

  const {
    connect: openSocket,
    disconnect: closeSocket,
    isConnected,
    send,
  } = useWebSocket({
    url: wsUrl,
    onState: handleState,
    onInfo: handleInfo,
    onHistory: handleHistory,
    onQueryRangeResponse: handleQueryRangeResponse,
    onSimulationTerminated: handleSimulationTerminated,
    onStatus: handleStatus,
    onError: handleError,
    onTexturesReady: handleTexturesReady,
    onSatelliteAdded: handleSatelliteAdded,
  });

  // The default pacing belongs to the server that said it. A connection may
  // reach another server (or one restarted with other flags), and one already
  // running sends no default until it goes idle, so a remembered one would
  // label the config dialog with the wrong server's speed.
  const connect = useCallback(() => {
    setServerDefaultPacing(null);
    openSocket();
  }, [openSocket]);
  const disconnect = useCallback(() => {
    setServerDefaultPacing(null);
    closeSocket();
  }, [closeSocket]);

  // Sim control callbacks

  const handleStartSimulation = useCallback(
    (config: SimConfig, pacing?: Pacing) => {
      send({ type: "start_simulation", config, ...(pacing !== undefined && { pacing }) });
    },
    [send],
  );

  const handlePause = useCallback(() => {
    send({ type: "pause_simulation" });
  }, [send]);

  const handleResume = useCallback(() => {
    send({ type: "resume_simulation" });
  }, [send]);

  const handleTerminate = useCallback(() => {
    send({ type: "terminate_simulation" });
  }, [send]);

  return {
    connect,
    disconnect,
    isConnected,
    send,
    handleStartSimulation,
    serverDefaultPacing,
    handlePause,
    handleResume,
    handleTerminate,
  };
}
