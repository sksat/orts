/**
 * Multi-satellite chart data engine: N DuckDB tables (one per satellite),
 * per-satellite queries with unified tMin/tMax, alignment via alignTimeSeries,
 * and serialized MultiChartDataMap broadcasts.
 *
 * Kept separate from the Worker entry point (`multiChartDataWorker.ts`) so the
 * message handling can be driven with an injected DuckDB and an injected
 * `post`. Like the single-satellite engine, every command runs on one serial
 * queue: `onmessage` is re-entrant across `await` points, so a rebuild and a
 * tick would otherwise interleave on the same table.
 */

import type { AsyncDuckDBConnection } from "@duckdb/duckdb-wasm";
import type { DuckDBInitOptions } from "../db/duckdb.js";
import {
  COMPACT_DEFAULTS,
  compactTable,
  createTable,
  insertRows,
  queryDerived,
  replaceRows,
} from "../db/store.js";
import type { TimeRange } from "../hooks/useTimeSeriesStore.js";
import type { ChartDataMap, TableSchema } from "../types.js";
import { alignTimeSeries, type NamedTimeSeries } from "../utils/alignTimeSeries.js";
import {
  type MultiMainToWorkerMessage,
  type MultiWorkerToMainMessage,
  type RowTuple,
  type SerializedMultiSeriesData,
  sameColumns,
  type WorkerSatelliteConfig,
  type WorkerTableSchema,
} from "./protocol.js";

/** Minimal DuckDB surface the engine needs — an injection seam for tests. */
export interface MultiChartDataDatabase {
  connect(): Promise<AsyncDuckDBConnection>;
}

export interface MultiChartDataCoreDeps {
  post(msg: MultiWorkerToMainMessage, transfer?: Transferable[]): void;
  initDb(options?: DuckDBInitOptions): Promise<MultiChartDataDatabase>;
}

/** A full-table replacement for one satellite, handed over by the main thread. */
interface Rebuild {
  rows: RowTuple[];
  latestT: number;
  /** The satellite's `datasetEpoch` when this rebuild was accepted. */
  epoch: number;
}

/**
 * Give up on a failing batch after this many retries. Matches the
 * single-satellite engine: an unbounded retry of a permanently failing batch
 * blocks every later row behind it and grows the queue without bound.
 */
const MAX_INGEST_RETRIES = 3;
const MAX_REBUILD_RETRIES = 3;

const COMPACT_COOLDOWN_AFTER_REBUILD = 5;

export function makeSatelliteTableName(satelliteId: string): string {
  const safeName = satelliteId.replace(/[^a-zA-Z0-9_]/g, "_");
  return `orbit_${safeName}`;
}

export class MultiChartDataCore {
  private readonly deps: MultiChartDataCoreDeps;

  private conn: AsyncDuckDBConnection | null = null;
  private baseSchema: WorkerTableSchema | null = null;
  private satelliteConfigs: WorkerSatelliteConfig[] = [];
  private metricNames: string[] = [];
  private timeRange: TimeRange = null;
  private maxPoints = 2000;
  private disposed = false;
  private tickTimer: ReturnType<typeof setTimeout> | null = null;

  private tickInterval = 500;
  private queryEveryN = 4;
  private compactEveryN = 20;

  // Per-satellite state
  private readonly createdTables = new Set<string>();
  private readonly hasData = new Set<string>();
  private readonly compactCooldowns = new Map<string, number>();
  /** Per-satellite ingest queues: satelliteId → rows. */
  private readonly ingestQueues = new Map<string, RowTuple[]>();
  private readonly ingestRetryCounts = new Map<string, number>();
  /** The rebuild to apply next per satellite, replaced when a newer one arrives. */
  private readonly queuedRebuilds = new Map<string, Rebuild>();
  /** Rebuilds whose transaction failed, waiting for a retry on the next tick. */
  private readonly pendingRebuilds = new Map<string, Rebuild>();
  private readonly rebuildRetryCounts = new Map<string, number>();
  /**
   * Per-satellite dataset generation, bumped whenever the table content is
   * replaced. Rows drained before the bump belong to the replaced dataset, so
   * a failed flush must drop them instead of re-queuing them on top.
   */
  private readonly datasetEpochs = new Map<string, number>();
  /**
   * Work a satellite's table needs before rows may be inserted into it or read
   * from it: `"recreate"` after a column change, `"empty"` after a dataset had
   * to be abandoned. While it is set the tick neither flushes nor queries.
   */
  private readonly tableRepairs = new Map<string, "recreate" | "empty">();
  /**
   * Generation of each repair request. A repair that awaited DuckDB clears the
   * request only if it is still the one it started on — otherwise a request
   * made while it ran would be dropped, leaving the table on an intermediate
   * schema with nothing left to fix it.
   */
  private readonly tableRepairSeqs = new Map<string, number>();
  private tableRepairSeq = 0;
  /**
   * Bumped whenever a running query stops describing what should be broadcast:
   * base-schema change, window or series-configuration change, or a dataset
   * replacement. A query that started before the bump must not post its
   * answer.
   */
  private queryEpoch = 0;
  /** Per-satellite latestT (for unified tMin computation). */
  private readonly latestTs = new Map<string, number>();

