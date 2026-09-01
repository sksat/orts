/**
 * RRD file SourceAdapter.
 *
 * Reads an RRD file via FileReader.readAsArrayBuffer(), sends the bytes
 * to a Web Worker (which decodes via rrd-wasm WASM), and translates
 * worker messages into SourceEvents.
 */

import type { OrbitPoint } from "../orbit.js";
import { initArika, orbit_derived_batch } from "../wasm/arikaInit.js";
import type { BodyCatalog } from "./bodyCatalog.js";
import { type CentralBody, describeCentralBodyError, resolveCentralBody } from "./centralBody.js";
import { rrdMetadataToSimInfo } from "./normalizeMetadata.js";
import { ORBIT_DERIVED_STRIDE, packStates, toOrbitPoints } from "./rrdOrbitDerived.js";
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
  /**
   * The body the derived values are measured against, resolved once from the
   * recording's metadata. The Worker sends metadata before the first chunk, so
   * this is set by then; a chunk arriving without it is a protocol violation
   * rather than a reason to reach for Earth.
   */
  private centralBody: CentralBody | null = null;
  /** Whether this load has already published its first point for the E2E. */
  private debugPointExposed = false;
  /**
   * Which load is current. `stopped` cannot stand in for this: a restart clears
   * it, so a callback of the load before would read it as its own go-ahead.
   */
  private loadGeneration = 0;

  /**
   * `bodyCatalog` extends the bodies whose constants a recording may leave out,
   * for a consumer simulating around one this viewer does not ship.
   */
  constructor(
    sourceId: SourceId,
    file: File,
    onEvent: SourceEventHandler,
    bodyCatalog?: BodyCatalog,
  ) {
    this.sourceId = sourceId;
    this.onEvent = onEvent;
    this.file = file;
    this.bodyCatalog = bodyCatalog;
  }

  private bodyCatalog: BodyCatalog | undefined;

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
    // Every other per-load field is reset here, and this one holds the central
    // body the derived values are computed against. The worker posts metadata
    // before any chunk, so a second load overwrites it in practice; keeping it
    // anyway would make that ordering load-bearing, and a chunk arriving first
    // would be derived against the previous recording's body.
    this.centralBody = null;

    const generation = ++this.loadGeneration;
    const isCurrent = () => !this.stopped && this.loadGeneration === generation;

    const reader = new FileReader();
    this.reader = reader;
    reader.onload = () => {
      if (!isCurrent()) return;
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
          // A restart while this was pending owns the adapter now. Starting a
          // worker here would put the previous file's buffer behind the current
          // load's worker reference, leaving both workers reporting into it and
          // neither of them stoppable.
          if (!isCurrent()) return;
          this.startWorker(buffer);
        });
    };
    reader.onerror = () => {
      if (!isCurrent()) return;
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
   *
   * `null` where the chunk cannot be read at all, the adapter having reported
   * why and stopped.
   */
  private deriveChunk(points: readonly RrdPointOut[]): OrbitPoint[] | null {
    const body = this.centralBody;
    if (body == null) {
      // The Worker posts metadata first, so reaching here means the recording
      // is being read against a body nobody named. Deriving anyway would put a
      // number on the chart that stands for nothing.
      this.onEvent(this.sourceId, {
        kind: "error",
        message: `${this.file.name}: the recording's points arrived before it said which body they are around`,
      });
      this.stop();
      return null;
    }
    let derived: Float64Array;
    try {
      derived = orbit_derived_batch(packStates(points), body.mu, body.bodyRadius);
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
        // Resolved here, once, and shared by the derived values and the
        // `SimInfo` the charts are built from. Waiting until the recording is
        // complete would put its points on screen before finding out they
        // cannot be measured.
        const resolved = resolveCentralBody(
          {
            bodyId: msg.metadata.body_name,
            mu: msg.metadata.mu,
            bodyRadius: msg.metadata.body_radius,
          },
          this.bodyCatalog,
        );
        if (!resolved.ok) {
          this.onEvent(id, {
            kind: "error",
            message: `${this.file.name}: ${describeCentralBodyError(resolved.error)}`,
          });
          this.stop();
          return;
        }
        this.pendingMetadata = msg.metadata;
        this.centralBody = resolved.body;
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
        const orbitPoints = this.deriveChunk(msg.points);
        if (orbitPoints == null) return;
        this.onEvent(id, {
          kind: "history-chunk",
          points: orbitPoints,
          done: false,
        });

        // When done: emit info (with all entity paths known) then complete
        if (msg.done) {
          if (!this.infoEmitted && this.pendingMetadata && this.centralBody) {
            const info = rrdMetadataToSimInfo(
              this.pendingMetadata,
              this.file.name,
              this.estimatedDt,
              [...this.pendingEntityPaths],
              this.centralBody,
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
