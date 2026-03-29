import { describe, expect, it } from "vitest";
import type { TableSchema } from "../types.js";
import {
  buildCompactDeleteSQL,
  buildCompactKeepersSQL,
  buildCreateTableSQL,
  buildDerivedQuery,
  buildIncrementalQuery,
  buildInsertSQL,
} from "./store.js";

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
  toRow: (p) => [p.t, p.count, p.id, p.approx],
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

  it("uses time-bucket downsampling when maxPoints is provided", () => {
    const sql = buildDerivedQuery(testSchema, undefined, 2000);
    // Should use equal-duration time buckets for temporal coverage
    expect(sql).toContain("bounds AS");
    expect(sql).toContain("MIN(t) AS t_lo");
    expect(sql).toContain("MAX(t) AS t_hi");
    expect(sql).toContain("FLOOR");
    expect(sql).toContain("2000.0");
    expect(sql).toContain("ROW_NUMBER");
    expect(sql).toContain("PARTITION BY bucket");
    expect(sql).toContain("ORDER BY t");
  });

  it("picks first point per bucket (rn = 1)", () => {
    const sql = buildDerivedQuery(testSchema, undefined, 100);
    expect(sql).toContain("rn = 1");
    expect(sql).toContain("100.0");
    expect(sql).toContain("PARTITION BY bucket");
  });

  it("includes both tMin filter and maxPoints", () => {
    const sql = buildDerivedQuery(testSchema, 500, 2000);
    expect(sql).toContain("WHERE t >= 500");
    expect(sql).toContain("2000.0");
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

  it("uses CTE with filtered, bounds, bucketed, and ranked for downsampled query", () => {
    const sql = buildDerivedQuery(testSchema, 100, 500);
    expect(sql).toContain("WITH filtered AS");
    expect(sql).toContain("bounds AS");
    expect(sql).toContain("bucketed AS");
    expect(sql).toContain("ranked AS");
    expect(sql).toContain("500.0");
  });

  it("bypasses downsampling when total <= maxPoints", () => {
    const sql = buildDerivedQuery(testSchema, undefined, 2000);
    expect(sql).toContain("total <= 2000");
  });

  it("handles t_hi == t_lo edge case with CASE WHEN and clamps with LEAST/GREATEST", () => {
    const sql = buildDerivedQuery(testSchema, undefined, 100);
    expect(sql).toContain("CASE WHEN b.t_hi = b.t_lo THEN 0");
    expect(sql).toContain("LEAST(");
    expect(sql).toContain("GREATEST(");
  });

  it("downsampled query includes MAX(t) boundary", () => {
    const sql = buildDerivedQuery(testSchema, undefined, 100);
    // The query must guarantee that the actual latest row (MAX(t)) is included
    // in the output, even when rn=1 picks the earliest per bucket.
    // Without this, the chart rightmost point lags behind the true latest.
    expect(sql).toMatch(/UNION[\s\S]*MAX\(t\)/);
  });

  it("downsampled query with tMin filter includes MAX(t) boundary", () => {
    const sql = buildDerivedQuery(testSchema, 500, 100);
    expect(sql).toContain("WHERE t >= 500");
    expect(sql).toMatch(/UNION[\s\S]*MAX\(t\)/);
  });

  it("no-downsampling path does not include UNION or MAX(t) boundary (regression)", () => {
    const sql = buildDerivedQuery(testSchema);
    expect(sql).not.toContain("UNION");
  });

  it("uses provided tMax in bounds CTE instead of computing MAX(t)", () => {
    const sql = buildDerivedQuery(testSchema, undefined, 100, 1000);
    // bounds CTE should use the provided tMax as t_hi, not MAX(t)
    expect(sql).toContain("1000 AS t_hi");
    // Should still compute MIN(t) and COUNT(*) from data
    expect(sql).toContain("MIN(t) AS t_lo");
    expect(sql).toContain("COUNT(*)");
  });

  it("two schemas with same tMax produce identical bucket boundaries", () => {
    const schemaA: TableSchema<TestPoint> = {
      ...testSchema,
      tableName: "orbit_SSO",
    };
    const schemaB: TableSchema<TestPoint> = {
      ...testSchema,
      tableName: "orbit_ISS",
    };
    const tMax = 2592000; // 30 days in seconds
    const sqlA = buildDerivedQuery(schemaA, undefined, 100, tMax);
    const sqlB = buildDerivedQuery(schemaB, undefined, 100, tMax);
    // Extract the bucket calculation part (everything after bucketed AS)
    // Both should use the same tMax value in the formula
    const bucketFormulaA = sqlA.replace(/orbit_SSO/g, "TABLE");
    const bucketFormulaB = sqlB.replace(/orbit_ISS/g, "TABLE");
    expect(bucketFormulaA).toBe(bucketFormulaB);
  });

  it("without tMax, bounds CTE computes MAX(t) from data", () => {
    const sql = buildDerivedQuery(testSchema, undefined, 100);
    // Default behavior: compute t_hi from data
    expect(sql).toContain("MAX(t) AS t_hi");
    expect(sql).not.toMatch(/\d+ AS t_hi/);
  });

  it("tMax is used with tMin together", () => {
    const sql = buildDerivedQuery(testSchema, 500, 100, 2000);
    expect(sql).toContain("WHERE t >= 500");
    expect(sql).toContain("2000 AS t_hi");
  });

  it("tMax has no effect without downsampling", () => {
    const sql = buildDerivedQuery(testSchema, undefined, undefined, 1000);
    // No downsampling → simple query, tMax is irrelevant
    expect(sql).not.toContain("bounds");
    expect(sql).not.toContain("bucket");
  });

  it("includes pass-through derived columns that reference base columns", () => {
    // When a base column (like 'value') should appear in chart output,
    // a pass-through derived entry { name: "value", sql: "value" } must exist.
    const schemaWithPassthrough: TableSchema<TestPoint> = {
      tableName: "passthrough_data",
      columns: [
        { name: "t", type: "DOUBLE" },
        { name: "value", type: "DOUBLE" },
        { name: "extra", type: "DOUBLE" },
      ],
      derived: [
        { name: "doubled", sql: "value * 2" },
        { name: "raw_value", sql: "value" }, // pass-through
      ],
      toRow: (p) => [p.t, p.value, p.extra],
    };
    const sql = buildDerivedQuery(schemaWithPassthrough);
    expect(sql).toContain("value AS raw_value");
    expect(sql).toContain("value * 2 AS doubled");
  });
});

