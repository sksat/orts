import type { AsyncDuckDBConnection } from "@duckdb/duckdb-wasm";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FakeDuckDBConn } from "../testing/fakeDuckDB.js";
import { ChartDataCore } from "./chartDataCore.js";
import type { RowTuple, WorkerTableSchema, WorkerToMainMessage } from "./protocol.js";

const EARTH_SCHEMA: WorkerTableSchema = {
  tableName: "orbit",
  columns: [
    { name: "t", type: "DOUBLE" },
    { name: "r", type: "DOUBLE" },
  ],
  derived: [{ name: "altitude", sql: "r - 6378.137" }],
};

/** Same columns, different central body radius baked into the derived SQL. */
const MOON_SCHEMA: WorkerTableSchema = {
  ...EARTH_SCHEMA,
  derived: [{ name: "altitude", sql: "r - 1737.4" }],
};

function rows(...ts: number[]): RowTuple[] {
  return ts.map((t) => [t, 7000 + t]);
}

function setup() {
  const conn = new FakeDuckDBConn();
  const posted: WorkerToMainMessage[] = [];
  const core = new ChartDataCore({
    post: (msg) => {
      posted.push(msg);
    },
    initDb: async () => ({
      connect: async () => conn as unknown as AsyncDuckDBConnection,
    }),
  });
  return { conn, posted, core };
}

async function init(core: ChartDataCore, schema = EARTH_SCHEMA, coldRefreshEveryN?: number) {
  core.handle({ type: "init", schema, tickInterval: 1_000_000, coldRefreshEveryN });
  core.handle({ type: "configure", timeRange: null, maxPoints: 2000 });
  await core.whenIdle();
}

/** Let already-queued promise callbacks run (no timers involved). */
async function flush(times = 50) {
  for (let i = 0; i < times; i++) await Promise.resolve();
}

/** Chart payloads the Worker sent, as row counts. */
function chartRowCounts(posted: WorkerToMainMessage[]): number[] {
  return posted
    .filter(
      (m): m is Extract<WorkerToMainMessage, { type: "chart-data" }> => m.type === "chart-data",
    )
    .map((m) => new Float64Array(m.buffers[0]).length);
}

