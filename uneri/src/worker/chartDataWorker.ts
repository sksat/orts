/**
 * Chart Data Web Worker.
 *
 * Owns DuckDB and runs the cold/hot tick loop autonomously.
 * Receives data points (as row tuples) from the main thread,
 * inserts them into DuckDB, and periodically queries + merges
 * to produce ChartDataMap which is transferred back via zero-copy.
 *
 * This is a direct port of the useTimeSeriesStore tick logic.
 */

import type { AsyncDuckDBConnection } from "@duckdb/duckdb-wasm";
import { initDuckDB } from "../db/duckdb.js";
import {
  buildInsertSQLFromRows,
  COMPACT_DEFAULTS,
  clearTable,
  compactTable,
  createTable,
  queryDerived,
  queryDerivedIncremental,
} from "../db/store.js";
import { computeTMin, DISPLAY_MAX_POINTS, type TimeRange } from "../hooks/useTimeSeriesStore.js";
import type { ChartDataMap, TableSchema } from "../types.js";
import { mergeChartData, trimChartDataLeft } from "../utils/mergeChartData.js";
import type {
  MainToWorkerMessage,
  RowTuple,
  WorkerTableSchema,
  WorkerToMainMessage,
} from "./protocol.js";

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

let conn: AsyncDuckDBConnection | null = null;
let schema: WorkerTableSchema | null = null;
let workerDisposed = false;
let timeRange: TimeRange = null;
let maxPoints: number = DISPLAY_MAX_POINTS;
let latestT = -Infinity;
let tickTimer: ReturnType<typeof setTimeout> | null = null;

// Cold/hot state (mirroring useTimeSeriesStore)
let coldSnapshot: ChartDataMap | null = null;
let coldTMax = -Infinity;
let hotBuffer: ChartDataMap | null = null;
let ticksSinceCold = 0;
let coldRefreshNeeded = true;
let coldQueryCount = 0;
let hasData = false;

let TICK_INTERVAL = 250;
let COLD_REFRESH_EVERY_N = 20;
let HOT_ROW_BUDGET = 500;
const COMPACT_EVERY_N = 5;
const COMPACT_COOLDOWN_AFTER_REBUILD = 5;
let compactCooldown = 0;

// Ingest queue: rows buffered between ticks
let ingestQueue: RowTuple[] = [];
let ingestRetryCount = 0;
const MAX_INGEST_RETRIES = 3;

const BATCH_SIZE = 1000;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function post(msg: WorkerToMainMessage, transfer?: Transferable[]) {
  if (transfer) {
    postMessage(msg, { transfer });
  } else {
    postMessage(msg);
  }
}

/** Convert ChartDataMap to transferable message and send. */
function sendChartData(data: ChartDataMap) {
  const keys: string[] = [];
  const buffers: ArrayBuffer[] = [];

  for (const key of Object.keys(data)) {
    const arr = data[key];
    if (!arr) continue;
    // Copy the Float64Array so we can transfer ownership of the buffer.
    // (The original may be a subarray sharing a larger buffer.)
    const copy = new Float64Array(arr.length);
    copy.set(arr);
    keys.push(key);
    buffers.push(copy.buffer as ArrayBuffer);
  }

  post({ type: "chart-data", keys, buffers }, buffers);
}

/** Build a minimal TableSchema (with dummy toRow) for store.ts functions. */
function toTableSchema(ws: WorkerTableSchema): TableSchema {
  return {
    ...ws,
    toRow: () => {
      throw new Error("toRow should not be called in worker");
    },
  };
}

// ---------------------------------------------------------------------------
// Tick loop (cold/hot query cycle)
// ---------------------------------------------------------------------------

async function tick() {
  if (!conn || !schema) return;

  const tableSchema = toTableSchema(schema);

  // 1. Flush ingest queue
  if (ingestQueue.length > 0) {
    const rows = ingestQueue;
    ingestQueue = [];
    try {
      for (let i = 0; i < rows.length; i += BATCH_SIZE) {
        const batch = rows.slice(i, i + BATCH_SIZE);
        const sql = buildInsertSQLFromRows(schema.tableName, batch);
        if (sql) await conn.query(sql);
      }
      hasData = true;
      ingestRetryCount = 0;
    } catch (e) {
      console.warn("chartDataWorker: insert failed:", e);
      if (ingestRetryCount < MAX_INGEST_RETRIES) {
        // Re-queue failed rows for retry
        ingestQueue = rows.concat(ingestQueue);
        ingestRetryCount++;
      } else {
        // Drop failed rows after max retries to avoid infinite loop
        console.warn(
          "chartDataWorker: dropping",
          rows.length,
          "rows after",
          MAX_INGEST_RETRIES,
          "retries",
        );
        ingestRetryCount = 0;
      }
    }
  }

  // 2. Cold/hot query cycle
  if (!hasData) return;

  ticksSinceCold++;
  const derivedNames = schema.derived.map((d) => d.name);

  const needsCold =
    coldRefreshNeeded ||
    ticksSinceCold >= COLD_REFRESH_EVERY_N ||
    (hotBuffer != null && hotBuffer.t.length > HOT_ROW_BUDGET);

  if (needsCold) {
    // COLD PATH: full downsampled query
    try {
      const tMin = computeTMin(timeRange, latestT);
      coldSnapshot = await queryDerived(conn, tableSchema, tMin, maxPoints);
      coldTMax = coldSnapshot.t.length > 0 ? coldSnapshot.t[coldSnapshot.t.length - 1] : -Infinity;
      hotBuffer = null;
      ticksSinceCold = 0;
      coldRefreshNeeded = false;

      // Compaction
      coldQueryCount++;
      if (compactCooldown > 0) {
        compactCooldown--;
      } else if (coldQueryCount % COMPACT_EVERY_N === 0) {
        const compacted = await compactTable(conn, tableSchema, COMPACT_DEFAULTS);
        if (compacted) coldRefreshNeeded = true;
      }
    } catch (e) {
      console.warn("chartDataWorker: cold query failed:", e);
    }
  } else {
    // HOT PATH: incremental query
    try {
      const tMin = computeTMin(timeRange, latestT);
      const hotLowerBound = tMin != null ? Math.max(coldTMax, tMin) : coldTMax;
      hotBuffer = await queryDerivedIncremental(conn, tableSchema, hotLowerBound);
    } catch (e) {
      console.warn("chartDataWorker: hot query failed:", e);
    }
  }

  // 3. Merge + trim → send
  if (coldSnapshot != null) {
    let merged = mergeChartData(coldSnapshot, hotBuffer, derivedNames);
    if (timeRange != null) {
      merged = trimChartDataLeft(merged, latestT - timeRange, derivedNames);
    }
    sendChartData(merged);
  }
}

