/**
 * Main-thread typed wrapper for the multi-satellite chart data Web Worker.
 */

import type { TimeRange } from "../hooks/useTimeSeriesStore.js";
import type {
  MultiMainToWorkerMessage,
  MultiWorkerToMainMessage,
  RowTuple,
  WorkerSatelliteConfig,
  WorkerTableSchema,
} from "./protocol.js";

/** Deserialized multi-series data for a single metric. */
export interface MultiSeriesResult {
  t: Float64Array;
  values: Float64Array[];
  series: Array<{ label: string; color: string }>;
}

/** Map from metric name to multi-series data. */
export type MultiChartDataResult = {
  [metricName: string]: MultiSeriesResult | null;
};

export class MultiChartDataWorkerClient {
  private worker: Worker;
  private ready = false;
  private disposed = false;
  private pendingMessages: MultiMainToWorkerMessage[] = [];
  private onDataCallback: ((data: MultiChartDataResult) => void) | null = null;
  private onErrorCallback: ((message: string) => void) | null = null;

  constructor() {
    this.worker = new Worker(new URL("./multiChartDataWorker.ts", import.meta.url), {
      type: "module",
    });
    this.worker.onmessage = (e: MessageEvent<MultiWorkerToMainMessage>) => {
      this.handleMessage(e.data);
    };
    this.worker.onerror = (e) => {
      this.onErrorCallback?.(`Worker error: ${e.message}`);
    };
  }

  init(
    baseSchema: WorkerTableSchema,
    satelliteConfigs: WorkerSatelliteConfig[],
    metricNames: string[],
    opts?: { tickInterval?: number; queryEveryN?: number; compactEveryN?: number },
  ): void {
    this.send({
      type: "multi-init",
      baseSchema,
      satelliteConfigs,
      metricNames,
      ...opts,
    });
  }

  ingest(satelliteId: string, rows: RowTuple[], latestT: number): void {
    if (rows.length === 0) return;
    this.send({ type: "multi-ingest", satelliteId, rows, latestT });
  }

  rebuild(satelliteId: string, rows: RowTuple[], latestT: number): void {
    this.send({ type: "multi-rebuild", satelliteId, rows, latestT });
  }

  configure(timeRange: TimeRange, maxPoints: number): void {
    this.send({ type: "multi-configure", timeRange, maxPoints });
  }

  updateConfigs(satelliteConfigs: WorkerSatelliteConfig[], metricNames: string[]): void {
    this.send({ type: "multi-update-configs", satelliteConfigs, metricNames });
  }

  onData(callback: (data: MultiChartDataResult) => void): void {
    this.onDataCallback = callback;
  }

  onError(callback: (message: string) => void): void {
    this.onErrorCallback = callback;
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.send({ type: "dispose" });
    this.pendingMessages = [];
    setTimeout(() => {
      this.worker.terminate();
    }, 100);
  }

  // -----------------------------------------------------------------------
  // Private
  // -----------------------------------------------------------------------

  private send(msg: MultiMainToWorkerMessage): void {
    if (this.disposed && msg.type !== "dispose") return;
    if (!this.ready && msg.type !== "multi-init" && msg.type !== "dispose") {
      this.pendingMessages.push(msg);
      return;
    }
    this.worker.postMessage(msg);
  }

  private handleMessage(msg: MultiWorkerToMainMessage): void {
    if (this.disposed) return;
    switch (msg.type) {
      case "ready": {
        this.ready = true;
        for (const pending of this.pendingMessages) {
          this.worker.postMessage(pending);
        }
        this.pendingMessages = [];
        break;
      }

      case "multi-chart-data": {
        // Deserialize transferred ArrayBuffers into MultiChartDataResult
        const result: MultiChartDataResult = {};
        for (const metric of msg.metrics) {
          const t = new Float64Array(metric.buffers[0]);
          const values: Float64Array[] = [];
          for (let i = 1; i < metric.buffers.length; i++) {
            values.push(new Float64Array(metric.buffers[i]));
          }
          const series = metric.seriesLabels.map((label, i) => ({
            label,
            color: metric.seriesColors[i],
          }));
          result[metric.metricName] = { t, values, series };
        }
        this.onDataCallback?.(result);
        break;
      }

      case "error": {
        this.onErrorCallback?.(msg.message);
        break;
      }
    }
  }
}
