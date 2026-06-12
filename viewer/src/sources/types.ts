/**
 * Source-based data architecture types.
 *
 * Every data input normalizes into the SourceEvent stream consumed by
 * useSourceRuntime / eventDispatcher. The live WebSocket path bridges
 * through the useWebSocket hook (see useWebSocketSource); file-replay
 * sources (CSV, RRD) implement the SourceAdapter interface.
 */

import type { SatelliteInfo, SimInfo } from "../hooks/useWebSocket.js";
import type { OrbitPoint } from "../orbit.js";

// Re-export for convenience
export type { SatelliteInfo, SimInfo };

/** Opaque identifier for a source instance. */
export type SourceId = string;

// Source events (discriminated union)

/** Events emitted by a source into the runtime. */
export type SourceEvent =
  | { kind: "info"; info: SimInfo }
  | { kind: "state"; point: OrbitPoint }
  | { kind: "history"; points: OrbitPoint[] }
  | { kind: "history-chunk"; points: OrbitPoint[]; done: boolean }
  | {
      kind: "range-response";
      tMin: number;
      tMax: number;
      points: OrbitPoint[];
    }
  | { kind: "terminated"; entityPath: string; t: number; reason: string }
  | { kind: "server-state"; state: string }
  | { kind: "error"; message: string }
  | { kind: "textures-ready"; body: string }
  | { kind: "complete" };

/**
 * Runtime-level connection state, derived from the event stream by
 * the event dispatcher (not reported by sources themselves).
 */
export type SourceConnectionState =
  | "disconnected"
  | "connecting"
  | "connected"
  | "loading" // file: parsing in progress
  | "complete" // file: fully loaded
  | "error";

// Source adapter interface

/** Callback signature for receiving events from a source. */
export type SourceEventHandler = (sourceId: SourceId, event: SourceEvent) => void;

/**
 * File-replay source adapter (CSV, RRD).
 *
 * Each adapter parses its file format off the main thread and normalizes
 * the result into SourceEvents. The live WebSocket path does not implement
 * this interface — it bridges through useWebSocket/useWebSocketSource,
 * which owns React connection state and the typed control channel.
 */
export interface SourceAdapter {
  readonly sourceId: SourceId;

  /** Start loading. Events are emitted via the handler passed at construction. */
  start(): void;

  /** Stop loading and clean up resources (abort reader, terminate worker). */
  stop(): void;
}