/** Schedule the next tick after the current one completes (setTimeout chain). */
function scheduleNextTick() {
  tickTimer = setTimeout(() => {
    tick()
      .catch((err) => {
        console.warn("chartDataWorker: tick error:", err);
      })
      .finally(() => {
        // Only schedule next tick if not disposed
        if (tickTimer !== null) {
          scheduleNextTick();
        }
      });
  }, TICK_INTERVAL);
}

// ---------------------------------------------------------------------------
// Message handler
// ---------------------------------------------------------------------------

self.onmessage = async (e: MessageEvent<MainToWorkerMessage>) => {
  const msg = e.data;
  if (workerDisposed && msg.type !== "dispose") return;

  switch (msg.type) {
    case "init": {
      try {
        schema = msg.schema;
        if (msg.tickInterval != null) TICK_INTERVAL = msg.tickInterval;
        if (msg.coldRefreshEveryN != null) COLD_REFRESH_EVERY_N = msg.coldRefreshEveryN;
        if (msg.hotRowBudget != null) HOT_ROW_BUDGET = msg.hotRowBudget;
        const db = await initDuckDB();
        conn = await db.connect();
        await createTable(conn, toTableSchema(schema));

        // Start autonomous tick loop (setTimeout chain to avoid concurrent ticks)
        scheduleNextTick();

        post({ type: "ready" });
      } catch (e) {
        post({
          type: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      }
      break;
    }

    case "ingest": {
      ingestQueue = ingestQueue.concat(msg.rows);
      latestT = msg.latestT;
      break;
    }

    case "rebuild": {
      if (!conn || !schema) break;
      try {
        await clearTable(conn, toTableSchema(schema));
        // Insert all rows
        for (let i = 0; i < msg.rows.length; i += BATCH_SIZE) {
          const batch = msg.rows.slice(i, i + BATCH_SIZE);
          const sql = buildInsertSQLFromRows(schema.tableName, batch);
          if (sql) await conn.query(sql);
        }
        hasData = msg.rows.length > 0;
        latestT = msg.latestT;
        compactCooldown = COMPACT_COOLDOWN_AFTER_REBUILD;
        coldRefreshNeeded = true;
        hotBuffer = null;
        ingestQueue = [];
      } catch (e) {
        console.warn("chartDataWorker: rebuild failed:", e);
      }
      break;
    }

    case "configure": {
      const rangeChanged = msg.timeRange !== timeRange;
      const pointsChanged = msg.maxPoints !== maxPoints;
      timeRange = msg.timeRange;
      maxPoints = msg.maxPoints;
      if (rangeChanged || pointsChanged) {
        coldRefreshNeeded = true;
      }
      break;
    }

    case "debug-query": {
      if (!conn || !schema) {
        post({ type: "debug-result", id: msg.id, result: 0 });
        break;
      }
      try {
        const result = await conn.query(`SELECT COUNT(*) AS cnt FROM ${schema.tableName}`);
        const count = Number(result.getChildAt(0)!.get(0));
        post({ type: "debug-result", id: msg.id, result: count });
      } catch {
        post({ type: "debug-result", id: msg.id, result: -1 });
      }
      break;
    }

    case "zoom-query": {
      if (!conn || !schema) {
        post({ type: "zoom-result", id: msg.id, keys: [], buffers: [] });
        break;
      }
      try {
        const tableSchema = toTableSchema(schema);
        const result = await queryDerived(conn, tableSchema, msg.tMin, msg.maxPoints, msg.tMax);
        const keys: string[] = [];
        const buffers: ArrayBuffer[] = [];
        for (const key of Object.keys(result)) {
          const arr = result[key];
          if (!arr) continue;
          const copy = new Float64Array(arr.length);
          copy.set(arr);
          keys.push(key);
          buffers.push(copy.buffer as ArrayBuffer);
        }
        post({ type: "zoom-result", id: msg.id, keys, buffers }, buffers);
      } catch {
        post({ type: "zoom-result", id: msg.id, keys: [], buffers: [] });
      }
      break;
    }

    case "dispose": {
      workerDisposed = true;
      if (tickTimer != null) {
        clearTimeout(tickTimer);
        tickTimer = null;
      }
      if (conn) {
        try {
          await conn.close();
        } catch {
          // ignore
        }
        conn = null;
      }
      break;
    }
  }
};
