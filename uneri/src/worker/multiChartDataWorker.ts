/**
 * Multi-satellite Chart Data Web Worker entry point.
 *
 * Manages N DuckDB tables (one per satellite), runs per-satellite queries
 * with unified tMin/tMax, aligns results via alignTimeSeries, and produces
 * serialized MultiChartDataMap transferred back to the main thread.
 *
 * The logic lives in `MultiChartDataCore` so it can be tested without a Worker.
 */

import { initDuckDB } from "../db/duckdb.js";
import { MultiChartDataCore } from "./multiChartDataCore.js";
import type { MultiMainToWorkerMessage } from "./protocol.js";

const core = new MultiChartDataCore({
  post: (msg, transfer) => {
    if (transfer) {
      postMessage(msg, { transfer });
    } else {
      postMessage(msg);
    }
  },
  initDb: (options) => initDuckDB(options),
});

self.onmessage = (e: MessageEvent<MultiMainToWorkerMessage>) => {
  core.handle(e.data);
};
