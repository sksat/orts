/**
 * WebSocket-based SourceAdapter.
 *
 * Wraps a WebSocket connection and translates server messages into SourceEvents
 * using the existing `dispatchServerMessage` pure function from useWebSocket.ts.
 *
 * No React dependency — pure class, fully testable.
 */

import {
  type DispatchCallbacks,
  dispatchServerMessage,
  type ServerMessage,
} from "../hooks/useWebSocket.js";
import type {
  SourceAdapter,
  SourceCapabilities,
  SourceConnectionState,
  SourceEventHandler,
  SourceId,
  SourceSpec,
} from "./types.js";

export class WebSocketAdapter implements SourceAdapter {
  readonly sourceId: SourceId;
  readonly spec: SourceSpec & { type: "websocket" };
  readonly capabilities: SourceCapabilities = {
    live: true,
    control: true,
    rangeQuery: true,
  };

  private ws: WebSocket | null = null;
  private _connectionState: SourceConnectionState = "disconnected";
  private onEvent: SourceEventHandler;

  constructor(sourceId: SourceId, url: string, onEvent: SourceEventHandler) {
    this.sourceId = sourceId;
    this.spec = { type: "websocket", url };
    this.onEvent = onEvent;
  }

  get connectionState(): SourceConnectionState {
    return this._connectionState;
  }

  start(): void {
    if (this.ws) return;
    this._connectionState = "connecting";

    this.ws = new WebSocket(this.spec.url);

    this.ws.onopen = () => {
      this._connectionState = "connected";
    };

    this.ws.onclose = () => {
      this._connectionState = "disconnected";
      this.ws = null;
    };

    this.ws.onerror = () => {
      this._connectionState = "error";
      this.onEvent(this.sourceId, {
        kind: "error",
        message: "WebSocket transport error",
      });
    };

    this.ws.onmessage = (e: MessageEvent) => {
      try {
        const msg = JSON.parse(e.data as string) as ServerMessage;
        this.dispatchToSourceEvents(msg);
      } catch {
        // Ignore malformed frames
      }
    };
  }

  stop(): void {
    if (this.ws) {
      this.ws.onopen = null;
      this.ws.onclose = null;
      this.ws.onerror = null;
      this.ws.onmessage = null;
      this.ws.close();
      this.ws = null;
    }
    this._connectionState = "disconnected";
  }

  send(msg: Record<string, unknown>): void {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(msg));
    }
  }

  /**
   * Translate a ServerMessage into SourceEvent(s) using dispatchServerMessage.
   * We provide DispatchCallbacks that emit SourceEvents for each callback.
   */
  private dispatchToSourceEvents(msg: ServerMessage): void {
    const id = this.sourceId;
    const emit = this.onEvent;

    const callbacks: DispatchCallbacks = {
      onState: (point) => emit(id, { kind: "state", point }),
      onInfo: (info) => emit(id, { kind: "info", info }),
      onHistory: (points) => emit(id, { kind: "history", points }),
      onQueryRangeResponse: (response) =>
        emit(id, {
          kind: "range-response",
          tMin: response.tMin,
          tMax: response.tMax,
          points: response.points,
        }),
      onSimulationTerminated: (entityPath, t, reason) =>
        emit(id, { kind: "terminated", entityPath, t, reason }),
      onStatus: (state) => emit(id, { kind: "server-state", state }),
      onError: (message) => emit(id, { kind: "error", message }),
      onTexturesReady: (body) => emit(id, { kind: "textures-ready", body }),
    };

    dispatchServerMessage(msg, callbacks);
  }
}
