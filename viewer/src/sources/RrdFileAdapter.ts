/**
 * RRD file SourceAdapter.
 *
 * Reads an RRD file via FileReader.readAsArrayBuffer(), sends the bytes
 * to a Web Worker (which decodes via rrd-wasm WASM), and translates
 * worker messages into SourceEvents.
 */

import type { OrbitPoint } from "../orbit.js";
import { rrdMetadataToSimInfo } from "./normalizeMetadata.js";
import type { RrdPointOut, RrdWorkerMessage } from "./rrdParseLogic.js";
import type {
  SourceAdapter,
  SourceCapabilities,
  SourceConnectionState,
  SourceEventHandler,
  SourceId,
  SourceSpec,
} from "./types.js";

function rrdPointToOrbitPoint(p: RrdPointOut): OrbitPoint {
  return {
    t: p.t,
    x: p.x,
    y: p.y,
    z: p.z,
    vx: p.vx,
    vy: p.vy,
    vz: p.vz,
    entityPath: p.entityPath ?? undefined,
    // Keplerian elements — not available from RRD decode (raw state vectors only)
    a: 0,
    e: 0,
    inc: 0,
    raan: 0,
    omega: 0,
    nu: 0,
    altitude: 0,
    specific_energy: 0,
    angular_momentum: 0,
    velocity_mag: Math.sqrt(p.vx * p.vx + p.vy * p.vy + p.vz * p.vz),
    // Attitude (optional)
    qw: p.qw,
    qx: p.qx,
    qy: p.qy,
    qz: p.qz,
    wx: p.wx,
    wy: p.wy,
    wz: p.wz,
  };
}

export class RrdFileAdapter implements SourceAdapter {
  readonly sourceId: SourceId;
  readonly spec: SourceSpec & { type: "rrd-file" };
  readonly capabilities: SourceCapabilities = {
    live: false,
    control: false,
    rangeQuery: false,
  };

  private worker: Worker | null = null;
  private reader: FileReader | null = null;
  private _connectionState: SourceConnectionState = "disconnected";
  private onEvent: SourceEventHandler;
  private file: File;
  private estimatedDt = 10;
  private stopped = false;

  constructor(sourceId: SourceId, file: File, onEvent: SourceEventHandler) {
    this.sourceId = sourceId;
    this.spec = { type: "rrd-file", file };
    this.onEvent = onEvent;
    this.file = file;
  }

  get connectionState(): SourceConnectionState {
    return this._connectionState;
  }

  start(): void {
    this.estimatedDt = 10;
    this.pendingEntityPaths = new Set();
    this.infoEmitted = false;
    this.stopped = false;
    this._connectionState = "loading";

    const reader = new FileReader();
    this.reader = reader;
    reader.onload = () => {
      if (this.stopped) return;
      const buffer = reader.result as ArrayBuffer;
      this.startWorker(buffer);
    };
    reader.onerror = () => {
      if (this.stopped) return;
      this._connectionState = "error";
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
    this._connectionState = "disconnected";
  }

  private startWorker(buffer: ArrayBuffer): void {
    this.worker = new Worker(new URL("./rrdParseWorker.ts", import.meta.url), { type: "module" });

    this.worker.onmessage = (e: MessageEvent<RrdWorkerMessage>) => {
      this.handleWorkerMessage(e.data);
    };

    this.worker.onerror = (err: ErrorEvent) => {
      this._connectionState = "error";
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

  private handleWorkerMessage(msg: RrdWorkerMessage): void {
    const id = this.sourceId;

    switch (msg.type) {
      case "metadata": {
        this.pendingMetadata = msg.metadata;
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
        const orbitPoints: OrbitPoint[] = msg.points.map(rrdPointToOrbitPoint);
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
          this._connectionState = "complete";
          if (this.worker) {
            this.worker.terminate();
            this.worker = null;
          }
        }
        break;
      }

      case "error":
        this.onEvent(id, { kind: "error", message: msg.message });
        this._connectionState = "error";
        break;
    }
  }
}