  private tickCount = 0;
  private queryCount = 0;
  /** Whether the last broadcast carried any series, so an emptied dataset is sent once. */
  private lastBroadcastHadMetrics = false;

  // Serial command queue
  private commands: Array<() => Promise<void>> = [];
  private running = false;
  private idleWaiters: Array<() => void> = [];

  constructor(deps: MultiChartDataCoreDeps) {
    this.deps = deps;
  }

  handle(msg: MultiMainToWorkerMessage): void {
    if (this.disposed && msg.type !== "dispose") return;

    switch (msg.type) {
      case "multi-ingest": {
        // Synchronous so rows arriving during a rebuild cannot be dropped.
        const existing = this.ingestQueues.get(msg.satelliteId) ?? [];
        this.ingestQueues.set(msg.satelliteId, existing.concat(msg.rows));
        this.latestTs.set(msg.satelliteId, msg.latestT);
        break;
      }

      case "multi-configure": {
        if (msg.timeRange !== this.timeRange || msg.maxPoints !== this.maxPoints) {
          this.queryEpoch++;
        }
        this.timeRange = msg.timeRange;
        this.maxPoints = msg.maxPoints;
        break;
      }

      case "multi-update-configs": {
        this.satelliteConfigs = msg.satelliteConfigs;
        this.metricNames = msg.metricNames;
        this.queryEpoch++;
        break;
      }

      case "dispose": {
        this.disposed = true;
        if (this.tickTimer != null) {
          clearTimeout(this.tickTimer);
          this.tickTimer = null;
        }
        this.enqueue(() => this.closeConnection());
        break;
      }

      case "multi-init":
        // Adopt the schema synchronously: a `multi-update-schema` that arrives
        // before the database finishes opening must see it as the previous
        // one, not be overwritten by it.
        this.baseSchema = msg.baseSchema;
        this.enqueue(() => this.handleInit(msg));
        break;

      case "multi-update-schema":
        this.adoptSchema(msg.baseSchema);
        break;

      case "multi-rebuild": {
        // Rows queued before this message belong to the dataset being
        // replaced. Dropping them here, and not when the rebuild finishes,
        // is what keeps the rows that arrive while it runs.
        const satId = msg.satelliteId;
        this.ingestQueues.set(satId, []);
        this.ingestRetryCounts.delete(satId);
        const epoch = (this.datasetEpochs.get(satId) ?? 0) + 1;
        this.datasetEpochs.set(satId, epoch);
        this.queryEpoch++;
        // The window bounds follow the replacement dataset from now on. Set
        // here rather than when the rebuild finishes, so a `multi-ingest` that
        // arrives while it runs raises them instead of being rolled back.
        this.latestTs.set(satId, msg.latestT);
        // A newer full replacement supersedes an older one, including one that
        // is waiting for a retry: only the newest is ever applied.
        this.pendingRebuilds.delete(satId);
        this.rebuildRetryCounts.delete(satId);
        this.queuedRebuilds.set(satId, { rows: msg.rows, latestT: msg.latestT, epoch });
        this.enqueue(() => this.runQueuedRebuild(satId));
        break;
      }

      case "multi-zoom-query":
        this.enqueue(() => this.handleZoomQuery(msg.id, msg.tMin, msg.tMax, msg.maxPoints));
        break;
    }
  }

  /** Run one tick of the flush + query cycle, serialized with all commands. */
  tickOnce(): Promise<void> {
    return this.enqueue(() => this.tick());
  }

  /** Resolves once the serial command queue is empty. */
  whenIdle(): Promise<void> {
    if (!this.running && this.commands.length === 0) return Promise.resolve();
    return new Promise((resolve) => {
      this.idleWaiters.push(resolve);
    });
  }