describe("ChartDataCore schema updates", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    return () => {
      vi.useRealTimers();
    };
  });

  it("queries with the derived expressions of the latest schema", async () => {
    const { core, conn } = setup();
    await init(core);
    core.handle({ type: "ingest", rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();
    expect(conn.queries.some((q) => q.includes("r - 6378.137"))).toBe(true);

    core.handle({ type: "update-schema", schema: MOON_SCHEMA });
    await core.whenIdle();
    const before = conn.queries.length;
    await core.tickOnce();

    const afterUpdate = conn.queries.slice(before);
    expect(afterUpdate.some((q) => q.includes("r - 1737.4"))).toBe(true);
    expect(afterUpdate.some((q) => q.includes("r - 6378.137"))).toBe(false);
    // Only the expressions changed, so the stored rows are kept.
    expect(conn.tValuesOf("orbit")).toEqual([0, 1]);
  });

  it("recreates the table and clears the chart when the columns change", async () => {
    const { core, conn, posted } = setup();
    await init(core);
    core.handle({ type: "ingest", rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();
    expect(chartRowCounts(posted)).toEqual([2]);

    core.handle({
      type: "update-schema",
      schema: {
        tableName: "orbit",
        columns: [
          { name: "t", type: "DOUBLE" },
          { name: "r", type: "DOUBLE" },
          { name: "extra", type: "DOUBLE" },
        ],
        derived: [{ name: "altitude", sql: "r - 1737.4" }],
      },
    });
    await core.whenIdle();

    expect(conn.queries.some((q) => q.startsWith("CREATE OR REPLACE TABLE orbit"))).toBe(true);
    expect(conn.tValuesOf("orbit")).toEqual([]);
    expect(chartRowCounts(posted)).toEqual([2, 0]);
  });

  it("keeps the window at rows that arrive while the table is recreated", async () => {
    const { core, conn } = setup();
    await init(core);
    core.handle({ type: "configure", timeRange: 50, maxPoints: 2000 });

    conn.stallOn((sql) => sql.startsWith("CREATE OR REPLACE TABLE"));
    core.handle({
      type: "update-schema",
      schema: {
        tableName: "orbit",
        columns: [
          { name: "t", type: "DOUBLE" },
          { name: "r", type: "DOUBLE" },
          { name: "extra", type: "DOUBLE" },
        ],
        derived: [{ name: "altitude", sql: "r - 1737.4" }],
      },
    });
    await flush();
    expect(conn.stalledCount).toBe(1);

    // Rows built with the new schema arrive while the table is recreated.
    core.handle({ type: "ingest", rows: [[100, 7100, 0]], latestT: 100 });
    conn.stallOn(null);
    conn.releaseStalled();
    await core.whenIdle();
    const from = conn.queries.length;
    await core.tickOnce();

    const queried = conn.queries.slice(from).filter((q) => q.includes("t >= "));
    expect(queried.length).toBeGreaterThan(0);
    expect(queried.every((q) => q.includes("t >= 50"))).toBe(true);
  });

  it("recreates the table before querying it when the columns change mid-flush", async () => {
    const { core, conn, posted } = setup();
    await init(core);
    core.handle({ type: "ingest", rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();
    const from = conn.queries.length;
    const sentBefore = chartRowCounts(posted).length;

    // A column change arrives while this tick's insert is in flight.
    core.handle({ type: "ingest", rows: rows(2), latestT: 2 });
    conn.stallOn((sql) => sql.startsWith("INSERT"));
    const tick = core.tickOnce();
    await flush();
    expect(conn.stalledCount).toBe(1);

    core.handle({
      type: "update-schema",
      schema: {
        tableName: "orbit",
        columns: [
          { name: "t", type: "DOUBLE" },
          { name: "r", type: "DOUBLE" },
          { name: "extra", type: "DOUBLE" },
        ],
        derived: [{ name: "altitude", sql: "r - 1737.4" }],
      },
    });
    conn.stallOn(null);
    conn.releaseStalled();
    await tick;

    // The rows still in the table were built with the old columns, so this
    // tick must not query them - and must not broadcast them as new-schema
    // values. The recreation happens first.
    const after = conn.queries.slice(from);
    const createAt = after.findIndex((q) => q.startsWith("CREATE OR REPLACE TABLE"));
    const selectAt = after.findIndex((q) => q.startsWith("SELECT") || q.startsWith("WITH"));
    expect(selectAt === -1 || (createAt !== -1 && createAt < selectAt)).toBe(true);
    expect(
      chartRowCounts(posted)
        .slice(sentBefore)
        .every((n) => n === 0),
    ).toBe(true);
  });

  it("still recreates the table when the column change lands during another repair", async () => {
    const { core, conn } = setup();
    await init(core);
    core.handle({ type: "ingest", rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();

    // Drive the engine into abandoning the dataset, and stall the delete that
    // empties the table (the 5th: one per rebuild attempt, then this one).
    let deletes = 0;
    conn.failOn((sql) => sql.includes("(10,7010)"));
    conn.stallOn((sql) => sql.startsWith("DELETE") && ++deletes === 5);
    core.handle({ type: "rebuild", rows: rows(10, 11), latestT: 11 });
    await core.whenIdle();
    for (let i = 0; i < 2; i++) await core.tickOnce();
    const third = core.tickOnce();
    await flush();
    expect(conn.stalledCount).toBe(1);

    // A column change arrives while that delete is in flight.
    const from = conn.queries.length;
    core.handle({
      type: "update-schema",
      schema: {
        tableName: "orbit",
        columns: [
          { name: "t", type: "DOUBLE" },
          { name: "r", type: "DOUBLE" },
          { name: "extra", type: "DOUBLE" },
        ],
        derived: [{ name: "altitude", sql: "r - 1737.4" }],
      },
    });
    conn.failOn(null);
    conn.stallOn(null);
    conn.releaseStalled();
    await third;
    await core.whenIdle();

    // The recreation must not be lost by the delete that finished after it
    // was requested.
    expect(conn.queries.slice(from).some((q) => q.startsWith("CREATE OR REPLACE TABLE"))).toBe(
      true,
    );

    // And rows built with the new columns land in the recreated table.
    core.handle({ type: "ingest", rows: [[200, 7200, 0]], latestT: 200 });
    await core.tickOnce();
    expect(conn.tValuesOf("orbit")).toEqual([200]);
  });

  it("moves to the new table and drops the old one when the name changes", async () => {
    const { core, conn } = setup();
    await init(core);
    core.handle({ type: "ingest", rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();

    core.handle({
      type: "update-schema",
      schema: { ...EARTH_SCHEMA, tableName: "orbit_v2" },
    });
    await core.whenIdle();

    expect(conn.queries).toContain("DROP TABLE IF EXISTS orbit");
    expect(conn.queries.some((q) => q.startsWith("CREATE OR REPLACE TABLE orbit_v2"))).toBe(true);

    core.handle({ type: "ingest", rows: rows(2), latestT: 2 });
    await core.tickOnce();
    expect(conn.tValuesOf("orbit_v2")).toEqual([2]);
  });

  it("uses the new schema on the next tick even if a query was in flight", async () => {
    const { core, conn } = setup();
    await init(core);
    core.handle({ type: "ingest", rows: rows(0, 1), latestT: 1 });

    // The cold query of the old schema is in flight when the schema changes.
    conn.stallOn((sql) => sql.includes("r - 6378.137"));
    const tick = core.tickOnce();
    await flush();
    expect(conn.stalledCount).toBe(1);

    core.handle({ type: "update-schema", schema: MOON_SCHEMA });
    conn.stallOn(null);
    conn.releaseStalled();
    await tick;
    await core.whenIdle();

    // The stale answer must not be cached as the current snapshot: the very
    // next tick has to re-query, not wait for the periodic cold refresh.
    const from = conn.queries.length;
    await core.tickOnce();
    expect(conn.queries.slice(from).some((q) => q.includes("r - 1737.4"))).toBe(true);
  });
});

describe("ChartDataCore rebuild", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    return () => {
      vi.useRealTimers();
    };
  });

  it("keeps rows that arrive while a rebuild is in flight", async () => {
    const { core, conn } = setup();
    await init(core);

    conn.stallOn((sql) => sql.startsWith("INSERT"));
    core.handle({ type: "rebuild", rows: rows(0, 1), latestT: 1 });
    const idle = core.whenIdle();
    await flush();
    expect(conn.stalledCount).toBe(1);

    // Streaming continues while the rebuild transaction is open.
    core.handle({ type: "ingest", rows: rows(2), latestT: 2 });

    conn.stallOn(null);
    conn.releaseStalled();
    await idle;
    await core.tickOnce();

    expect(conn.tValuesOf("orbit")).toEqual([0, 1, 2]);
  });

  it("drops rows queued before a rebuild and keeps those queued after", async () => {
    const { core, conn } = setup();
    await init(core);

    // Not flushed yet: these belong to the dataset the rebuild replaces.
    core.handle({ type: "ingest", rows: rows(0, 1), latestT: 1 });
    core.handle({ type: "rebuild", rows: rows(10, 11), latestT: 11 });
    core.handle({ type: "ingest", rows: rows(12), latestT: 12 });
    await core.whenIdle();
    await core.tickOnce();

    expect(conn.tValuesOf("orbit")).toEqual([10, 11, 12]);
  });

  it("keeps rows that arrive after a rebuild queued behind a running tick", async () => {
    const { core, conn } = setup();
    await init(core);
    core.handle({ type: "ingest", rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();

    // A first replacement is committing; a tick is queued behind it.
    conn.stallOn((sql) => sql.startsWith("INSERT"));
    core.handle({ type: "rebuild", rows: rows(10, 11), latestT: 11 });
    await flush();
    expect(conn.stalledCount).toBe(1);
    const tick = core.tickOnce();

    // A newer replacement lands behind that tick, then streaming resumes.
    core.handle({ type: "rebuild", rows: rows(20, 21), latestT: 21 });
    core.handle({ type: "ingest", rows: rows(22), latestT: 22 });

    conn.stallOn(null);
    conn.releaseStalled();
    await tick;
    await core.whenIdle();
    await core.tickOnce();

    // t=22 arrived after the last replacement, so it must survive it.
    expect(conn.tValuesOf("orbit")).toEqual([20, 21, 22]);
  });

  it("queries with the new schema when it arrives during the insert flush", async () => {
    const { core, conn } = setup();
    await init(core);

    // The flush of the tick is in flight when the schema changes: the query
    // that follows it, in the same tick, must already use the new one.
    core.handle({ type: "ingest", rows: rows(0, 1), latestT: 1 });
    conn.stallOn((sql) => sql.startsWith("INSERT"));
    const tick = core.tickOnce();
    await flush();
    expect(conn.stalledCount).toBe(1);

    core.handle({ type: "update-schema", schema: MOON_SCHEMA });
    conn.stallOn(null);
    conn.releaseStalled();
    await tick;
    await core.whenIdle();

    // The same tick queries, and with the new expressions.
    expect(conn.queries.some((q) => q.includes("r - 1737.4"))).toBe(true);
    expect(conn.queries.some((q) => q.includes("r - 6378.137"))).toBe(false);
  });

  it("keeps the window at rows that arrive while a rebuild runs", async () => {
    const { core, conn } = setup();
    await init(core);
    core.handle({ type: "configure", timeRange: 50, maxPoints: 2000 });

    conn.stallOn((sql) => sql.startsWith("INSERT"));
    core.handle({ type: "rebuild", rows: rows(0, 1), latestT: 1 });
    await flush();
    expect(conn.stalledCount).toBe(1);

    // Streaming has already moved past the rebuild's own latest t.
    core.handle({ type: "ingest", rows: rows(100), latestT: 100 });

    conn.stallOn(null);
    conn.releaseStalled();
    await core.whenIdle();
    const from = conn.queries.length;
    await core.tickOnce();

    // The 50 s window is anchored at t=100, not rolled back to the rebuild's
    // t=1 (which would query `t >= -49` and re-show the whole dataset).
    const queried = conn.queries.slice(from).filter((q) => q.includes("t >= "));
    expect(queried.length).toBeGreaterThan(0);
    expect(queried.every((q) => q.includes("t >= 50"))).toBe(true);
  });

  it("does not query the table while a rebuild transaction is open", async () => {
    const { core, conn } = setup();
    await init(core);
    // Give the engine data first, so a tick really would query.
    core.handle({ type: "ingest", rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();
    const from = conn.queries.length;

    conn.stallOn((sql) => sql.startsWith("INSERT"));
    core.handle({ type: "rebuild", rows: rows(2, 3), latestT: 3 });
    const rebuildIdle = core.whenIdle();
    await flush();
    expect(conn.stalledCount).toBe(1);

    const tick = core.tickOnce();
    await flush();
    conn.stallOn(null);
    conn.releaseStalled();
    await rebuildIdle;
    await tick;

    const after = conn.queries.slice(from);
    const beginAt = after.findIndex((q) => q.startsWith("BEGIN"));
    const commitAt = after.findIndex((q) => q.startsWith("COMMIT"));
    const selects = after
      .map((q, i) => ({ q, i }))
      .filter(({ q }) => q.startsWith("SELECT") || q.startsWith("WITH"))
      .map(({ i }) => i);
    expect(beginAt).toBeGreaterThanOrEqual(0);
    expect(commitAt).toBeGreaterThan(beginAt);
    expect(selects.length).toBeGreaterThan(0);
    // No read of a half-replaced table.
    expect(selects.some((i) => i > beginAt && i < commitAt)).toBe(false);
  });

  it("does not resurrect rows of the replaced dataset from a failed flush", async () => {
    const { core, conn } = setup();
    await init(core);

    // A flush of the old dataset is in flight and about to fail.
    core.handle({ type: "ingest", rows: rows(0, 1), latestT: 1 });
    conn.stallOn((sql) => sql.startsWith("INSERT"));
    conn.failOn((sql) => sql.includes("(0,7000)"));
    const tick = core.tickOnce();
    await flush();
    expect(conn.stalledCount).toBe(1);

    // The whole dataset is replaced while that flush is stalled.
    core.handle({ type: "rebuild", rows: rows(10, 11), latestT: 11 });

    conn.stallOn(null);
    conn.releaseStalled();
    await tick; // the flush fails here, after the replacement was accepted
    conn.failOn(null);
    await core.whenIdle();
    await core.tickOnce();

    expect(conn.tValuesOf("orbit")).toEqual([10, 11]);
  });

  it("coalesces rebuilds that pile up behind an in-flight one", async () => {
    const { core, conn } = setup();
    await init(core);

    conn.stallOn((sql) => sql.startsWith("INSERT"));
    core.handle({ type: "rebuild", rows: rows(0, 1), latestT: 1 });
    await flush();
    expect(conn.stalledCount).toBe(1);

    // Two more full replacements arrive while the first one is committing.
    core.handle({ type: "rebuild", rows: rows(5, 6), latestT: 6 });
    core.handle({ type: "rebuild", rows: rows(20, 21), latestT: 21 });

    conn.stallOn(null);
    conn.releaseStalled();
    await core.whenIdle();

    expect(conn.tValuesOf("orbit")).toEqual([20, 21]);
    // The superseded middle dataset is never written at all.
    expect(conn.queries.some((q) => q.includes("(5,7005)"))).toBe(false);
  });

  it("leaves the table untouched when the rebuild fails, and retries it", async () => {
    const { core, conn } = setup();
    await init(core);
    core.handle({ type: "ingest", rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();
    expect(conn.tValuesOf("orbit")).toEqual([0, 1]);

    conn.failOnNthInsert(1);
    core.handle({ type: "rebuild", rows: rows(10, 11), latestT: 11 });
    await core.whenIdle();

    // All-or-nothing: neither emptied nor half-filled.
    expect(conn.tValuesOf("orbit")).toEqual([0, 1]);
    expect(conn.queries.some((q) => q.startsWith("ROLLBACK"))).toBe(true);

    conn.failOn(null);
    await core.tickOnce();
    expect(conn.tValuesOf("orbit")).toEqual([10, 11]);
  });

  it("drops a rebuild that keeps failing, and empties the table", async () => {
    const { core, conn, posted } = setup();
    await init(core);
    core.handle({ type: "ingest", rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();

    conn.failOn((sql) => sql.includes("(10,7010)"));
    core.handle({ type: "rebuild", rows: rows(10, 11), latestT: 11 });
    await core.whenIdle();
    for (let i = 0; i < 5; i++) await core.tickOnce();

    const inserts = conn.queries.filter((q) => q.includes("(10,7010)")).length;
    // 1 initial attempt + MAX_REBUILD_RETRIES retries, then no more.
    expect(inserts).toBe(4);
    // Keeping the old dataset would splice it with the rows that keep
    // arriving, so it is dropped and reported.
    expect(conn.tValuesOf("orbit")).toEqual([]);
    expect(posted.some((m) => m.type === "error")).toBe(true);
  });

  it("does not append to a table whose abandoned dataset could not be emptied", async () => {
    const { core, conn } = setup();
    await init(core);
    core.handle({ type: "ingest", rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();
    expect(conn.tValuesOf("orbit")).toEqual([0, 1]);

    // The replacement never commits, and the deletes that empty the table
    // after giving up fail twice as well.
    let deletes = 0;
    conn.failOn((sql) => {
      if (sql.includes("(10,7010)")) return true;
      if (sql.startsWith("DELETE")) {
        deletes++;
        return deletes >= 5; // 4 rebuild attempts, then the abandoning ones
      }
      return false;
    });
    core.handle({ type: "rebuild", rows: rows(10, 11), latestT: 11 });
    await core.whenIdle();
    for (let i = 0; i < 3; i++) await core.tickOnce();

    // Emptying failed, so the old dataset is still in the table.
    expect(conn.tValuesOf("orbit")).toEqual([0, 1]);

    // Newer rows must not be appended to it while that is the case.
    core.handle({ type: "ingest", rows: rows(50), latestT: 50 });
    await core.tickOnce();
    expect(conn.tValuesOf("orbit")).toEqual([0, 1]);

    // Once the delete succeeds, streaming resumes into the emptied table.
    conn.failOn(null);
    await core.tickOnce();
    expect(conn.tValuesOf("orbit")).toEqual([50]);
  });

  it("keeps a later successful rebuild instead of emptying it", async () => {
    const { core, conn } = setup();
    await init(core);
    core.handle({ type: "ingest", rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();

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
    core.handle({ type: "rebuild", rows: rows(10, 11), latestT: 11 });
    await core.whenIdle();
    for (let i = 0; i < 3; i++) await core.tickOnce();
    expect(conn.tValuesOf("orbit")).toEqual([0, 1]);

    // A new replacement then succeeds: it already removed everything the
    // failed emptying was after, so it must not be deleted afterwards.
    conn.failOn(null);
    core.handle({ type: "rebuild", rows: rows(70, 71), latestT: 71 });
    await core.whenIdle();
    expect(conn.tValuesOf("orbit")).toEqual([70, 71]);

    for (let i = 0; i < 3; i++) await core.tickOnce();
    expect(conn.tValuesOf("orbit")).toEqual([70, 71]);
  });

  it("does not broadcast a snapshot taken before a rebuild that arrived during compaction", async () => {
    const { core, conn, posted } = setup();
    await init(core, EARTH_SCHEMA, 1); // cold refresh on every tick
    core.handle({ type: "ingest", rows: rows(0, 1), latestT: 1 });

    // The compaction check runs on every 5th cold query and starts with a
    // COUNT(*), which is where this tick is stalled.
    for (let i = 0; i < 4; i++) await core.tickOnce();
    const sent = chartRowCounts(posted).length;
    expect(sent).toBeGreaterThan(0);

    conn.stallOn((sql) => sql.startsWith("SELECT COUNT(*)"));
    const tick = core.tickOnce();
    await flush();
    expect(conn.stalledCount).toBe(1);

    // A full replacement is accepted while the compaction check is in flight.
    core.handle({ type: "rebuild", rows: rows(40, 41), latestT: 41 });
    conn.stallOn(null);
    conn.releaseStalled();
    await tick;

    // The snapshot describes the table the replacement is about to delete.
    expect(chartRowCounts(posted)).toHaveLength(sent);
  });

  it("broadcasts an empty dataset after an empty rebuild", async () => {
    const { core, conn, posted } = setup();
    await init(core);
    core.handle({ type: "ingest", rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();
    expect(chartRowCounts(posted)).toEqual([2]);

    core.handle({ type: "rebuild", rows: [], latestT: -Infinity });
    await core.whenIdle();
    await core.tickOnce();

    expect(conn.tValuesOf("orbit")).toEqual([]);
    expect(chartRowCounts(posted)).toEqual([2, 0]);
  });
});

describe("ChartDataCore ingest", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    return () => {
      vi.useRealTimers();
    };
  });

  it("re-inserts a failed flush without duplicating rows", async () => {
    const { core, conn } = setup();
    await init(core);
    conn.failOnNthInsert(1);
    core.handle({ type: "ingest", rows: rows(0, 1), latestT: 1 });
    await core.tickOnce();
    expect(conn.tValuesOf("orbit")).toEqual([]);
    expect(conn.queries.some((q) => q.startsWith("ROLLBACK"))).toBe(true);

    conn.failOn(null);
    await core.tickOnce();
    expect(conn.tValuesOf("orbit")).toEqual([0, 1]);
  });
});
