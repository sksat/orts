import type { TableSchema, TimePoint } from "@sksat/uneri";
import type { WorkerTableSchema } from "@sksat/uneri/workerProtocol";
import { describe, expect, it } from "vitest";
import { type BaseSchemaSyncTarget, forwardBaseSchema } from "./useMultiSatelliteStoreWorker.js";

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

function fakeClient(): { sent: WorkerTableSchema[]; client: BaseSchemaSyncTarget } {
  const sent: WorkerTableSchema[] = [];
  return { sent, client: { updateSchema: (schema) => sent.push(schema) } };
}

describe("forwardBaseSchema", () => {
  it("forwards a schema whose derived SQL changed", () => {
    const { sent, client } = fakeClient();

    const now = forwardBaseSchema(client, MOON_SCHEMA, EARTH_SCHEMA);

    expect(sent).toHaveLength(1);
    expect(sent[0].derived).toEqual([{ name: "altitude", sql: "r - 1737.4" }]);
    expect(now).toBe(MOON_SCHEMA);
  });

  it("forwards a schema whose columns changed", () => {
    const { sent, client } = fakeClient();
    const withSpeed: TableSchema<Point> = {
      ...EARTH_SCHEMA,
      columns: [...EARTH_SCHEMA.columns, { name: "v", type: "DOUBLE" }],
    };

    forwardBaseSchema(client, withSpeed, EARTH_SCHEMA);

    expect(sent).toHaveLength(1);
    expect(sent[0].columns.map((c) => c.name)).toEqual(["t", "r", "v"]);
  });

  it("does not send anything when the schema is the same object", () => {
    const { sent, client } = fakeClient();

    const now = forwardBaseSchema(client, EARTH_SCHEMA, EARTH_SCHEMA);

    expect(sent).toEqual([]);
    expect(now).toBe(EARTH_SCHEMA);
  });

  it("does not send anything for an equal schema rebuilt as a new object", () => {
    const { sent, client } = fakeClient();
    const rebuilt: TableSchema<Point> = {
      ...EARTH_SCHEMA,
      columns: EARTH_SCHEMA.columns.map((c) => ({ ...c })),
      derived: EARTH_SCHEMA.derived.map((d) => ({ ...d })),
    };

    const now = forwardBaseSchema(client, rebuilt, EARTH_SCHEMA);

    expect(sent).toEqual([]);
    // The identity is still adopted, so the comparison is not repeated every
    // drain for a caller that builds a fresh object each render.
    expect(now).toBe(rebuilt);
  });

  it("ignores the base schema's own table name", () => {
    const { sent, client } = fakeClient();
    const renamed: TableSchema<Point> = { ...EARTH_SCHEMA, tableName: "orbit_sat_2" };

    forwardBaseSchema(client, renamed, EARTH_SCHEMA);

    // The Worker names a table per satellite; the base name is not what it uses.
    expect(sent).toEqual([]);
  });
});
