import { describe, expect, it } from "vitest";
import { IngestBuffer } from "../db/IngestBuffer.js";
import type { TableSchema, TimePoint } from "../types.js";
import type { RowTuple, WorkerTableSchema } from "../worker/protocol.js";
import type { TimeRange } from "./useTimeSeriesStore.js";
import {
  drainToWorker,
  type WorkerSyncState,
  type WorkerSyncTarget,
} from "./useTimeSeriesStoreWorker.js";

interface Point extends TimePoint {
  r: number;
}

const EARTH_SCHEMA: TableSchema<Point> = {
  tableName: "orbit",
  columns: [
    { name: "t", type: "DOUBLE" },
    { name: "r", type: "DOUBLE" },
  ],
  derived: [{ name: "altitude", sql: "r - 6378.137" }],
  toRow: (p) => [p.t, p.r],
};

/** Same columns, different central body baked into the derived SQL. */
const MOON_SCHEMA: TableSchema<Point> = {
  ...EARTH_SCHEMA,
  derived: [{ name: "altitude", sql: "r - 1737.4" }],
};

type Call =
  | { kind: "update-schema"; schema: WorkerTableSchema }
  | { kind: "ingest"; rows: RowTuple[] }
  | { kind: "rebuild"; rows: RowTuple[] }
  | { kind: "configure"; timeRange: TimeRange; maxPoints: number };

function fakeClient(): { calls: Call[]; client: WorkerSyncTarget } {
  const calls: Call[] = [];
  return {
    calls,
    client: {
      updateSchema: (schema) => calls.push({ kind: "update-schema", schema }),
      ingest: (rows) => calls.push({ kind: "ingest", rows }),
      rebuild: (rows) => calls.push({ kind: "rebuild", rows }),
      configure: (timeRange, maxPoints) => calls.push({ kind: "configure", timeRange, maxPoints }),
    },
  };
}

function syncState(schema: TableSchema<Point>): WorkerSyncState<Point> {
  return { schema, timeRange: null, maxPoints: 2000 };
}

describe("drainToWorker", () => {
  it("sends a changed schema before the rows produced by it", () => {
    const { calls, client } = fakeClient();
    const buffer = new IngestBuffer<Point>();
    buffer.push({ t: 0, r: 7000 });
    const sent = syncState(EARTH_SCHEMA);

    drainToWorker(client, buffer, syncState(MOON_SCHEMA), sent);

    expect(calls.map((c) => c.kind)).toEqual(["update-schema", "ingest"]);
    expect(calls[0]).toMatchObject({
      schema: { derived: [{ name: "altitude", sql: "r - 1737.4" }] },
    });
    expect(sent.schema).toBe(MOON_SCHEMA);
  });

  it("does not resend an unchanged schema", () => {
    const { calls, client } = fakeClient();
    const buffer = new IngestBuffer<Point>();
    const sent = syncState(EARTH_SCHEMA);

    buffer.push({ t: 0, r: 7000 });
    drainToWorker(client, buffer, syncState(EARTH_SCHEMA), sent);
    buffer.push({ t: 1, r: 7001 });
    drainToWorker(client, buffer, syncState(EARTH_SCHEMA), sent);

    expect(calls.filter((c) => c.kind === "update-schema")).toHaveLength(0);
    expect(calls.filter((c) => c.kind === "ingest")).toHaveLength(2);
  });

  it("does not send a message for a new but equal schema object", () => {
    const { calls, client } = fakeClient();
    const buffer = new IngestBuffer<Point>();
    const sent = syncState(EARTH_SCHEMA);

    // A caller that builds the schema inline every render.
    drainToWorker(client, buffer, syncState({ ...EARTH_SCHEMA }), sent);
    drainToWorker(client, buffer, syncState({ ...EARTH_SCHEMA }), sent);

    expect(calls).toEqual([]);
  });

  it("sends a changed schema before a rebuild", () => {
    const { calls, client } = fakeClient();
    const buffer = new IngestBuffer<Point>();
    buffer.markRebuild([
      { t: 0, r: 7000 },
      { t: 1, r: 7001 },
    ]);
    const sent = syncState(EARTH_SCHEMA);

    drainToWorker(client, buffer, syncState(MOON_SCHEMA), sent);

    expect(calls.map((c) => c.kind)).toEqual(["update-schema", "rebuild"]);
    expect(calls[1]).toMatchObject({
      rows: [
        [0, 7000],
        [1, 7001],
      ],
    });
  });

  it("forwards timeRange and maxPoints changes once each", () => {
    const { calls, client } = fakeClient();
    const buffer = new IngestBuffer<Point>();
    const sent = syncState(EARTH_SCHEMA);

    drainToWorker(client, buffer, { schema: EARTH_SCHEMA, timeRange: 300, maxPoints: 2000 }, sent);
    drainToWorker(client, buffer, { schema: EARTH_SCHEMA, timeRange: 300, maxPoints: 2000 }, sent);

    expect(calls).toEqual([{ kind: "configure", timeRange: 300, maxPoints: 2000 }]);
    expect(sent.timeRange).toBe(300);
  });
});
