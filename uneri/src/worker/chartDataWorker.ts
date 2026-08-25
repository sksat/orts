/**
 * Chart Data Web Worker entry point.
 *
 * Owns DuckDB and runs the cold/hot tick loop autonomously.
 * Receives data points (as row tuples) from the main thread,
 * inserts them into DuckDB, and periodically queries + merges
 * to produce ChartDataMap which is transferred back via zero-copy.
 *
 * The logic lives in `ChartDataCore` so it can be tested without a Worker.
 */

import { initDuckDB } from "../db/duckdb.js";
import { ChartDataCore } from "./chartDataCore.js";
import type { MainToWorkerMessage } from "./protocol.js";

const core = new ChartDataCore({
  post: (msg, transfer) => {
    if (transfer) {
      postMessage(msg, { transfer });
    } else {
      postMessage(msg);
    }
  },
  initDb: (options) => initDuckDB(options),
});

self.onmessage = (e: MessageEvent<MainToWorkerMessage>) => {
  core.handle(e.data);
};
