/**
 * Multi-satellite Chart Data Web Worker.
 *
 * Manages N DuckDB tables (one per satellite), runs per-satellite queries
 * with unified tMin/tMax, aligns results via alignTimeSeries, and produces
 * serialized MultiChartDataMap transferred back to the main thread.
 *
 * Direct port of useMultiSatelliteStore tick logic.
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
} from "../db/store.js";
import type { TimeRange } from "../hooks/useTimeSeriesStore.js";
import type { ChartDataMap, TableSchema } from "../types.js";
import { alignTimeSeries, type NamedTimeSeries } from "../utils/alignTimeSeries.js";
import type {
  MultiMainToWorkerMessage,
  MultiWorkerToMainMessage,
  RowTuple,
  SerializedMultiSeriesData,
  WorkerSatelliteConfig,
  WorkerTableSchema,
} from "./protocol.js";

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

let conn: AsyncDuckDBConnection | null = null;
let baseSchema: WorkerTableSchema | null = null;
let satelliteConfigs: WorkerSatelliteConfig[] = [];
let metricNames: string[] = [];
let timeRange: TimeRange = null;
let maxPoints = 2000;
let workerDisposed = false;
let tickTimer: ReturnType<typeof setTimeout> | null = null;

let TICK_INTERVAL = 500;
let QUERY_EVERY_N = 4;
let COMPACT_EVERY_N = 20;
const COMPACT_COOLDOWN_AFTER_REBUILD = 5;
const BATCH_SIZE = 1000;

// Per-satellite state
const createdTables = new Set<string>();
const hasData = new Set<string>();
const compactCooldowns = new Map<string, number>();
/** Per-satellite ingest queues: satelliteId → rows. */
const ingestQueues = new Map<string, RowTuple[]>();
/** Per-satellite latestT (for unified tMin computation). */
const latestTs = new Map<string, number>();

let tickCount = 0;
let queryCount = 0;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function post(msg: MultiWorkerToMainMessage, transfer?: Transferable[]) {
  if (transfer) {
    postMessage(msg, { transfer });
  } else {
    postMessage(msg);
  }
}

function makeSatelliteTableName(satelliteId: string): string {
  const safeName = satelliteId.replace(/[^a-zA-Z0-9_]/g, "_");
  return `orbit_${safeName}`;
}

function toTableSchema(tableName: string): TableSchema {
  if (!baseSchema) throw new Error("baseSchema not initialized");
  return {
    ...baseSchema,
    tableName,
    toRow: () => {
      throw new Error("toRow should not be called in worker");
    },
  };
}

async function ensureTable(satelliteId: string) {
  if (createdTables.has(satelliteId)) return;
  if (!conn) return;
  const tableName = makeSatelliteTableName(satelliteId);
  await createTable(conn, toTableSchema(tableName));
  createdTables.add(satelliteId);
}

/** Compute unified tMin from all satellite latestTs. */
function computeUnifiedTMin(): number | undefined {
  if (timeRange == null) return undefined;
  let max = -Infinity;
  for (const t of latestTs.values()) {
    if (t > max) max = t;
  }
  if (!Number.isFinite(max)) return undefined;
  return max - timeRange;
}

/**
 * Build the serialized multi-series payload from per-satellite
 * `ChartDataMap` results. Shared between the periodic tick loop and
 * one-shot zoom queries.
 */
function buildMultiSeriesPayload(
  perSatData: Map<string, ChartDataMap>,
  configs: WorkerSatelliteConfig[],
  metrics: string[],
): { metrics: SerializedMultiSeriesData[]; transfers: ArrayBuffer[] } {
  const activeSats = configs.filter((cfg) => perSatData.has(cfg.id));
  const serializedMetrics: SerializedMultiSeriesData[] = [];
  const allTransfers: ArrayBuffer[] = [];

  if (activeSats.length === 0) {
    return { metrics: serializedMetrics, transfers: allTransfers };
  }

  for (const metric of metrics) {
    const inputs: NamedTimeSeries[] = [];
    const seriesLabels: string[] = [];
    const seriesColors: string[] = [];

    for (const sat of activeSats) {
      const data = perSatData.get(sat.id);
      if (!data || !data[metric]) continue;
      inputs.push({ label: sat.label, t: data.t, values: data[metric] });
      seriesLabels.push(sat.label);
      seriesColors.push(sat.color);
    }

    if (inputs.length === 0) continue;

    const aligned = alignTimeSeries(inputs);

    // Pack: [t, values[0], values[1], ...]
    const buffers: ArrayBuffer[] = [];
    const tCopy = new Float64Array(aligned.t.length);
    tCopy.set(aligned.t);
    buffers.push(tCopy.buffer as ArrayBuffer);
    allTransfers.push(tCopy.buffer as ArrayBuffer);

    for (const vals of aligned.values) {
      const copy = new Float64Array(vals.length);
      copy.set(vals);
      buffers.push(copy.buffer as ArrayBuffer);
      allTransfers.push(copy.buffer as ArrayBuffer);
    }

    serializedMetrics.push({ metricName: metric, seriesLabels, seriesColors, buffers });
  }

  return { metrics: serializedMetrics, transfers: allTransfers };
}

