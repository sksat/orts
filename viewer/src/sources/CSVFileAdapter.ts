/**
 * CSV file SourceAdapter.
 *
 * Reads a CSV file via FileReader, sends it to a Web Worker for chunked
 * parsing, and translates worker messages into SourceEvents.
 *
 * The CSV metadata is normalized into SimInfo via normalizeMetadata,
 * so downstream consumers only deal with SimInfo.
 */

import type { CSVMetadata } from "../orbit.js";
import type { CSVWorkerMessage } from "./csvParseLogic.js";
import { csvMetadataToSimInfo } from "./normalizeMetadata.js";
import type {
  SourceAdapter,
  SourceCapabilities,
  SourceConnectionState,
  SourceEventHandler,
  SourceId,
  SourceSpec,
} from "./types.js";

export class CSVFileAdapter implements SourceAdapter {
  readonly sourceId: SourceId;
  readonly spec: SourceSpec & { type: "csv-file" };
  readonly capabilities: SourceCapabilities = {
    live: false,
    control: false,
    rangeQuery: false,
    backfill: false,
  };

  private worker: Worker | null = null;
  private reader: FileReader | null = null;
  private _connectionState: SourceConnectionState = "disconnected";
  private onEvent: SourceEventHandler;
  private file: File;
  private estimatedDt = 10; // will be refined from first chunk
  private stopped = false;

  constructor(sourceId: SourceId, file: File, onEvent: SourceEventHandler) {
    this.sourceId = sourceId;
    this.spec = { type: "csv-file", file };
    this.onEvent = onEvent;
    this.file = file;
  }

  get connectionState(): SourceConnectionState {
    return this._connectionState;
  }

  start(): void {
    // Reset state for restartability
    this.estimatedDt = 10;
    this.pendingMetadata = null;
    this.infoEmitted = false;
    this.stopped = false;
    this._connectionState = "loading";

    const reader = new FileReader();
    this.reader = reader;
    reader.onload = () => {
      if (this.stopped) return; // Cancelled while reading
      const text = reader.result as string;
      this.startWorker(text);
    };
    reader.onerror = () => {
      if (this.stopped) return;
      this._connectionState = "error";
      this.onEvent(this.sourceId, {
        kind: "error",
        message: `Failed to read file: ${this.file.name}`,
      });
    };
    reader.readAsText(this.file);
  }

  stop(): void {
    this.stopped = true;
    if (this.reader) {
      this.reader.abort();
      this.reader = null;
    }
    if (this.worker) {
      this.worker.terminate();
      this.worker = null;
    }
    this._connectionState = "disconnected";
  }

  private startWorker(text: string): void {
    this.worker = new Worker(new URL("./csvParseWorker.ts", import.meta.url), { type: "module" });

    this.worker.onmessage = (e: MessageEvent<CSVWorkerMessage>) => {
      this.handleWorkerMessage(e.data);
    };

    this.worker.onerror = (err: ErrorEvent) => {
      this._connectionState = "error";
      this.onEvent(this.sourceId, {
        kind: "error",
        message: `CSV worker error: ${err.message ?? "unknown"}`,
      });
    };

    this.worker.postMessage({ type: "parse", text });
  }

  private pendingMetadata: CSVMetadata | null = null;
  private infoEmitted = false;

  private handleWorkerMessage(msg: CSVWorkerMessage): void {
    const id = this.sourceId;

    switch (msg.type) {
      case "metadata": {
        // Defer info emission until first chunk arrives so dt is accurate
        this.pendingMetadata = msg.metadata;
        break;
      }

      case "chunk": {
        // Estimate dt from first chunk's first two points
        if (msg.points.length >= 2 && this.estimatedDt === 10) {
          this.estimatedDt = msg.points[1].t - msg.points[0].t;
        }
        // Emit info event on first chunk (now dt is known)
        if (!this.infoEmitted && this.pendingMetadata) {
          const info = csvMetadataToSimInfo(this.pendingMetadata, this.file.name, this.estimatedDt);
          this.onEvent(id, { kind: "info", info });
          this.infoEmitted = true;
        }
        this.onEvent(id, {
          kind: "history-chunk",
          points: msg.points,
          done: false,
        });
        break;
      }

      case "complete":
        // Emit info if never emitted (e.g., file with metadata but no valid data rows)
        if (!this.infoEmitted && this.pendingMetadata) {
          const info = csvMetadataToSimInfo(this.pendingMetadata, this.file.name, this.estimatedDt);
          this.onEvent(id, { kind: "info", info });
          this.infoEmitted = true;
        }
        this.onEvent(id, { kind: "history-chunk", points: [], done: true });
        this.onEvent(id, { kind: "complete" });
        this._connectionState = "complete";
        // Clean up worker
        if (this.worker) {
          this.worker.terminate();
          this.worker = null;
        }
        break;

      case "error":
        this.onEvent(id, { kind: "error", message: msg.message });
        this._connectionState = "error";
        break;
    }
  }
}
