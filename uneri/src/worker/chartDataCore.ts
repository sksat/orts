/**
 * Chart data engine: the DuckDB ownership + cold/hot tick loop that the
 * chart data Web Worker runs.
 *
 * Kept separate from the Worker entry point (`chartDataWorker.ts`) so the
 * message handling can be driven directly — with an injected DuckDB and an
 * injected `post` — without a Worker or a real database.
 *
 * All commands (init, ingest, rebuild, schema update, tick, queries, dispose)
 * run on a single serial queue. `onmessage` in a Worker is re-entrant across
 * `await` points, so without that queue a rebuild and a tick interleave and
 * corrupt each other's view of the table.
 */

import type { AsyncDuckDBConnection } from "@duckdb/duckdb-wasm";
import type { DuckDBInitOptions } from "../db/duckdb.js";
import {
  COMPACT_DEFAULTS,
  compactTable,
  createTable,
  insertRows,
  queryDerived,
  queryDerivedIncremental,
  replaceRows,
} from "../db/store.js";
import { computeTMin, DISPLAY_MAX_POINTS, type TimeRange } from "../hooks/useTimeSeriesStore.js";
import type { ChartDataMap, TableSchema } from "../types.js";
import { mergeChartData, trimChartDataLeft } from "../utils/mergeChartData.js";
import {
  type MainToWorkerMessage,
  type RowTuple,
  sameColumns,
  sameDerived,
  type WorkerTableSchema,
  type WorkerToMainMessage,
} from "./protocol.js";

/** Minimal DuckDB surface the engine needs — an injection seam for tests. */
export interface ChartDataDatabase {
  connect(): Promise<AsyncDuckDBConnection>;
}

export interface ChartDataCoreDeps {
  /** Send a message to the main thread (Worker `postMessage`). */
  post(msg: WorkerToMainMessage, transfer?: Transferable[]): void;
  /** Open the database. Called once per `init`. */
  initDb(options?: DuckDBInitOptions): Promise<ChartDataDatabase>;
}

/** A full-table replacement handed over by the main thread. */
interface Rebuild {
  rows: RowTuple[];
  latestT: number;
  /** `datasetEpoch` when this rebuild was accepted (see `datasetEpoch`). */
  epoch: number;
}

/** Give up on a failing batch after this many retries instead of retrying forever. */
const MAX_INGEST_RETRIES = 3;
/** Same bound for a failing full rebuild. */
const MAX_REBUILD_RETRIES = 3;

const COMPACT_EVERY_N = 5;
const COMPACT_COOLDOWN_AFTER_REBUILD = 5;

export class ChartDataCore {
  private readonly deps: ChartDataCoreDeps;

  private conn: AsyncDuckDBConnection | null = null;
  private schema: WorkerTableSchema | null = null;
  private disposed = false;
  private timeRange: TimeRange = null;
  private maxPoints: number = DISPLAY_MAX_POINTS;
  private latestT = -Infinity;
  private earliestT = Infinity;
  private tickTimer: ReturnType<typeof setTimeout> | null = null;

  // Cold/hot state (mirroring useTimeSeriesStore)
  private coldSnapshot: ChartDataMap | null = null;
  private coldTMax = -Infinity;
  private hotBuffer: ChartDataMap | null = null;
  private ticksSinceCold = 0;
  private coldRefreshNeeded = true;
  private coldQueryCount = 0;
  private hasData = false;
  private compactCooldown = 0;
  /**
   * Rows of the last broadcast, so an emptied dataset is broadcast exactly
   * once. Starts at 0: before anything has been sent there is nothing to
   * clear on the main thread.
   */
  private lastSentRowCount = 0;

  private tickInterval = 250;
  private coldRefreshEveryN = 20;
  private hotRowBudget = 500;
  /** True when coldRefreshEveryN was not explicitly set by the caller. */
  private useAdaptiveAllMode = true;

  /**
   * Work the table needs before rows may be inserted into it or read from it:
   * `"recreate"` after a column change, `"empty"` after a dataset had to be
   * abandoned. While it is set the tick neither flushes nor queries, so no row
   * lands in — and no payload is built from — a table that does not match the
   * current schema and dataset.
   */
  private tableRepair: "recreate" | "empty" | null = null;

