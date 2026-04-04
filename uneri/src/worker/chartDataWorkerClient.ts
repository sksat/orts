/**
 * Main-thread typed wrapper for the chart data Web Worker.
 *
 * Manages Worker lifecycle, message buffering before ready,
 * and typed callbacks for chart data and errors.
 */

import type { TimeRange } from "../hooks/useTimeSeriesStore.js";
import type { ChartDataMap } from "../types.js";
import type {
  MainToWorkerMessage,
  RowTuple,
  WorkerTableSchema,
  WorkerToMainMessage,
} from "./protocol.js";

export class ChartDataWorkerClient {
  private worker: Worker;
  private ready = false;
  private disposed = false;
  private pendingMessages: MainToWorkerMessage[] = [];
  private onDataCallback: ((data: ChartDataMap) => void) | null = null;
  private onErrorCallback: ((message: string) => void) | null = null;

  // Debug/zoom query: pending promise resolvers keyed by request id
  private queryId = 0;
  private debugResolvers = new Map<number, (value: number) => void>();
  private zoomResolvers = new Map<number, (value: ChartDataMap) => void>();

  constructor() {
    this.worker = new Worker(new URL("./chartDataWorker.ts", import.meta.url), { type: "module" });
    this.worker.onmessage = (e: MessageEvent<WorkerToMainMessage>) => {
      this.handleMessage(e.data);
    };
    this.worker.onerror = (e) => {
      this.onErrorCallback?.(`Worker error: ${e.message}`);
      // Clean up pending resolvers so callers don't hang forever
      this.resolveAllPending();
    };
  }

  /** Initialize the Worker with a table schema and optional tick parameters. */
  init(
    schema: WorkerTableSchema,
    opts?: { tickInterval?: number; coldRefreshEveryN?: number; hotRowBudget?: number },
  ): void {
    this.send({ type: "init", schema, ...opts });
  }

  /** Send data points (as pre-converted row tuples) to the Worker. */
  ingest(rows: RowTuple[], latestT: number): void {
    if (rows.length === 0) return;
    this.send({ type: "ingest", rows, latestT });
  }

  /** Signal a full table rebuild with new data. */
  rebuild(rows: RowTuple[], latestT: number): void {
    this.send({ type: "rebuild", rows, latestT });
  }

  /** Update time range and max points configuration. */
  configure(timeRange: TimeRange, maxPoints: number): void {
    this.send({ type: "configure", timeRange, maxPoints });
  }

  /** Register callback for receiving chart data from the Worker. */
  onData(callback: (data: ChartDataMap) => void): void {
    this.onDataCallback = callback;
  }

  /** Register callback for Worker errors. */
  onError(callback: (message: string) => void): void {
    this.onErrorCallback = callback;
  }

  /** Query the row count in DuckDB (for debug/testing). */
  queryRowCount(): Promise<number> {
    if (this.disposed) return Promise.resolve(0);
    return new Promise((resolve) => {
      const id = this.queryId++;
      this.debugResolvers.set(id, resolve);
      this.send({ type: "debug-query", id, query: "row-count" });
    });
  }

  /** Run a zoom query on the Worker's DuckDB and return the result. */
  zoomQuery(tMin: number, tMax: number, maxPoints: number): Promise<ChartDataMap> {
    if (this.disposed) return Promise.resolve({ t: new Float64Array(0) });
    return new Promise((resolve) => {
      const id = this.queryId++;
      this.zoomResolvers.set(id, resolve);
      this.send({ type: "zoom-query", id, tMin, tMax, maxPoints });
    });
  }

  /** Dispose the Worker and release resources. */
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.send({ type: "dispose" });
    this.resolveAllPending();
    this.pendingMessages = [];
    // Give the worker a moment to clean up, then terminate
    setTimeout(() => {
      this.worker.terminate();
    }, 100);
  }

  // -----------------------------------------------------------------------
  // Private
  // -----------------------------------------------------------------------

  /** Resolve all pending query Promises so callers don't hang. */
  private resolveAllPending(): void {
    for (const resolver of this.debugResolvers.values()) {
      resolver(0);
    }
    this.debugResolvers.clear();
    const emptyData: ChartDataMap = { t: new Float64Array(0) };
    for (const resolver of this.zoomResolvers.values()) {
      resolver(emptyData);
    }
    this.zoomResolvers.clear();
  }

  private send(msg: MainToWorkerMessage): void {
    if (this.disposed && msg.type !== "dispose") return;

    // Buffer messages until Worker is ready (except init and dispose)
    if (!this.ready && msg.type !== "init" && msg.type !== "dispose") {
      this.pendingMessages.push(msg);
      return;
    }

    this.worker.postMessage(msg);
  }

  private handleMessage(msg: WorkerToMainMessage): void {
    if (this.disposed) return;
    switch (msg.type) {
      case "ready": {
        this.ready = true;
        // Flush buffered messages
        for (const pending of this.pendingMessages) {
          this.worker.postMessage(pending);
        }
        this.pendingMessages = [];
        break;
      }

      case "chart-data": {
        // Reconstruct ChartDataMap from transferred ArrayBuffers
        const data: ChartDataMap = { t: new Float64Array(0) };
        for (let i = 0; i < msg.keys.length; i++) {
          data[msg.keys[i]] = new Float64Array(msg.buffers[i]);
        }
        this.onDataCallback?.(data);
        break;
      }

      case "error": {
        this.onErrorCallback?.(msg.message);
        break;
      }

      case "debug-result": {
        const resolver = this.debugResolvers.get(msg.id);
        if (resolver) {
          this.debugResolvers.delete(msg.id);
          resolver(msg.result);
        }
        break;
      }

      case "zoom-result": {
        const resolver = this.zoomResolvers.get(msg.id);
        if (resolver) {
          this.zoomResolvers.delete(msg.id);
          const data: ChartDataMap = { t: new Float64Array(0) };
          for (let i = 0; i < msg.keys.length; i++) {
            data[msg.keys[i]] = new Float64Array(msg.buffers[i]);
          }
          resolver(data);
        }
        break;
      }
    }
  }
}