  // Serial command queue

  private enqueue(task: () => Promise<void>): Promise<void> {
    return new Promise((resolve) => {
      this.commands.push(async () => {
        try {
          await task();
        } finally {
          resolve();
        }
      });
      if (!this.running) void this.runCommands();
    });
  }

  private async runCommands(): Promise<void> {
    this.running = true;
    try {
      while (this.commands.length > 0) {
        const command = this.commands.shift();
        if (!command) break;
        try {
          await command();
        } catch (e) {
          console.warn("multiChartDataWorker: command failed:", e);
        }
      }
    } finally {
      this.running = false;
      for (const waiter of this.idleWaiters.splice(0)) waiter();
    }
  }

  // Commands

  private async handleInit(
    msg: Extract<MultiMainToWorkerMessage, { type: "multi-init" }>,
  ): Promise<void> {
    try {
      this.satelliteConfigs = msg.satelliteConfigs;
      this.metricNames = msg.metricNames;
      if (msg.tickInterval != null) this.tickInterval = msg.tickInterval;
      if (msg.queryEveryN != null) this.queryEveryN = msg.queryEveryN;
      if (msg.compactEveryN != null) this.compactEveryN = msg.compactEveryN;

      const db = await this.deps.initDb(msg.duckDB);
      this.conn = await db.connect();

      // Create tables for initial satellite configs
      for (const cfg of this.satelliteConfigs) {
        await this.ensureTable(cfg.id);
      }

      if (!this.disposed) this.scheduleNextTick();
      this.deps.post({ type: "ready" });
    } catch (e) {
      this.deps.post({
        type: "error",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  }

  /**
   * Adopt a new base schema for every satellite table. Derived expressions are
   * recomputed on the stored rows; a changed column list forces every table to
   * be recreated, dropping rows whose tuples no longer match the columns.
   */
  private adoptSchema(next: WorkerTableSchema): void {
    const prev = this.baseSchema;
    this.baseSchema = next;
    if (prev == null) return;
    this.queryEpoch++;
    if (sameColumns(prev, next)) return; // derived-only change: rows stay valid

    // Queued rows came from the previous schema's toRow, so their tuples do
    // not match the new columns: drop them with the tables. Done
    // synchronously so rows arriving after this message are kept.
    this.ingestQueues.clear();
    this.ingestRetryCounts.clear();
    this.queuedRebuilds.clear();
    this.pendingRebuilds.clear();
    this.rebuildRetryCounts.clear();
    this.hasData.clear();
    this.latestTs.clear();
    for (const satId of this.createdTables) {
      this.datasetEpochs.set(satId, (this.datasetEpochs.get(satId) ?? 0) + 1);
    }
    this.broadcastEmptyIfNeeded();

    for (const satId of this.createdTables) {
      this.requestRepair(satId, "recreate");
    }
    void this.enqueue(() => this.repairTables());
  }

  /**
   * Replace one satellite's table content with `rows`. On failure the table
   * keeps its previous content and the rows are held for a retry on the next
   * tick, instead of leaving a cleared table and dropping the data.
   */
  private async runQueuedRebuild(satId: string): Promise<void> {
    const rebuild = this.queuedRebuilds.get(satId);
    this.queuedRebuilds.delete(satId);
    // Already claimed by an earlier command that ran after this message was
    // coalesced into the slot.
    if (rebuild == null) return;
    await this.runRebuild(satId, rebuild);
  }

  private async runRebuild(satId: string, rebuild: Rebuild): Promise<void> {
    if (!this.conn || !this.baseSchema) {
      this.pendingRebuilds.set(satId, rebuild);
      return;
    }

    try {
      await this.ensureTable(satId);
      await replaceRows(this.conn, makeSatelliteTableName(satId), rebuild.rows);
    } catch (e) {
      console.warn(`multiChartDataWorker: rebuild failed for ${satId}:`, e);
      if (rebuild.epoch !== (this.datasetEpochs.get(satId) ?? 0)) {
        // Superseded while it ran: the newer replacement owns the table.
        return;
      }
      const retries = this.rebuildRetryCounts.get(satId) ?? 0;
      if (retries < MAX_REBUILD_RETRIES) {
        this.rebuildRetryCounts.set(satId, retries + 1);
        this.pendingRebuilds.set(satId, rebuild);
      } else {
        // Keeping the previous dataset would splice it with the newer rows
        // that keep arriving, so empty the table and say so.
        console.warn(
          `multiChartDataWorker: dropping rebuild of ${rebuild.rows.length} rows for ${satId} after`,
          MAX_REBUILD_RETRIES,
          "retries",
        );
        this.pendingRebuilds.delete(satId);
        this.rebuildRetryCounts.delete(satId);
        await this.abandonDataset(satId);
        this.deps.post({
          type: "error",
          message: `rebuild of ${rebuild.rows.length} rows for ${satId} failed ${MAX_REBUILD_RETRIES + 1} times; table emptied`,
        });
      }
      return;
    }

    this.pendingRebuilds.delete(satId);
    this.rebuildRetryCounts.delete(satId);
    // A successful replacement leaves nothing of the abandoned dataset behind,
    // so a pending "empty this table" request is satisfied. A pending
    // "recreate" is not: the columns are still the old ones.
    if (this.tableRepairs.get(satId) === "empty") {
      this.tableRepairs.delete(satId);
      this.tableRepairSeqs.delete(satId);
    }
    if (rebuild.rows.length > 0) {
      this.hasData.add(satId);
    } else {
      // A rebuild is a full replacement, so an empty one must empty the series.
      this.hasData.delete(satId);
    }
    this.compactCooldowns.set(satId, COMPACT_COOLDOWN_AFTER_REBUILD);

    // The tick loop stops querying once no satellite has data, so the
    // "everything is empty now" broadcast has to happen here.
    if (this.hasData.size === 0) this.broadcastEmptyIfNeeded();
  }

  private async tick(): Promise<void> {
    if (!this.conn || !this.baseSchema || this.disposed) return;

    // 0. Retry rebuilds that failed earlier, before inserting newer rows.
    for (const [satId, rebuild] of [...this.pendingRebuilds]) {
      this.pendingRebuilds.delete(satId);
      await this.runRebuild(satId, rebuild);
    }

    // 0b. Bring tables that need a column change applied, or an abandoned
    //     dataset removed, into the expected state before anything is
    //     inserted into them or read from them.
    if (this.tableRepairs.size > 0) await this.repairTables();

    // 1. Flush per-satellite ingest queues (atomically, so a failed flush
    //    inserts nothing and a retry cannot duplicate rows).
    for (const [satId, queue] of [...this.ingestQueues]) {
      if (queue.length === 0) continue;
      // A replacement (retrying or queued behind this tick) must land first:
      // flushing now would insert these rows into the table it deletes. The
      // same holds while an abandoned dataset is still in the table.
      if (this.pendingRebuilds.has(satId) || this.queuedRebuilds.has(satId)) continue;
      if (this.tableRepairs.has(satId)) continue;
      this.ingestQueues.set(satId, []);
      const epoch = this.datasetEpochs.get(satId) ?? 0;
      try {
        await this.ensureTable(satId);
        await insertRows(this.conn, makeSatelliteTableName(satId), queue);
        // Only claim the table holds data if it is still the same table: a
        // column change accepted during the insert makes these rows stale.
        if (epoch === (this.datasetEpochs.get(satId) ?? 0)) this.hasData.add(satId);
        this.ingestRetryCounts.delete(satId);
      } catch (e) {
        console.warn(`multiChartDataWorker: insert failed for ${satId}:`, e);
        const retries = this.ingestRetryCounts.get(satId) ?? 0;
        if (epoch !== (this.datasetEpochs.get(satId) ?? 0)) {
          // The dataset was replaced while the flush ran: these rows describe
          // the replaced one, so re-queuing them would resurrect it.
          console.warn(
            `multiChartDataWorker: dropping ${queue.length} rows for ${satId} from a replaced dataset`,
          );
        } else if (retries < MAX_INGEST_RETRIES) {
          this.ingestRetryCounts.set(satId, retries + 1);
          const current = this.ingestQueues.get(satId) ?? [];
          this.ingestQueues.set(satId, queue.concat(current));
        } else {
          console.warn(
            `multiChartDataWorker: dropping ${queue.length} rows for ${satId} after`,
            MAX_INGEST_RETRIES,
            "retries",
          );
          this.ingestRetryCounts.delete(satId);
        }
      }
    }

    // 2. Query cycle (every queryEveryN ticks)
    this.tickCount++;
    if (this.hasData.size === 0 || this.tickCount % this.queryEveryN !== 0) return;

    // A replacement is queued or retrying, or a table still needs repairing:
    // the tables hold a dataset that is about to be replaced, so querying now
    // would broadcast it under the new generation. The next tick queries once
    // the replacement has landed.
    if (this.queuedRebuilds.size > 0 || this.pendingRebuilds.size > 0) return;
    if (this.tableRepairs.size > 0) return;

    // The flush awaited DuckDB, so read the generation after it: a schema,
    // window or dataset change may have arrived in between.
    const epoch = this.queryEpoch;

    try {
      const perSatData = new Map<string, ChartDataMap>();
      const tMin = this.computeUnifiedTMin();

      // Compute unified tMax across all satellite tables
      let unifiedTMax: number | undefined;
      if (this.hasData.size > 1) {
        let maxT = -Infinity;
        for (const satId of this.hasData) {
          const tableName = makeSatelliteTableName(satId);
          const res = await this.conn.query(`SELECT MAX(t) AS t_max FROM ${tableName}`);
          const val = Number(res.getChildAt(0)?.get(0));
          if (Number.isFinite(val) && val > maxT) maxT = val;
        }
        if (Number.isFinite(maxT)) unifiedTMax = maxT;
      }

      // Query each satellite
      for (const satId of this.hasData) {
        const schema = this.toTableSchema(makeSatelliteTableName(satId));
        const result = await queryDerived(this.conn, schema, tMin, this.maxPoints, unifiedTMax);
        if (result.t.length > 0) {
          perSatData.set(satId, result);
        }
      }

      if (this.queryEpoch !== epoch) {
        // The schema, window, series configuration or a dataset changed while
        // these queries ran: the payload describes the old one. The next tick
        // re-queries.
        return;
      }
      if (this.tableRepairs.size > 0 || this.queuedRebuilds.size > 0) return;
      this.sendMultiChartData(perSatData);

      // Compaction
      this.queryCount++;
      if (this.queryCount % this.compactEveryN === 0) {
        for (const satId of this.hasData) {
          const cd = this.compactCooldowns.get(satId) ?? 0;
          if (cd > 0) {
            this.compactCooldowns.set(satId, cd - 1);
            continue;
          }
          try {
            await compactTable(
              this.conn,
              this.toTableSchema(makeSatelliteTableName(satId)),
              COMPACT_DEFAULTS,
            );
          } catch (e) {
            console.warn(`multiChartDataWorker: compact failed for ${satId}:`, e);
          }
        }
      }
    } catch (e) {
      console.warn("multiChartDataWorker: query cycle failed:", e);
    }
  }

  /**
   * Run an ad-hoc zoom query for the absolute window `[tMin, tMax]` against
   * every satellite's DuckDB table and post a one-shot `multi-zoom-result`
   * correlated by `id`. Empty results still post (with no metrics) so the
   * client-side promise always resolves.
   */
  private async handleZoomQuery(
    id: number,
    tMin: number,
    tMax: number,
    zoomMaxPoints: number,
  ): Promise<void> {
    if (!this.conn || !this.baseSchema) {
      this.deps.post({ type: "multi-zoom-result", id, metrics: [] });
      return;
    }

    const perSatData = new Map<string, ChartDataMap>();
    for (const satId of this.hasData) {
      const schema = this.toTableSchema(makeSatelliteTableName(satId));
      try {
        const result = await queryDerived(this.conn, schema, tMin, zoomMaxPoints, tMax);
        if (result.t.length > 0) {
          perSatData.set(satId, result);
        }
      } catch (e) {
        console.warn(`multiChartDataWorker: zoom query failed for ${satId}:`, e);
      }
    }

    const { metrics, transfers } = this.buildMultiSeriesPayload(perSatData);
    this.deps.post({ type: "multi-zoom-result", id, metrics }, transfers);
  }

  /**
   * Give up on one satellite's dataset: empty its table and drop its series, so
   * a failed replacement cannot leave the previous dataset to be spliced with
   * the rows that keep arriving.
   */
  private async abandonDataset(satId: string): Promise<void> {
    // Reporting the dataset as gone while its rows are still in the table
    // would let the next flush append to it. `repairTables` keeps the request
    // set until the delete succeeds.
    this.requestRepair(satId, "empty");
    await this.repairTables();
  }

  /**
   * Bring the tables into the state the current schema and datasets expect:
   * recreate after a column change, empty after an abandoned dataset. A
   * request that fails stays set, so the next tick tries again before any row
   * is inserted or read.
   */
  private requestRepair(satId: string, repair: "recreate" | "empty"): void {
    this.tableRepairs.set(satId, repair);
    this.tableRepairSeqs.set(satId, ++this.tableRepairSeq);
  }

  private async repairTables(): Promise<void> {
    if (!this.conn || !this.baseSchema) return;
    for (const [satId, repair] of [...this.tableRepairs]) {
      const seq = this.tableRepairSeqs.get(satId);
      const tableName = makeSatelliteTableName(satId);
      try {
        if (repair === "recreate") {
          await createTable(this.conn, this.toTableSchema(tableName));
        } else {
          await replaceRows(this.conn, tableName, []);
        }
      } catch (e) {
        console.warn(`multiChartDataWorker: failed to ${repair} ${satId}, retrying:`, e);
        continue;
      }
      if (this.tableRepairSeqs.get(satId) !== seq) continue; // newer request
      this.tableRepairs.delete(satId);
      this.tableRepairSeqs.delete(satId);
      // A rebuild still in flight may have re-added this satellite in between;
      // its table is empty now. latestTs is left alone: rows that arrived while
      // the table was being repaired have already moved it.
      this.hasData.delete(satId);
      this.queryEpoch++;
    }
    if (this.hasData.size === 0) this.broadcastEmptyIfNeeded();
  }

  private async closeConnection(): Promise<void> {
    if (!this.conn) return;
    try {
      await this.conn.close();
    } catch {
      // ignore
    }
    this.conn = null;
  }

  // Helpers

  private scheduleNextTick(): void {
    this.tickTimer = setTimeout(() => {
      void this.enqueue(async () => {
        try {
          await this.tick();
        } finally {
          if (!this.disposed && this.tickTimer !== null) this.scheduleNextTick();
        }
      });
    }, this.tickInterval);
  }

  private async ensureTable(satelliteId: string): Promise<void> {
    if (this.createdTables.has(satelliteId)) return;
    if (!this.conn) return;
    // Registered before the await, so a schema change that arrives while the
    // CREATE runs queues a repair for this table too instead of missing it.
    this.createdTables.add(satelliteId);
    try {
      await createTable(this.conn, this.toTableSchema(makeSatelliteTableName(satelliteId)));
    } catch (e) {
      this.createdTables.delete(satelliteId);
      throw e;
    }
  }

  private toTableSchema(tableName: string): TableSchema {
    if (!this.baseSchema) throw new Error("baseSchema not initialized");
    return {
      ...this.baseSchema,
      tableName,
      toRow: () => {
        throw new Error("toRow should not be called in worker");
      },
    };
  }

  /** Compute unified tMin from all satellite latestTs. */
  private computeUnifiedTMin(): number | undefined {
    if (this.timeRange == null) return undefined;
    let max = -Infinity;
    for (const t of this.latestTs.values()) {
      if (t > max) max = t;
    }
    if (!Number.isFinite(max)) return undefined;
    return max - this.timeRange;
  }

  /**
   * Build the serialized multi-series payload from per-satellite
   * `ChartDataMap` results. Shared between the periodic tick loop and
   * one-shot zoom queries.
   */
  private buildMultiSeriesPayload(perSatData: Map<string, ChartDataMap>): {
    metrics: SerializedMultiSeriesData[];
    transfers: ArrayBuffer[];
  } {
    const activeSats = this.satelliteConfigs.filter((cfg) => perSatData.has(cfg.id));
    const serializedMetrics: SerializedMultiSeriesData[] = [];
    const allTransfers: ArrayBuffer[] = [];

    if (activeSats.length === 0) {
      return { metrics: serializedMetrics, transfers: allTransfers };
    }

    for (const metric of this.metricNames) {
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

  /** Serialize per-satellite results → transferable tick broadcast. */
  private sendMultiChartData(perSatData: Map<string, ChartDataMap>): void {
    const { metrics, transfers } = this.buildMultiSeriesPayload(perSatData);
    if (metrics.length === 0) {
      this.broadcastEmptyIfNeeded();
      return;
    }
    this.lastBroadcastHadMetrics = true;
    this.deps.post({ type: "multi-chart-data", metrics }, transfers);
  }

  /** Broadcast "no series" once, so charts clear instead of showing stale data. */
  private broadcastEmptyIfNeeded(): void {
    if (!this.lastBroadcastHadMetrics) return;
    this.lastBroadcastHadMetrics = false;
    this.deps.post({ type: "multi-chart-data", metrics: [] });
  }
}
