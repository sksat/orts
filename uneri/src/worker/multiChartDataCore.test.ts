import type { AsyncDuckDBConnection } from "@duckdb/duckdb-wasm";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FakeDuckDBConn } from "../testing/fakeDuckDB.js";
import { MultiChartDataCore, makeSatelliteTableName } from "./multiChartDataCore.js";
import type { MultiWorkerToMainMessage, RowTuple, WorkerTableSchema } from "./protocol.js";

const SAT_ID = "sat-a";
const TABLE = makeSatelliteTableName(SAT_ID);

const EARTH_SCHEMA: WorkerTableSchema = {
  tableName: "orbit",
  columns: [
    { name: "t", type: "DOUBLE" },
    { name: "r", type: "DOUBLE" },
  ],
  derived: [{ name: "altitude", sql: "r - 6378.137" }],
};

const MOON_SCHEMA: WorkerTableSchema = {
  ...EARTH_SCHEMA,
  derived: [{ name: "altitude", sql: "r - 1737.4" }],
};

function rows(...ts: number[]): RowTuple[] {
  return ts.map((t) => [t, 7000 + t]);
}

/** Let already-queued promise callbacks run (no timers involved). */
async function flush(times = 50) {
  for (let i = 0; i < times; i++) await Promise.resolve();
}

function setup() {
  const conn = new FakeDuckDBConn();
  const posted: MultiWorkerToMainMessage[] = [];
  const core = new MultiChartDataCore({
    post: (msg) => {
      posted.push(msg);
    },
    initDb: async () => ({
      connect: async () => conn as unknown as AsyncDuckDBConnection,
    }),
  });
  return { conn, posted, core };
}

async function init(core: MultiChartDataCore, baseSchema = EARTH_SCHEMA) {
  core.handle({
    type: "multi-init",
    baseSchema,
    satelliteConfigs: [{ id: SAT_ID, label: "A", color: "#fff" }],
    metricNames: ["altitude"],
    tickInterval: 1_000_000,
    queryEveryN: 1,
  });
  core.handle({ type: "multi-configure", timeRange: null, maxPoints: 2000 });
  await core.whenIdle();
}

/** Series counts of every broadcast, in order. */
function broadcastMetricCounts(posted: MultiWorkerToMainMessage[]): number[] {
  return posted
    .filter(
      (m): m is Extract<MultiWorkerToMainMessage, { type: "multi-chart-data" }> =>
        m.type === "multi-chart-data",
    )
    .map((m) => m.metrics.length);
}

