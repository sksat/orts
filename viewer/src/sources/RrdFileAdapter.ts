/**
 * RRD file SourceAdapter.
 *
 * Reads an RRD file via FileReader.readAsArrayBuffer(), sends the bytes
 * to a Web Worker (which decodes via rrd-wasm WASM), and translates
 * worker messages into SourceEvents.
 */

import type { OrbitPoint } from "../orbit.js";
import { initArika, orbit_derived_batch } from "../wasm/arikaInit.js";
import { rrdMetadataToSimInfo } from "./normalizeMetadata.js";
import {
  ORBIT_DERIVED_STRIDE,
  type OrbitDerivedContext,
  orbitDerivedContext,
  packStates,
  toOrbitPoints,
} from "./rrdOrbitDerived.js";
import type { RrdPointOut, RrdWorkerMessage } from "./rrdParseLogic.js";
import type { SourceAdapter, SourceEventHandler, SourceId } from "./types.js";

export class RrdFileAdapter implements SourceAdapter {
  readonly sourceId: SourceId;

  private worker: Worker | null = null;
  private reader: FileReader | null = null;
  private onEvent: SourceEventHandler;
  private file: File;
  private estimatedDt = 10;
  private stopped = false;
  /// The central body constants the derived values are computed against. The
  /// Worker sends metadata before the first chunk, so this is set by then.
  private derivedContext: OrbitDerivedContext | null = null;
  /// Whether this load has already published its first point for the E2E.
  private debugPointExposed = false;

  constructor(sourceId: SourceId, file: File, onEvent: SourceEventHandler) {
    this.sourceId = sourceId;
    this.onEvent = onEvent;
    this.file = file;
  }

  start(): void {
    // Make restart safe: kill any in-flight read/worker first, then reset
    // per-load state (stop() sets `stopped`; the reset below re-arms it).
    this.stop();
    this.estimatedDt = 10;
    this.pendingMetadata = null;
    this.pendingEntityPaths = new Set();
    this.lastTByEntity.clear();
    this.infoEmitted = false;
    this.stopped = false;
    this.debugPointExposed = false;

    const reader = new FileReader();
    this.reader = reader;
    reader.onload = () => {
      if (this.stopped) return;
      const buffer = reader.result as ArrayBuffer;
      // The derived values need `arika-wasm`. The app starts loading it at
      // mount, but the file input is usable before that finishes, so wait
      // here rather than letting the first chunks come out non-finite with no
      // second chance at them.
      initArika()
        .catch((e) => {
          console.warn("RrdFileAdapter: arika WASM failed to load:", e);
        })
        .then(() => {
          if (this.stopped) return;
          this.startWorker(buffer);
        });
    };
    reader.onerror = () => {
      if (this.stopped) return;
      this.onEvent(this.sourceId, {
        kind: "error",
        message: `Failed to read file: ${this.file.name}`,
      });
    };
    reader.readAsArrayBuffer(this.file);
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

  private startWorker(buffer: ArrayBuffer): void {
    this.worker = new Worker(new URL("./rrdParseWorker.ts", import.meta.url), { type: "module" });

    this.worker.onmessage = (e: MessageEvent<RrdWorkerMessage>) => {
      this.handleWorkerMessage(e.data);
    };

    this.worker.onerror = (err: ErrorEvent) => {
      this.onEvent(this.sourceId, {
        kind: "error",
        message: `RRD worker error: ${err.message ?? "unknown"}`,
      });
    };

    // Transfer the ArrayBuffer to the worker (zero-copy)
    this.worker.postMessage({ type: "parse", buffer }, [buffer]);
  }

  private pendingMetadata: import("../wasm/rrdWasmInit.js").RrdMetadata | null = null;
  private pendingEntityPaths = new Set<string>();
  private infoEmitted = false;
  /** Last seen timestamp per entity, persisted across chunks for dt estimation. */
  private lastTByEntity = new Map<string, number>();

  /**
   * Fill in the Keplerian elements and the chart scalars for one chunk.
   *
   * The `arika-wasm` instance is awaited before the file is read, so this is a
   * synchronous call by the time chunks arrive. If initialisation itself failed,
   * the points still reach the store with the state vectors intact and the
   * derived fields left non-finite — dropping the chunk would lose the
   * trajectory as well.
   */
  private deriveChunk(points: readonly RrdPointOut[]): OrbitPoint[] {
    const ctx = this.derivedContext ?? orbitDerivedContext({ mu: null, body_radius: null });
    let derived: Float64Array;
    try {
      derived = orbit_derived_batch(packStates(points), ctx.mu, ctx.bodyRadius);
    } catch (e) {
      console.warn("RrdFileAdapter: could not derive orbital values:", e);
      derived = new Float64Array(points.length * ORBIT_DERIVED_STRIDE).fill(Number.NaN);
    }
    const orbitPoints = toOrbitPoints(points, derived);
    // `IngestBuffer` only drains, so an E2E cannot read a point back out of it.
    // Expose this load's first one instead: what the derived fields hold there
    // is what this whole path exists to produce. Per adapter, so opening a
    // second recording replaces it rather than keeping the first file's point.
    if (import.meta.env.DEV && orbitPoints.length > 0 && !this.debugPointExposed) {
      (window as unknown as Record<string, unknown>).__debug_rrd_first_point = orbitPoints[0];
      this.debugPointExposed = true;
    }
    return orbitPoints;
  }

  private handleWorkerMessage(msg: RrdWorkerMessage): void {
    const id = this.sourceId;

    switch (msg.type) {
      case "metadata": {
        this.pendingMetadata = msg.metadata;
        this.derivedContext = orbitDerivedContext(msg.metadata);
        break;
      }

      case "chunk": {
        // Collect entity paths from ALL chunks before emitting info
        for (const p of msg.points) {
          if (p.entityPath) this.pendingEntityPaths.add(p.entityPath);
        }

        // Estimate dt from consecutive points of the SAME entity.
        // Persists last-seen timestamps across chunks so chunk boundaries
        // don't prevent detection in multi-entity recordings.
        if (this.estimatedDt === 10) {
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

        // Convert and emit points as history-chunk (info emitted later on done)
        const orbitPoints: OrbitPoint[] = this.deriveChunk(msg.points);
        this.onEvent(id, {
          kind: "history-chunk",
          points: orbitPoints,
          done: false,
        });

        // When done: emit info (with all entity paths known) then complete
        if (msg.done) {
          if (!this.infoEmitted && this.pendingMetadata) {
            const info = rrdMetadataToSimInfo(
              this.pendingMetadata,
              this.file.name,
              this.estimatedDt,
              [...this.pendingEntityPaths],
            );
            this.onEvent(id, { kind: "info", info });
            this.infoEmitted = true;
          }
          this.onEvent(id, { kind: "history-chunk", points: [], done: true });
          this.onEvent(id, { kind: "complete" });
          if (this.worker) {
            this.worker.terminate();
            this.worker = null;
          }
        }
        break;
      }

      case "error":
        this.onEvent(id, { kind: "error", message: msg.message });
        // Worker errors are fatal — no further messages will arrive.
        if (this.worker) {
          this.worker.terminate();
          this.worker = null;
        }
        break;
    }
  }
}
