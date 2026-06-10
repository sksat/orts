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
import type { SourceAdapter, SourceEventHandler, SourceId } from "./types.js";

export class CSVFileAdapter implements SourceAdapter {
  readonly sourceId: SourceId;

  private worker: Worker | null = null;
  private reader: FileReader | null = null;
  private onEvent: SourceEventHandler;
  private file: File;
  private stopped = false;

  constructor(sourceId: SourceId, file: File, onEvent: SourceEventHandler) {
    this.sourceId = sourceId;
    this.onEvent = onEvent;
    this.file = file;
  }

  start(): void {
    // Reset state for restartability
    this.estimatedDt = null;
    this.lastTByEntity.clear();
    this.pendingMetadata = null;
    this.infoEmitted = false;
    this.stopped = false;

    const reader = new FileReader();
    this.reader = reader;
    reader.onload = () => {
      if (this.stopped) return; // Cancelled while reading
      const text = reader.result as string;
      this.startWorker(text);
    };
    reader.onerror = () => {
      if (this.stopped) return;
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
  }

  private startWorker(text: string): void {
    this.worker = new Worker(new URL("./csvParseWorker.ts", import.meta.url), { type: "module" });

    this.worker.onmessage = (e: MessageEvent<CSVWorkerMessage>) => {
      this.handleWorkerMessage(e.data);
    };

    this.worker.onerror = (err: ErrorEvent) => {
      this.onEvent(this.sourceId, {
        kind: "error",
        message: `CSV worker error: ${err.message ?? "unknown"}`,
      });
    };

    this.worker.postMessage({ type: "parse", text });
  }

  private pendingMetadata: CSVMetadata | null = null;
  private infoEmitted = false;
  /** Estimated time step [s]; null until two same-entity timestamps are seen. */
  private estimatedDt: number | null = null;
  /** Last seen timestamp per entity, persisted across chunks for dt estimation. */
  private lastTByEntity = new Map<string, number>();

  /** Fallback dt when a file has fewer than two same-entity points. */
  private static readonly DEFAULT_DT = 10;

  private handleWorkerMessage(msg: CSVWorkerMessage): void {
    const id = this.sourceId;

    switch (msg.type) {
      case "metadata": {
        // Defer info emission until first chunk arrives so dt is accurate
        this.pendingMetadata = msg.metadata;
        break;
      }

      case "chunk": {
        // Estimate dt from consecutive points of the SAME entity. Multi-sat
        // CSVs interleave entities row by row (sat1@t0, sat2@t0, sat1@t1, …),
        // so naive points[1].t - points[0].t would latch dt = 0. Last-seen
        // timestamps persist across chunks so chunk boundaries don't prevent
        // detection.
        if (this.estimatedDt === null) {
          for (const p of msg.points) {
            const key = p.entityPath ?? "default";
            const prevT = this.lastTByEntity.get(key);
            if (prevT !== undefined && p.t > prevT) {
              this.estimatedDt = p.t - prevT;
              break;
            }
            this.lastTByEntity.set(key, p.t);
          }
        }
        // Emit info event on first chunk (now dt is known)
        if (!this.infoEmitted && this.pendingMetadata) {
          const info = csvMetadataToSimInfo(
            this.pendingMetadata,
            this.file.name,
            this.estimatedDt ?? CSVFileAdapter.DEFAULT_DT,
          );
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
          const info = csvMetadataToSimInfo(
            this.pendingMetadata,
            this.file.name,
            this.estimatedDt ?? CSVFileAdapter.DEFAULT_DT,
          );
          this.onEvent(id, { kind: "info", info });
          this.infoEmitted = true;
        }
        this.onEvent(id, { kind: "history-chunk", points: [], done: true });
        this.onEvent(id, { kind: "complete" });
        // Clean up worker
        if (this.worker) {
          this.worker.terminate();
          this.worker = null;
        }
        break;

      case "error":
        this.onEvent(id, { kind: "error", message: msg.message });
        break;
    }
  }
}