/** Serialize MultiChartDataMap → transferable tick broadcast. */
function sendMultiChartData(
  perSatData: Map<string, ChartDataMap>,
  configs: WorkerSatelliteConfig[],
  metrics: string[],
) {
  const { metrics: serializedMetrics, transfers } = buildMultiSeriesPayload(
    perSatData,
    configs,
    metrics,
  );
  if (serializedMetrics.length > 0) {
    post({ type: "multi-chart-data", metrics: serializedMetrics }, transfers);
  }
}

/**
 * Run an ad-hoc zoom query for the absolute window `[tMin, tMax]` against
 * every satellite's DuckDB table and post a one-shot `multi-zoom-result`
 * correlated by `id`. Empty results still post (with no metrics) so the
 * client-side promise always resolves.
 */
async function handleMultiZoomQuery(id: number, tMin: number, tMax: number, zoomMaxPoints: number) {
  if (!conn || !baseSchema) {
    post({ type: "multi-zoom-result", id, metrics: [] });
    return;
  }

  const perSatData = new Map<string, ChartDataMap>();
  for (const satId of hasData) {
    const tableName = makeSatelliteTableName(satId);
    const schema = toTableSchema(tableName);
    try {
      const result = await queryDerived(conn, schema, tMin, zoomMaxPoints, tMax);
      if (result.t.length > 0) {
        perSatData.set(satId, result);
      }
    } catch (e) {
      console.warn(`multiChartDataWorker: zoom query failed for ${satId}:`, e);
    }
  }

  const { metrics: serializedMetrics, transfers } = buildMultiSeriesPayload(
    perSatData,
    satelliteConfigs,
    metricNames,
  );
  post({ type: "multi-zoom-result", id, metrics: serializedMetrics }, transfers);
}

// ---------------------------------------------------------------------------
// Tick loop
// ---------------------------------------------------------------------------

async function tick() {
  if (!conn || !baseSchema) return;

  // 1. Flush per-satellite ingest queues
  for (const [satId, queue] of ingestQueues.entries()) {
    if (queue.length === 0) continue;
    ingestQueues.set(satId, []);
    try {
      await ensureTable(satId);
      const tableName = makeSatelliteTableName(satId);
      for (let i = 0; i < queue.length; i += BATCH_SIZE) {
        const batch = queue.slice(i, i + BATCH_SIZE);
        const sql = buildInsertSQLFromRows(tableName, batch);
        if (sql) await conn.query(sql);
      }
      hasData.add(satId);
    } catch (e) {
      console.warn(`multiChartDataWorker: insert failed for ${satId}:`, e);
      // Re-queue failed rows so they're retried on the next tick
      const current = ingestQueues.get(satId) ?? [];
      ingestQueues.set(satId, queue.concat(current));
    }
  }

  // 2. Query cycle (every QUERY_EVERY_N ticks)
  tickCount++;
  if (hasData.size === 0 || tickCount % QUERY_EVERY_N !== 0) return;

  try {
    const perSatData = new Map<string, ChartDataMap>();
    const tMin = computeUnifiedTMin();

    // Compute unified tMax across all satellite tables
    let unifiedTMax: number | undefined;
    if (hasData.size > 1) {
      let maxT = -Infinity;
      for (const satId of hasData) {
        const tableName = makeSatelliteTableName(satId);
        const res = await conn.query(`SELECT MAX(t) AS t_max FROM ${tableName}`);
        const val = Number(res.getChildAt(0)?.get(0));
        if (Number.isFinite(val) && val > maxT) maxT = val;
      }
      if (Number.isFinite(maxT)) unifiedTMax = maxT;
    }

    // Query each satellite
    for (const satId of hasData) {
      const tableName = makeSatelliteTableName(satId);
      const schema = toTableSchema(tableName);
      const result = await queryDerived(conn, schema, tMin, maxPoints, unifiedTMax);
      if (result.t.length > 0) {
        perSatData.set(satId, result);
      }
    }

    if (perSatData.size > 0) {
      sendMultiChartData(perSatData, satelliteConfigs, metricNames);
    }

    // Compaction
    queryCount++;
    if (queryCount % COMPACT_EVERY_N === 0) {
      for (const satId of hasData) {
        const cd = compactCooldowns.get(satId) ?? 0;
        if (cd > 0) {
          compactCooldowns.set(satId, cd - 1);
          continue;
        }
        const tableName = makeSatelliteTableName(satId);
        try {
          await compactTable(conn, toTableSchema(tableName), COMPACT_DEFAULTS);
        } catch (e) {
          console.warn(`multiChartDataWorker: compact failed for ${satId}:`, e);
        }
      }
    }
  } catch (e) {
    console.warn("multiChartDataWorker: query cycle failed:", e);
  }
}