  /** Table the schema change renamed away from, dropped by the repair. */
  private previousTableName: string | null = null;
  /**
   * Bumped on every repair request. A repair that awaited DuckDB clears the
   * request only if it is still the one it started on — otherwise a request
   * made while it ran (a second column change) would be dropped, leaving the
   * table on an intermediate schema with nothing left to fix it.
   */
  private tableRepairSeq = 0;

  /** Rows buffered between ticks. */
  private ingestQueue: RowTuple[] = [];
  private ingestRetryCount = 0;
  /** The rebuild to apply next, replaced when a newer one arrives. */
  private queuedRebuild: Rebuild | null = null;
  /** A rebuild whose transaction failed, waiting to be retried on the next tick. */
  private pendingRebuild: Rebuild | null = null;
  private rebuildRetryCount = 0;

  /**
   * Bumped whenever the cached cold/hot results stop describing the query the
   * engine should be running (schema swap, window change, new dataset). A query
   * that started before the bump must not overwrite the state it invalidated —
   * otherwise a tick already awaiting DuckDB re-caches the stale answer and
   * clears `coldRefreshNeeded`, delaying the new one by a whole refresh period.
   */
  private queryEpoch = 0;
  /**
   * Bumped whenever the table content is replaced. Rows drained before the
   * bump belong to the replaced dataset, so a failed flush must drop them
   * instead of re-queuing them on top of the new one.
   */
  private datasetEpoch = 0;

  // Serial command queue
  private commands: Array<() => Promise<void>> = [];
  private running = false;
  private idleWaiters: Array<() => void> = [];

  constructor(deps: ChartDataCoreDeps) {
    this.deps = deps;
  }

