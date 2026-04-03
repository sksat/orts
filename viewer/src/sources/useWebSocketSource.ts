/**
 * Adapter hook that encapsulates the WebSocket → SourceEvent bridge.
 *
 * Connects the low-level `useWebSocket` hook to the unified
 * `SourceEvent` pipeline, translating each WS callback into
 * the appropriate event and routing it through `handleEvent`.
 */

import { useCallback, useRef } from "react";
import type { SimConfigPayload } from "../components/SimConfigForm.js";
import { type QueryRangeResponse, type SimInfo, useWebSocket } from "../hooks/useWebSocket.js";
import type { OrbitPoint } from "../orbit.js";
import { mergeQueryRangePoints } from "../utils/mergeQueryRange.js";
import type { TrailBuffer } from "../utils/TrailBuffer.js";
import type { SourceEvent } from "./types.js";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

export const WS_SOURCE_ID = "ws-0";

// ---------------------------------------------------------------------------
// Options & result types
// ---------------------------------------------------------------------------

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
  send: (msg: unknown) => void;
  handleStartSimulation: (config: SimConfigPayload) => void;
  handlePause: () => void;
  handleResume: () => void;
  handleTerminate: () => void;
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

export function useWebSocketSource(options: UseWebSocketSourceOptions): WebSocketSourceResult {
  const { wsUrl, handleEvent, trailBuffers, simInfo, latestRequestedRangeRef } = options;

  // Keep mutable refs for values that change between renders but shouldn't
  // trigger re-creation of the callbacks passed to useWebSocket.
  const trailBuffersRef = useRef(trailBuffers);
  trailBuffersRef.current = trailBuffers;

  const simInfoRef = useRef(simInfo);
  simInfoRef.current = simInfo;

  // --- WS → SourceEvent bridge callbacks ---

  const handleState = useCallback(
    (point: OrbitPoint) => handleEvent(WS_SOURCE_ID, { kind: "state", point }),
    [handleEvent],
  );
  const handleInfo = useCallback(
    (info: SimInfo) => handleEvent(WS_SOURCE_ID, { kind: "info", info }),
    [handleEvent],
  );
  const handleStatus = useCallback(
    (state: string) => handleEvent(WS_SOURCE_ID, { kind: "server-state", state }),
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
  const handleHistoryDetail = useCallback(
    (points: OrbitPoint[]) => handleEvent(WS_SOURCE_ID, { kind: "history-detail", points }),
    [handleEvent],
  );
  const handleHistoryDetailComplete = useCallback(
    () => handleEvent(WS_SOURCE_ID, { kind: "history-detail-complete" }),
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
      // Merge with existing streaming data to avoid position rewind
      const satId = simInfoRef.current?.satellites[0]?.id ?? "default";
      const trailBuf = trailBuffersRef.current.get(satId);
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

  // --- useWebSocket ---

  const { connect, disconnect, isConnected, send } = useWebSocket({
    url: wsUrl,
    onState: handleState,
    onInfo: handleInfo,
    onHistory: handleHistory,
    onHistoryDetail: handleHistoryDetail,
    onHistoryDetailComplete: handleHistoryDetailComplete,
    onQueryRangeResponse: handleQueryRangeResponse,
    onSimulationTerminated: handleSimulationTerminated,
    onStatus: handleStatus,
    onError: handleError,
    onTexturesReady: handleTexturesReady,
  });

  // --- Sim control callbacks ---

  const handleStartSimulation = useCallback(
    (config: SimConfigPayload) => {
      send({ type: "start_simulation", config });
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
    send: send as (msg: unknown) => void,
    handleStartSimulation,
    handlePause,
    handleResume,
    handleTerminate,
  };
}