function scheduleNextTick() {
  tickTimer = setTimeout(() => {
    tick()
      .catch((err) => {
        console.warn("multiChartDataWorker: tick error:", err);
      })
      .finally(() => {
        if (tickTimer !== null) {
          scheduleNextTick();
        }
      });
  }, TICK_INTERVAL);
}

// ---------------------------------------------------------------------------
// Message handler
// ---------------------------------------------------------------------------

self.onmessage = async (e: MessageEvent<MultiMainToWorkerMessage>) => {
  const msg = e.data;
  if (workerDisposed && msg.type !== "dispose") return;

  switch (msg.type) {
    case "multi-init": {
      try {
        baseSchema = msg.baseSchema;
        satelliteConfigs = msg.satelliteConfigs;
        metricNames = msg.metricNames;
        if (msg.tickInterval != null) TICK_INTERVAL = msg.tickInterval;
        if (msg.queryEveryN != null) QUERY_EVERY_N = msg.queryEveryN;
        if (msg.compactEveryN != null) COMPACT_EVERY_N = msg.compactEveryN;

        const db = await initDuckDB();
        conn = await db.connect();

        // Create tables for initial satellite configs
        for (const cfg of satelliteConfigs) {
          await ensureTable(cfg.id);
        }

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

    case "multi-ingest": {
      const existing = ingestQueues.get(msg.satelliteId) ?? [];
      ingestQueues.set(msg.satelliteId, existing.concat(msg.rows));
      latestTs.set(msg.satelliteId, msg.latestT);
      break;
    }

    case "multi-rebuild": {
      if (!conn || !baseSchema) break;
      const satId = msg.satelliteId;
      try {
        await ensureTable(satId);
        const tableName = makeSatelliteTableName(satId);
        await clearTable(conn, toTableSchema(tableName));
        for (let i = 0; i < msg.rows.length; i += BATCH_SIZE) {
          const batch = msg.rows.slice(i, i + BATCH_SIZE);
          const sql = buildInsertSQLFromRows(tableName, batch);
          if (sql) await conn.query(sql);
        }
        if (msg.rows.length > 0) hasData.add(satId);
        latestTs.set(satId, msg.latestT);
        compactCooldowns.set(satId, COMPACT_COOLDOWN_AFTER_REBUILD);
        ingestQueues.set(satId, []);
      } catch (e) {
        console.warn(`multiChartDataWorker: rebuild failed for ${satId}:`, e);
      }
      break;
    }

    case "multi-configure": {
      timeRange = msg.timeRange;
      maxPoints = msg.maxPoints;
      break;
    }

    case "multi-update-configs": {
      satelliteConfigs = msg.satelliteConfigs;
      metricNames = msg.metricNames;
      break;
    }

    case "multi-zoom-query": {
      try {
        await handleMultiZoomQuery(msg.id, msg.tMin, msg.tMax, msg.maxPoints);
      } catch (e) {
        console.warn("multiChartDataWorker: multi-zoom-query failed:", e);
        post({ type: "multi-zoom-result", id: msg.id, metrics: [] });
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