  /**
   * Accept a message from the main thread. Returns immediately; the work is
   * appended to the serial command queue.
   */
  handle(msg: MainToWorkerMessage): void {
    if (this.disposed && msg.type !== "dispose") return;

    switch (msg.type) {
      case "ingest": {
        // Synchronous so it cannot be lost between a rebuild's await points:
        // rows appended here are newer than any rebuild already in flight.
        this.ingestQueue = this.ingestQueue.concat(msg.rows);
        this.latestT = msg.latestT;
        if (this.earliestT === Infinity && msg.rows.length > 0 && msg.rows[0][0] != null) {
          this.earliestT = msg.rows[0][0];
        }
        break;
      }

      case "configure": {
        const rangeChanged = msg.timeRange !== this.timeRange;
        const pointsChanged = msg.maxPoints !== this.maxPoints;
        this.timeRange = msg.timeRange;
        this.maxPoints = msg.maxPoints;
        if (rangeChanged || pointsChanged) {
          this.coldRefreshNeeded = true;
          this.queryEpoch++;
        }
        break;
      }

      case "dispose": {
        // Stop the tick loop synchronously; the connection is closed once the
        // in-flight command finishes.
        this.disposed = true;
        if (this.tickTimer != null) {
          clearTimeout(this.tickTimer);
          this.tickTimer = null;
        }
        this.enqueue(() => this.closeConnection());
        break;
      }

      case "init":
        // Adopt the schema synchronously: an `update-schema` that arrives
        // before the database finishes opening must see it as the previous
        // one, not be overwritten by it.
        this.schema = msg.schema;
        this.enqueue(() => this.handleInit(msg));
        break;

      case "update-schema":
        this.adoptSchema(msg.schema);
        break;

      case "rebuild": {
        // Rows queued before this message belong to the dataset being
        // replaced. Dropping them here, and not when the rebuild finishes,
        // is what keeps the rows that arrive while it runs.
        this.ingestQueue = [];
        this.ingestRetryCount = 0;
        // The window bounds follow the replacement dataset from now on. Set
        // here rather than when the rebuild finishes, so an `ingest` that
        // arrives while it runs raises them instead of being rolled back.
        this.latestT = msg.latestT;
        this.earliestT = msg.rows.length > 0 && msg.rows[0][0] != null ? msg.rows[0][0] : Infinity;
        this.datasetEpoch++;
        this.queryEpoch++;
        // A newer full replacement supersedes an older one, including one that
        // is waiting for a retry: only the newest is ever applied.
        this.pendingRebuild = null;
        this.rebuildRetryCount = 0;
        this.queuedRebuild = {
          rows: msg.rows,
          latestT: msg.latestT,
          epoch: this.datasetEpoch,
        };
        this.enqueue(() => this.runQueuedRebuild());
        break;
      }

      case "debug-query":
        this.enqueue(() => this.handleDebugQuery(msg.id));
        break;

      case "zoom-query":
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
          console.warn("chartDataWorker: command failed:", e);
        }
      }
    } finally {
      this.running = false;
      for (const waiter of this.idleWaiters.splice(0)) waiter();
    }
  }

  // Commands

  private async handleInit(msg: Extract<MainToWorkerMessage, { type: "init" }>): Promise<void> {
    try {
      if (msg.tickInterval != null) this.tickInterval = msg.tickInterval;
      if (msg.coldRefreshEveryN != null) {
        this.coldRefreshEveryN = msg.coldRefreshEveryN;
        this.useAdaptiveAllMode = false;
      }
      if (msg.hotRowBudget != null) this.hotRowBudget = msg.hotRowBudget;
      const db = await this.deps.initDb(msg.duckDB);
      this.conn = await db.connect();
      if (this.schema) await createTable(this.conn, this.toTableSchema(this.schema));

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
   * Adopt a new schema. Derived expressions are recomputed on the stored rows;
   * a changed column list forces the table to be recreated, which drops the
   * rows (their tuples no longer match the columns) and clears the chart.
   *
   * The state changes happen synchronously, so rows that arrive after this
   * message — which the main thread produced with the new schema — are not
   * caught by the cleanup.
   */
  private adoptSchema(next: WorkerTableSchema): void {
    const prev = this.schema;
    this.schema = next;
    if (prev == null) return;
    // A rename needs a new table as much as a column change does.
    const columnsChanged = !sameColumns(prev, next) || prev.tableName !== next.tableName;
    if (!columnsChanged && sameDerived(prev, next)) return;

    this.queryEpoch++;

    if (columnsChanged) {
      // Queued rows were produced by the previous schema's toRow, so their
      // tuples do not match the new columns: drop them with the table, and
      // with the rows a flush already in flight would re-queue.
      this.datasetEpoch++;
      this.ingestQueue = [];
      this.queuedRebuild = null;
      this.pendingRebuild = null;
      this.ingestRetryCount = 0;
      this.rebuildRetryCount = 0;
      this.hasData = false;
      this.latestT = -Infinity;
      this.earliestT = Infinity;
    }

    // Derived values in the snapshot were computed with the old expressions.
    this.coldSnapshot = null;
    this.coldTMax = -Infinity;
    this.hotBuffer = null;
    this.coldRefreshNeeded = true;
    if (!this.hasData) this.sendEmptyChartData();

    if (columnsChanged) {
      this.tableRepair = "recreate";
      this.tableRepairSeq++;
      this.previousTableName = prev.tableName;
      void this.enqueue(() => this.repairTable());
    }
  }

  /**
   * Bring the table into the state the current schema and dataset expect:
   * recreate it after a column change, or empty it after a dataset had to be
   * abandoned. Left set on failure so the next tick tries again before any row
   * is inserted or read.
   */
  private async repairTable(): Promise<void> {
    const repair = this.tableRepair;
    const seq = this.tableRepairSeq;
    if (repair == null || !this.conn || !this.schema) return;
    try {
      if (repair === "recreate") {
        if (this.previousTableName != null && this.previousTableName !== this.schema.tableName) {
          await this.conn.query(`DROP TABLE IF EXISTS ${this.previousTableName}`);
        }
        await createTable(this.conn, this.toTableSchema(this.schema));
      } else {
        await replaceRows(this.conn, this.schema.tableName, []);
      }
    } catch (e) {
      console.warn(`chartDataWorker: failed to ${repair} the table, retrying:`, e);
      return;
    }
    if (this.tableRepairSeq !== seq) return; // a newer request took over
    this.tableRepair = null;
    this.previousTableName = null;
    // A rebuild that was still in flight may have finished in between and
    // re-set this; the table is empty now. latestT/earliestT stay as they are:
    // rows that arrived while the table was being repaired have already moved
    // them to where the next rows are.
    this.hasData = false;
    this.coldSnapshot = null;
    this.coldTMax = -Infinity;
    this.hotBuffer = null;
    this.queryEpoch++;
    this.sendEmptyChartData();
  }

  /** Apply the newest queued rebuild, if this command is the one that claims it. */
  private async runQueuedRebuild(): Promise<void> {
    const rebuild = this.queuedRebuild;
    this.queuedRebuild = null;
    // Already claimed by an earlier command that ran after this message was
    // coalesced into the slot.
    if (rebuild == null) return;
    await this.runRebuild(rebuild);
  }

  /**
   * Replace the table content with `rows`. On failure the table keeps its
   * previous content (the whole replacement is one transaction) and the rows
   * are held for a retry on the next tick, so a failed rebuild neither empties
   * the table nor loses the data the main thread already handed over.
   */
  private async runRebuild(rebuild: Rebuild): Promise<void> {
    if (!this.conn || !this.schema) {
      // Not initialized yet: hold the rows so init + tick can apply them.
      this.pendingRebuild = rebuild;
      return;
    }

    try {
      await replaceRows(this.conn, this.schema.tableName, rebuild.rows);
    } catch (e) {
      console.warn("chartDataWorker: rebuild failed:", e);
      if (rebuild.epoch !== this.datasetEpoch) {
        // Superseded while it ran: the newer replacement owns the table.
        return;
      }
      if (this.rebuildRetryCount < MAX_REBUILD_RETRIES) {
        this.rebuildRetryCount++;
        this.pendingRebuild = rebuild;
      } else {
        // Keeping the previous dataset would splice it with the newer rows
        // that keep arriving, so empty the table and say so.
        console.warn(
          "chartDataWorker: dropping rebuild of",
          rebuild.rows.length,
          "rows after",
          MAX_REBUILD_RETRIES,
          "retries",
        );
        this.pendingRebuild = null;
        this.rebuildRetryCount = 0;
        await this.abandonDataset();
        this.deps.post({
          type: "error",
          message: `rebuild of ${rebuild.rows.length} rows failed ${MAX_REBUILD_RETRIES + 1} times; table emptied`,
        });
      }
      return;
    }

    this.pendingRebuild = null;
    this.rebuildRetryCount = 0;
    // A successful replacement leaves nothing of the abandoned dataset behind,
    // so a pending "empty this table" request is satisfied. A pending
    // "recreate" is not: the columns are still the old ones.
    if (this.tableRepair === "empty") this.tableRepair = null;
    this.hasData = rebuild.rows.length > 0;
    this.compactCooldown = COMPACT_COOLDOWN_AFTER_REBUILD;
    this.coldRefreshNeeded = true;
    this.coldSnapshot = null;
    this.coldTMax = -Infinity;
    this.hotBuffer = null;

    // A rebuild is a full replacement, so an empty one must empty the chart.
    // The tick loop stops querying once hasData is false, so send it here.
    if (!this.hasData) this.sendEmptyChartData();
  }

  private async tick(): Promise<void> {
    if (!this.conn || !this.schema || this.disposed) return;

    // 0. Retry a rebuild that failed earlier, before inserting newer rows.
    if (this.pendingRebuild != null) {
      const retry = this.pendingRebuild;
      this.pendingRebuild = null;
      await this.runRebuild(retry);
      if (this.pendingRebuild != null) return; // still failing — retry next tick
    }

    // The table is not in the state the current schema and dataset expect
    // (a column change to apply, or an abandoned dataset to remove): fix it
    // before anything is inserted into it or read from it.
    if (this.tableRepair != null) {
      await this.repairTable();
      if (this.tableRepair != null) return;
    }

    // A replacement is queued behind this tick: flushing now would insert the
    // newer rows into the table it is about to delete. They stay queued and go
    // in on the next tick, after the replacement.
    if (this.queuedRebuild != null) return;

    // 1. Flush ingest queue (atomically: a failed flush inserts nothing, so
    //    re-queuing the rows cannot duplicate them).
    if (this.ingestQueue.length > 0) {
      const rows = this.ingestQueue;
      const epoch = this.datasetEpoch;
      this.ingestQueue = [];
      try {
        await insertRows(this.conn, this.schema.tableName, rows);
        // Only claim the table holds data if it is still the same table: a
        // column change accepted during the insert makes these rows stale.
        if (epoch === this.datasetEpoch) this.hasData = true;
        this.ingestRetryCount = 0;
      } catch (e) {
        console.warn("chartDataWorker: insert failed:", e);
        if (epoch !== this.datasetEpoch) {
          // The dataset was replaced while the flush ran: these rows describe
          // the replaced one, so re-queuing them would resurrect it.
          console.warn("chartDataWorker: dropping", rows.length, "rows from a replaced dataset");
        } else if (this.ingestRetryCount < MAX_INGEST_RETRIES) {
          this.ingestQueue = rows.concat(this.ingestQueue);
          this.ingestRetryCount++;
        } else {
          console.warn(
            "chartDataWorker: dropping",
            rows.length,
            "rows after",
            MAX_INGEST_RETRIES,
            "retries",
          );
          this.ingestRetryCount = 0;
        }
      }
    }

    // 2. Cold/hot query cycle
    if (!this.hasData || this.disposed || !this.schema) return;
    // The flush awaited DuckDB, so re-read everything the query depends on:
    // a schema swap or a new replacement may have arrived in between.
    if (this.queuedRebuild != null || this.pendingRebuild != null) return;
    const tableSchema = this.toTableSchema(this.schema);
    const epoch = this.queryEpoch;

    this.ticksSinceCold++;
    // From the same object the query is built with, so a mid-flight schema
    // swap cannot pair new names with old columns.
    const derivedNames = tableSchema.derived.map((d) => d.name);

    // "All" mode adaptive refresh
    // In "All" mode (timeRange === null), the chart's time range grows
    // continuously. As it widens, a single new data point shifts fewer
    // and fewer pixels on screen — refreshing every 250ms is wasteful.
    // We scale the cold refresh interval based on the elapsed time span
    // so that updates happen only as often as they're visually meaningful.
    const effectiveColdEveryN = this.computeEffectiveColdEveryN(this.timeRange);

    const needsCold =
      this.coldRefreshNeeded ||
      this.ticksSinceCold >= effectiveColdEveryN ||
      (this.hotBuffer != null && this.hotBuffer.t.length > this.hotRowBudget);

    if (needsCold) {
      // COLD PATH: full downsampled query
      try {
        const tMin = computeTMin(this.timeRange, this.latestT);
        const snapshot = await queryDerived(this.conn, tableSchema, tMin, this.maxPoints);
        if (this.queryEpoch !== epoch) {
          // Schema, window or dataset changed while this query ran: its answer
          // describes the old one. Leave coldRefreshNeeded set and retry.
          this.coldRefreshNeeded = true;
          return;
        }
        this.coldSnapshot = snapshot;
        this.coldTMax =
          this.coldSnapshot.t.length > 0
            ? this.coldSnapshot.t[this.coldSnapshot.t.length - 1]
            : -Infinity;
        this.hotBuffer = null;
        this.ticksSinceCold = 0;
        this.coldRefreshNeeded = false;

        // Compaction
        this.coldQueryCount++;
        if (this.compactCooldown > 0) {
          this.compactCooldown--;
        } else if (this.coldQueryCount % COMPACT_EVERY_N === 0) {
          const compacted = await compactTable(this.conn, tableSchema, COMPACT_DEFAULTS);
          if (compacted) this.coldRefreshNeeded = true;
        }
      } catch (e) {
        console.warn("chartDataWorker: cold query failed:", e);
      }
    } else if (this.timeRange != null) {
      // HOT PATH: incremental query (only for windowed mode).
      // In "All" mode, skip hot queries — the downsampled cold snapshot
      // already covers the full range, and incremental additions are
      // sub-pixel and don't justify the query + render cost.
      try {
        const tMin = computeTMin(this.timeRange, this.latestT);
        const hotLowerBound = tMin != null ? Math.max(this.coldTMax, tMin) : this.coldTMax;
        const hot = await queryDerivedIncremental(this.conn, tableSchema, hotLowerBound);
        if (this.queryEpoch !== epoch) {
          this.coldRefreshNeeded = true;
          return;
        }
        this.hotBuffer = hot;
      } catch (e) {
        console.warn("chartDataWorker: hot query failed:", e);
      }
    }

    // 3. Merge + trim → send. Compaction and the queries above awaited DuckDB,
    //    so re-check: a replacement or a schema change accepted in between
    //    makes this snapshot describe a table that no longer exists.
    if (
      this.queryEpoch !== epoch ||
      this.queuedRebuild != null ||
      this.pendingRebuild != null ||
      this.tableRepair != null
    ) {
      this.coldSnapshot = null;
      this.coldTMax = -Infinity;
      this.hotBuffer = null;
      this.coldRefreshNeeded = true;
      return;
    }

    if (this.coldSnapshot != null) {
      let merged = mergeChartData(this.coldSnapshot, this.hotBuffer, derivedNames);
      if (this.timeRange != null) {
        merged = trimChartDataLeft(merged, this.latestT - this.timeRange, derivedNames);
      }
      this.sendChartData(merged);
    }
  }

  /**
   * Give up on the current dataset: empty the table and clear the chart, so a
   * failed replacement cannot leave the previous dataset to be spliced with
   * the rows that keep arriving.
   */
  private async abandonDataset(): Promise<void> {
    // Reporting the dataset as gone while its rows are still in the table
    // would let the next flush append to it, which is the splice this avoids.
    // `repairTable` keeps the request set until the delete succeeds.
    this.tableRepair = "empty";
    this.tableRepairSeq++;
    await this.repairTable();
  }

  private async handleDebugQuery(id: number): Promise<void> {
    if (!this.conn || !this.schema) {
      this.deps.post({ type: "debug-result", id, result: 0 });
      return;
    }
    try {
      const result = await this.conn.query(`SELECT COUNT(*) AS cnt FROM ${this.schema.tableName}`);
      const count = Number(result.getChildAt(0)?.get(0));
      this.deps.post({ type: "debug-result", id, result: count });
    } catch {
      this.deps.post({ type: "debug-result", id, result: -1 });
    }
  }

  private async handleZoomQuery(
    id: number,
    tMin: number,
    tMax: number,
    zoomMaxPoints: number,
  ): Promise<void> {
    if (!this.conn || !this.schema) {
      this.deps.post({ type: "zoom-result", id, keys: [], buffers: [] });
      return;
    }
    try {
      const tableSchema = this.toTableSchema(this.schema);
      const result = await queryDerived(this.conn, tableSchema, tMin, zoomMaxPoints, tMax);
      const { keys, buffers } = serializeChartData(result);
      this.deps.post({ type: "zoom-result", id, keys, buffers }, buffers);
    } catch {
      this.deps.post({ type: "zoom-result", id, keys: [], buffers: [] });
    }
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

  /** Schedule the next tick after the current one completes (setTimeout chain). */
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

  /**
   * Compute the effective cold refresh interval (in ticks) based on the
   * current time range span. In windowed mode (timeRange != null), use
   * the configured coldRefreshEveryN. In "All" mode, scale the
   * interval so that refreshes are less frequent as the time span grows.
   *
   * Rationale: with an 800px-wide chart, 1 second of new data at
   * elapsed=3600s shifts < 0.25 pixels. Refreshing every 250ms is
   * wasteful — we only need to refresh when enough new data has
   * accumulated to be visually distinguishable.
   *
   * Heuristic: refresh interval ≈ max(baseInterval, elapsed / 200).
   * This means roughly 1 refresh per 0.5% time-range growth:
   *   - 0–60s elapsed:   every 5s   (20 ticks at 250ms)
   *   - 10 min elapsed:  every 3s   (12 ticks)
   *   - 1 hour elapsed:  every 18s  (72 ticks)
   *   - 24 hours elapsed: every 7m  (~1700 ticks)
   */
  private computeEffectiveColdEveryN(range: TimeRange): number {
    // Windowed mode or explicitly configured: use configured interval
    if (range != null || !this.useAdaptiveAllMode) return this.coldRefreshEveryN;

    // "All" mode: scale based on total time span
    const span = this.latestT - this.earliestT;
    if (!Number.isFinite(span) || span <= 0) return this.coldRefreshEveryN;

    // Convert span to tick count: span / 200 / (tickInterval / 1000)
    const adaptiveTicks = Math.ceil(span / 200 / (this.tickInterval / 1000));
    return Math.max(this.coldRefreshEveryN, adaptiveTicks);
  }

  /** Build a minimal TableSchema (with dummy toRow) for store.ts functions. */
  private toTableSchema(ws: WorkerTableSchema): TableSchema {
    return {
      ...ws,
      toRow: () => {
        throw new Error("toRow should not be called in worker");
      },
    };
  }

  private sendChartData(data: ChartDataMap): void {
    const { keys, buffers } = serializeChartData(data);
    this.lastSentRowCount = data.t.length;
    this.deps.post({ type: "chart-data", keys, buffers }, buffers);
  }

  /** Broadcast an empty dataset once, so the chart clears instead of freezing. */
  private sendEmptyChartData(): void {
    if (this.lastSentRowCount === 0) return;
    const empty: ChartDataMap = { t: new Float64Array(0) };
    for (const d of this.schema?.derived ?? []) {
      empty[d.name] = new Float64Array(0);
    }
    this.sendChartData(empty);
  }
}

/** Copy a ChartDataMap into transferable buffers. */
function serializeChartData(data: ChartDataMap): { keys: string[]; buffers: ArrayBuffer[] } {
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
  return { keys, buffers };
}
