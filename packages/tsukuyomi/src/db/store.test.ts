import { describe, it, expect } from "vitest";
import {
  buildCreateTableSQL,
  buildInsertSQL,
  buildDerivedQuery,
} from "./store.js";
import type { TableSchema } from "../types.js";

// --- Test schema ---

interface TestPoint {
  t: number;
  value: number;
  extra: number;
}

const testSchema: TableSchema<TestPoint> = {
  tableName: "test_data",
  columns: [
    { name: "t", type: "DOUBLE" },
    { name: "value", type: "DOUBLE" },
    { name: "extra", type: "DOUBLE" },
  ],
  derived: [
    { name: "doubled", sql: "value * 2" },
    { name: "sum", sql: "value + extra" },
  ],
  toRow: (p) => [p.t, p.value, p.extra],
};

// Schema with no derived columns
const noDerivedSchema: TableSchema<TestPoint> = {
  tableName: "raw_data",
  columns: [
    { name: "t", type: "DOUBLE" },
    { name: "value", type: "DOUBLE" },
    { name: "extra", type: "DOUBLE" },
  ],
  derived: [],
  toRow: (p) => [p.t, p.value, p.extra],
};

// Schema with mixed column types
const mixedTypeSchema: TableSchema = {
  tableName: "mixed_data",
  columns: [
    { name: "t", type: "DOUBLE" },
    { name: "count", type: "INTEGER" },
    { name: "id", type: "BIGINT" },
    { name: "approx", type: "FLOAT" },
  ],
  derived: [],
  toRow: (p) => [p.t, p["count"], p["id"], p["approx"]],
};

// ---------------------------------------------------------------------------
// buildCreateTableSQL
// ---------------------------------------------------------------------------

describe("buildCreateTableSQL", () => {
  it("generates CREATE TABLE with all columns", () => {
    const sql = buildCreateTableSQL(testSchema);
    expect(sql).toContain("CREATE OR REPLACE TABLE test_data");
    expect(sql).toContain("t DOUBLE");
    expect(sql).toContain("value DOUBLE");
    expect(sql).toContain("extra DOUBLE");
  });

  it("uses correct column types", () => {
    const sql = buildCreateTableSQL(mixedTypeSchema);
    expect(sql).toContain("t DOUBLE");
    expect(sql).toContain("count INTEGER");
    expect(sql).toContain("id BIGINT");
    expect(sql).toContain("approx FLOAT");
  });
});

// ---------------------------------------------------------------------------
// buildInsertSQL
// ---------------------------------------------------------------------------

describe("buildInsertSQL", () => {
  it("generates correct VALUES for a batch of points", () => {
    const points: TestPoint[] = [
      { t: 0, value: 1.5, extra: 2.5 },
      { t: 1, value: 3.0, extra: 4.0 },
    ];
    const sql = buildInsertSQL(testSchema, points);
    expect(sql).toContain("INSERT INTO test_data VALUES");
    expect(sql).toContain("(0,1.5,2.5)");
    expect(sql).toContain("(1,3,4)");
  });

  it("returns empty string for empty batch", () => {
    const sql = buildInsertSQL(testSchema, []);
    expect(sql).toBe("");
  });
});

// ---------------------------------------------------------------------------
// buildDerivedQuery
// ---------------------------------------------------------------------------

describe("buildDerivedQuery", () => {
  it("generates basic SELECT with derived column expressions", () => {
    const sql = buildDerivedQuery(testSchema);
    expect(sql).toContain("SELECT");
    expect(sql).toContain("t");
    expect(sql).toContain("value * 2 AS doubled");
    expect(sql).toContain("value + extra AS sum");
    expect(sql).toContain("FROM test_data");
    expect(sql).toContain("ORDER BY t");
    // No downsampling
    expect(sql).not.toContain("ROW_NUMBER");
  });

  it("includes WHERE clause when tMin is provided", () => {
    const sql = buildDerivedQuery(testSchema, 500);
    expect(sql).toContain("WHERE t >= 500");
    expect(sql).toContain("ORDER BY t");
  });

  it("uses row-count based (NTILE) downsampling when maxPoints is provided", () => {
    const sql = buildDerivedQuery(testSchema, undefined, 2000);
    // Should use NTILE for even row-count distribution, not time-bucket
    expect(sql).toContain("NTILE(2000)");
    expect(sql).toContain("ROW_NUMBER");
    expect(sql).toContain("PARTITION BY bucket");
    expect(sql).toContain("ORDER BY t");
  });

  it("picks first point per bucket (rn = 1)", () => {
    const sql = buildDerivedQuery(testSchema, undefined, 100);
    expect(sql).toContain("rn = 1");
    expect(sql).toContain("NTILE(100)");
    expect(sql).toContain("PARTITION BY bucket");
  });

  it("includes both tMin filter and maxPoints", () => {
    const sql = buildDerivedQuery(testSchema, 500, 2000);
    expect(sql).toContain("WHERE t >= 500");
    expect(sql).toContain("NTILE(2000)");
  });

  it("handles case with no derived columns (just SELECT t)", () => {
    const sql = buildDerivedQuery(noDerivedSchema);
    expect(sql).toContain("SELECT");
    expect(sql).toContain("t");
    expect(sql).toContain("FROM raw_data");
    expect(sql).toContain("ORDER BY t");
    // Should not have any AS clauses for derived columns
    expect(sql).not.toContain(" AS ");
  });

  it("passes all rows when maxPoints is 0", () => {
    const sql = buildDerivedQuery(testSchema, undefined, 0);
    expect(sql).not.toContain("bucket");
  });

  it("uses CTE with filtered, bucketed, and ranked for downsampled query", () => {
    const sql = buildDerivedQuery(testSchema, 100, 500);
    expect(sql).toContain("WITH filtered AS");
    expect(sql).toContain("bucketed AS");
    expect(sql).toContain("ranked AS");
    expect(sql).toContain("NTILE(500)");
  });

  it("bypasses downsampling when total <= maxPoints", () => {
    const sql = buildDerivedQuery(testSchema, undefined, 2000);
    expect(sql).toContain("total <= 2000");
  });
});