// ---------------------------------------------------------------------------
// buildCompactKeepersSQL
// ---------------------------------------------------------------------------

describe("buildCompactKeepersSQL", () => {
  it("generates CREATE TEMP TABLE with NTILE bucketing", () => {
    const sql = buildCompactKeepersSQL("orbit_points", 5000, 1000);
    expect(sql).toContain("CREATE TEMP TABLE IF NOT EXISTS _compact_keepers");
    expect(sql).toContain("NTILE(1000)");
    expect(sql).toContain("WHERE t < 5000");
    expect(sql).toContain("GROUP BY bucket");
    expect(sql).toContain("MIN(t)");
  });

  it("uses the correct table name", () => {
    const sql = buildCompactKeepersSQL("custom_table", 100, 50);
    expect(sql).toContain("FROM custom_table");
  });

  it("uses the provided cutoff value", () => {
    const sql = buildCompactKeepersSQL("data", 42.5, 200);
    expect(sql).toContain("WHERE t < 42.5");
  });
});

// ---------------------------------------------------------------------------
// buildCompactDeleteSQL
// ---------------------------------------------------------------------------

describe("buildCompactDeleteSQL", () => {
  it("generates DELETE with NOT IN keepers subquery", () => {
    const sql = buildCompactDeleteSQL("orbit_points", 5000);
    expect(sql).toContain("DELETE FROM orbit_points");
    expect(sql).toContain("WHERE t < 5000");
    expect(sql).toContain("NOT IN");
    expect(sql).toContain("_compact_keepers");
  });

  it("uses the correct cutoff value", () => {
    const sql = buildCompactDeleteSQL("data", 99.9);
    expect(sql).toContain("WHERE t < 99.9");
  });
});

// ---------------------------------------------------------------------------
// buildIncrementalQuery
// ---------------------------------------------------------------------------

describe("buildIncrementalQuery", () => {
  it("generates simple SELECT with derived columns and WHERE t > tAfter", () => {
    const sql = buildIncrementalQuery(testSchema, 100.5);
    expect(sql).toBe(
      "SELECT t, value * 2 AS doubled, value + extra AS sum FROM test_data WHERE t > 100.5 ORDER BY t",
    );
  });

  it("handles schema with no derived columns (t only)", () => {
    const sql = buildIncrementalQuery(noDerivedSchema, 50);
    expect(sql).toBe("SELECT t FROM raw_data WHERE t > 50 ORDER BY t");
  });

  it("uses strict inequality (>) not >=", () => {
    const sql = buildIncrementalQuery(testSchema, 0);
    expect(sql).toContain("WHERE t > 0");
    expect(sql).not.toContain(">=");
  });

  it("does not include downsampling CTEs", () => {
    const sql = buildIncrementalQuery(testSchema, 100);
    expect(sql).not.toContain("WITH");
    expect(sql).not.toContain("ROW_NUMBER");
    expect(sql).not.toContain("NTILE");
    expect(sql).not.toContain("bucket");
  });
});
