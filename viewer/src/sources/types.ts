/**
 * Source-based data architecture types.
 *
 * All data inputs (WebSocket, CSV files, RRD files) are abstracted as
 * SourceAdapters that emit SourceEvents into a unified pipeline.
 */

import type { SatelliteInfo, SimInfo } from "../hooks/useWebSocket.js";
import type { OrbitPoint } from "../orbit.js";

// Re-export for convenience
export type { SatelliteInfo, SimInfo };

// ---------------------------------------------------------------------------
// Source specification
// ---------------------------------------------------------------------------

/** Describes how to connect to / load a data source. */
export type SourceSpec =
  | { type: "websocket"; url: string }
  | { type: "csv-file"; file: File }
  | { type: "rrd-file"; file: File }; // Phase 2+

/** Opaque identifier for a source instance. */
export type SourceId = string;

// ---------------------------------------------------------------------------
// Source events (discriminated union)
// ---------------------------------------------------------------------------

/** Events emitted by a SourceAdapter into the runtime. */
export type SourceEvent =
  | { kind: "info"; info: SimInfo }
  | { kind: "state"; point: OrbitPoint }
  | { kind: "history"; points: OrbitPoint[] }
  | { kind: "history-chunk"; points: OrbitPoint[]; done: boolean }
  | { kind: "history-detail"; points: OrbitPoint[] }
  | { kind: "history-detail-complete" }
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
  | { kind: "progress"; loaded: number; total: number }
  | { kind: "complete" };

// ---------------------------------------------------------------------------
// Source capabilities & connection state
// ---------------------------------------------------------------------------

/** What a source can do. */
export interface SourceCapabilities {
  /** Source is still receiving new data (WS streaming). */
  live: boolean;
  /** Can send control messages (pause/resume/terminate). */
  control: boolean;
  /** Supports query_range requests. */
  rangeQuery: boolean;
  /** Supports history_detail backfill. */
  backfill: boolean;
}

export type SourceConnectionState =
  | "disconnected"
  | "connecting"
  | "connected"
  | "loading" // file: parsing in progress
  | "complete" // file: fully loaded
  | "error";

// ---------------------------------------------------------------------------
// Source adapter interface
// ---------------------------------------------------------------------------

/** Callback signature for receiving events from an adapter. */
export type SourceEventHandler = (sourceId: SourceId, event: SourceEvent) => void;

/**
 * Abstract interface for a data source.
 *
 * Implementations: WebSocketAdapter, CSVFileAdapter, (future) RrdFileAdapter.
 * Each adapter normalizes its transport-specific protocol into SourceEvents.
 */
export interface SourceAdapter {
  readonly sourceId: SourceId;
  readonly spec: SourceSpec;
  readonly capabilities: SourceCapabilities;
  readonly connectionState: SourceConnectionState;

  /** Start receiving data. Events are emitted via the handler passed at construction. */
  start(): void;

  /** Stop receiving data and clean up resources. */
  stop(): void;

  /** Send a control message (only meaningful for WS adapters). */
  send?(msg: Record<string, unknown>): void;
}