describe("MultiChartDataCore", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    return () => {
      vi.useRealTimers();
    };
  });

  it("queries with the derived expressions of the latest base schema", async () => {
    const { core, conn } = setup();
    await init(core);
    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();
    expect(conn.queries.some((q) => q.includes("r - 6378.137"))).toBe(true);

    core.handle({ type: "multi-update-schema", baseSchema: MOON_SCHEMA });
    await core.whenIdle();
    const before = conn.queries.length;
    await core.tickOnce();

    const afterUpdate = conn.queries.slice(before);
    expect(afterUpdate.some((q) => q.includes("r - 1737.4"))).toBe(true);
    expect(afterUpdate.some((q) => q.includes("r - 6378.137"))).toBe(false);
    expect(conn.tValuesOf(TABLE)).toEqual([0, 1]);
  });

  it("keeps rows that arrive while a rebuild is in flight", async () => {
    const { core, conn } = setup();
    await init(core);

    conn.stallOn((sql) => sql.startsWith("INSERT"));
    core.handle({ type: "multi-rebuild", satelliteId: SAT_ID, rows: rows(0, 1), latestT: 1 });
    const idle = core.whenIdle();
    await flush();
    expect(conn.stalledCount).toBe(1);

    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(2), latestT: 2 });

    conn.stallOn(null);
    conn.releaseStalled();
    await idle;
    await core.tickOnce();

    expect(conn.tValuesOf(TABLE)).toEqual([0, 1, 2]);
  });

  it("drops rows queued before a rebuild and keeps those queued after", async () => {
    const { core, conn } = setup();
    await init(core);

    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(0, 1), latestT: 1 });
    core.handle({ type: "multi-rebuild", satelliteId: SAT_ID, rows: rows(10, 11), latestT: 11 });
    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(12), latestT: 12 });
    await core.whenIdle();
    await core.tickOnce();

    expect(conn.tValuesOf(TABLE)).toEqual([10, 11, 12]);
  });

  it("keeps rows that arrive after a rebuild queued behind a running tick", async () => {
    const { core, conn } = setup();
    await init(core);
    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();

    conn.stallOn((sql) => sql.startsWith("INSERT"));
    core.handle({ type: "multi-rebuild", satelliteId: SAT_ID, rows: rows(10, 11), latestT: 11 });
    await flush();
    expect(conn.stalledCount).toBe(1);
    const tick = core.tickOnce();

    core.handle({ type: "multi-rebuild", satelliteId: SAT_ID, rows: rows(20, 21), latestT: 21 });
    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(22), latestT: 22 });

    conn.stallOn(null);
    conn.releaseStalled();
    await tick;
    await core.whenIdle();
    await core.tickOnce();

    expect(conn.tValuesOf(TABLE)).toEqual([20, 21, 22]);
  });

  it("keeps the window at rows that arrive while a rebuild runs", async () => {
    const { core, conn } = setup();
    await init(core);
    core.handle({ type: "multi-configure", timeRange: 50, maxPoints: 2000 });

    conn.stallOn((sql) => sql.startsWith("INSERT"));
    core.handle({ type: "multi-rebuild", satelliteId: SAT_ID, rows: rows(0, 1), latestT: 1 });
    await flush();
    expect(conn.stalledCount).toBe(1);

    // Streaming has already moved past the rebuild's own latest t.
    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(100), latestT: 100 });

    conn.stallOn(null);
    conn.releaseStalled();
    await core.whenIdle();
    const from = conn.queries.length;
    await core.tickOnce();

    const queried = conn.queries.slice(from).filter((q) => q.includes("t >= "));
    expect(queried.length).toBeGreaterThan(0);
    expect(queried.every((q) => q.includes("t >= 50"))).toBe(true);
  });

  it("does not broadcast the old dataset while a replacement is queued", async () => {
    const { core, conn, posted } = setup();
    await init(core);
    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();
    expect(broadcastMetricCounts(posted)).toEqual([1]);

    // A replacement arrives while this tick's flush is in flight.
    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(2), latestT: 2 });
    conn.stallOn((sql) => sql.startsWith("INSERT"));
    const tick = core.tickOnce();
    await flush();
    expect(conn.stalledCount).toBe(1);

    core.handle({ type: "multi-rebuild", satelliteId: SAT_ID, rows: rows(30, 31), latestT: 31 });
    conn.stallOn(null);
    conn.releaseStalled();
    await tick;

    // The tables still hold the dataset the replacement will delete.
    expect(broadcastMetricCounts(posted)).toEqual([1]);

    await core.whenIdle();
    await core.tickOnce();
    expect(broadcastMetricCounts(posted)).toEqual([1, 1]);
    expect(conn.tValuesOf(TABLE)).toEqual([30, 31]);
  });

  it("does not broadcast a payload for a window that changed mid-query", async () => {
    const { core, conn, posted } = setup();
    await init(core);
    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();
    expect(broadcastMetricCounts(posted)).toEqual([1]);

    // The window changes while the per-satellite SELECT is in flight.
    conn.stallOn((sql) => sql.includes("r - 6378.137"));
    const tick = core.tickOnce();
    await flush();
    expect(conn.stalledCount).toBe(1);

    core.handle({ type: "multi-configure", timeRange: 50, maxPoints: 2000 });
    conn.stallOn(null);
    conn.releaseStalled();
    await tick;

    // The stale payload is dropped; the next tick queries the new window.
    expect(broadcastMetricCounts(posted)).toEqual([1]);
    await core.tickOnce();
    expect(broadcastMetricCounts(posted)).toEqual([1, 1]);
  });

  it("leaves the table untouched when the rebuild fails, and retries it", async () => {
    const { core, conn } = setup();
    await init(core);
    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();
    expect(conn.tValuesOf(TABLE)).toEqual([0, 1]);

    conn.failOnNthInsert(1);
    core.handle({ type: "multi-rebuild", satelliteId: SAT_ID, rows: rows(10, 11), latestT: 11 });
    await core.whenIdle();

    expect(conn.tValuesOf(TABLE)).toEqual([0, 1]);
    expect(conn.queries.some((q) => q.startsWith("ROLLBACK"))).toBe(true);

    conn.failOn(null);
    await core.tickOnce();
    expect(conn.tValuesOf(TABLE)).toEqual([10, 11]);
  });

  it("does not append to a table whose abandoned dataset could not be emptied", async () => {
    const { core, conn } = setup();
    await init(core);
    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();
    expect(conn.tValuesOf(TABLE)).toEqual([0, 1]);

    let deletes = 0;
    conn.failOn((sql) => {
      if (sql.includes("(10,7010)")) return true;
      if (sql.startsWith("DELETE")) {
        deletes++;
        return deletes >= 5; // 4 rebuild attempts, then the abandoning ones
      }
      return false;
    });
    core.handle({ type: "multi-rebuild", satelliteId: SAT_ID, rows: rows(10, 11), latestT: 11 });
    await core.whenIdle();
    for (let i = 0; i < 3; i++) await core.tickOnce();

    expect(conn.tValuesOf(TABLE)).toEqual([0, 1]);

    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(50), latestT: 50 });
    await core.tickOnce();
    expect(conn.tValuesOf(TABLE)).toEqual([0, 1]);

    conn.failOn(null);
    await core.tickOnce();
    expect(conn.tValuesOf(TABLE)).toEqual([50]);
  });

  it("neither queries nor re-empties a table around a failed abandoning delete", async () => {
    const { core, conn, posted } = setup();
    await init(core);
    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();
    const sent = broadcastMetricCounts(posted).length;

    // Give up on a replacement, and fail the delete that empties the table.
    let deletes = 0;
    conn.failOn((sql) => {
      if (sql.includes("(10,7010)")) return true;
      if (sql.startsWith("DELETE")) {
        deletes++;
        return deletes >= 5;
      }
      return false;
    });
    core.handle({ type: "multi-rebuild", satelliteId: SAT_ID, rows: rows(10, 11), latestT: 11 });
    await core.whenIdle();
    for (let i = 0; i < 3; i++) await core.tickOnce();
    expect(conn.tValuesOf(TABLE)).toEqual([0, 1]);

    // The table holds a dataset that is on its way out: it must not be
    // queried and broadcast in the meantime.
    await core.tickOnce();
    expect(broadcastMetricCounts(posted)).toHaveLength(sent);

    // A new replacement then succeeds: it already removed everything the
    // failed emptying was after, so it must not be deleted afterwards.
    conn.failOn(null);
    core.handle({ type: "multi-rebuild", satelliteId: SAT_ID, rows: rows(70, 71), latestT: 71 });
    await core.whenIdle();
    expect(conn.tValuesOf(TABLE)).toEqual([70, 71]);
    for (let i = 0; i < 3; i++) await core.tickOnce();
    expect(conn.tValuesOf(TABLE)).toEqual([70, 71]);
  });

  it("broadcasts an empty payload once every satellite is empty", async () => {
    const { core, conn, posted } = setup();
    await init(core);
    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();
    expect(broadcastMetricCounts(posted)).toEqual([1]);

    core.handle({ type: "multi-rebuild", satelliteId: SAT_ID, rows: [], latestT: -Infinity });
    await core.whenIdle();

    expect(conn.tValuesOf(TABLE)).toEqual([]);
    expect(broadcastMetricCounts(posted)).toEqual([1, 0]);

    // Only once: a second empty rebuild does not re-broadcast the same
    // "nothing to show" state.
    core.handle({ type: "multi-rebuild", satelliteId: SAT_ID, rows: [], latestT: -Infinity });
    await core.whenIdle();
    await core.tickOnce();
    expect(broadcastMetricCounts(posted)).toEqual([1, 0]);
  });

  it("does not resurrect rows of the replaced dataset from a failed flush", async () => {
    const { core, conn } = setup();
    await init(core);

    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(0, 1), latestT: 1 });
    conn.stallOn((sql) => sql.startsWith("INSERT"));
    conn.failOn((sql) => sql.includes("(0,7000)"));
    const tick = core.tickOnce();
    await flush();
    expect(conn.stalledCount).toBe(1);

    core.handle({ type: "multi-rebuild", satelliteId: SAT_ID, rows: rows(10, 11), latestT: 11 });

    conn.stallOn(null);
    conn.releaseStalled();
    await tick; // the flush fails here, after the replacement was accepted
    conn.failOn(null);
    await core.whenIdle();
    await core.tickOnce();

    expect(conn.tValuesOf(TABLE)).toEqual([10, 11]);
  });

  it("drops a rebuild that keeps failing, and empties the table", async () => {
    const { core, conn, posted } = setup();
    await init(core);
    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();

    conn.failOn((sql) => sql.includes("(10,7010)"));
    core.handle({ type: "multi-rebuild", satelliteId: SAT_ID, rows: rows(10, 11), latestT: 11 });
    await core.whenIdle();
    for (let i = 0; i < 5; i++) await core.tickOnce();

    // 1 initial attempt + 3 retries, then no more.
    expect(conn.queries.filter((q) => q.includes("(10,7010)")).length).toBe(4);
    // Keeping the old dataset would splice it with the rows that keep
    // arriving, so it is dropped and reported.
    expect(conn.tValuesOf(TABLE)).toEqual([]);
    expect(posted.some((m) => m.type === "error")).toBe(true);
  });

  it("re-inserts a failed flush without duplicating rows", async () => {
    const { core, conn } = setup();
    await init(core);
    conn.failOnNthInsert(1);
    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();
    expect(conn.tValuesOf(TABLE)).toEqual([]);

    conn.failOn(null);
    await core.tickOnce();
    expect(conn.tValuesOf(TABLE)).toEqual([0, 1]);
  });

  it("stops retrying an insert that keeps failing", async () => {
    const { core, conn } = setup();
    await init(core);
    conn.failOn((sql) => sql.startsWith("INSERT"));
    core.handle({ type: "multi-ingest", satelliteId: SAT_ID, rows: rows(0, 1), latestT: 1 });

    for (let i = 0; i < 8; i++) await core.tickOnce();
    // 1 initial attempt + 3 retries, then the batch is dropped instead of
    // being retried forever (the single-satellite engine's bound).
    expect(conn.queries.filter((q) => q.startsWith("INSERT")).length).toBe(4);
  });
});
