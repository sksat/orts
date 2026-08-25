/**
 * In-memory stand-in for an `AsyncDuckDBConnection`.
 *
 * Understands just enough SQL for the store/worker code paths — transactions,
 * table creation, full-table delete, `INSERT ... VALUES`, `COUNT(*)`, `MAX(t)`
 * and the derived `SELECT` — and can be made to fail or stall a chosen
 * statement. That makes the failure and re-entrancy paths (rollback, retry,
 * a message arriving mid-transaction) testable without a real database.
 *
 * A test double: not exported from the package entry point.
 */

/** Column accessor shaped like an Arrow vector. */
interface FakeVector {
  toArray(): Float64Array;
  get(index: number): number | null;
}

interface FakeResult {
  getChildAt(index: number): FakeVector | null;
}

function vector(values: number[]): FakeVector {
  return {
    toArray: () => Float64Array.from(values),
    get: (index) => values[index] ?? null,
  };
}

/** Parse the tuples of an `INSERT INTO tbl VALUES (..),(..)` statement. */
function parseInsertedRows(sql: string): Array<Array<number | null>> {
  const valuesAt = sql.indexOf("VALUES ");
  if (valuesAt < 0) return [];
  const body = sql.slice(valuesAt + "VALUES ".length);
  const rows: Array<Array<number | null>> = [];
  for (const match of body.matchAll(/\(([^)]*)\)/g)) {
    rows.push(
      match[1].split(",").map((cell) => {
        const trimmed = cell.trim();
        return trimmed === "NULL" ? null : Number(trimmed);
      }),
    );
  }
  return rows;
}

export class FakeDuckDBConn {
  /** Every statement executed, in order (including BEGIN/COMMIT/ROLLBACK). */
  readonly queries: string[] = [];
  /** Committed table content, keyed by table name, in insertion order. */
  readonly tables = new Map<string, Array<Array<number | null>>>();
  closed = false;

  /** Snapshot taken at BEGIN, restored on ROLLBACK. */
  private snapshot: Map<string, Array<Array<number | null>>> | null = null;
  /** Statements to reject: called with the SQL and its 1-based position. */
  private failWhen: ((sql: string, index: number) => boolean) | null = null;
  /** Statements to stall: the returned promise is awaited before applying. */
  private stallWhen: ((sql: string) => boolean) | null = null;
  private stalled: Array<() => void> = [];

  /** Reject every statement matching `predicate`. */
  failOn(predicate: ((sql: string, index: number) => boolean) | null): void {
    this.failWhen = predicate;
  }

  /** Reject the `n`-th (1-based) INSERT statement from now on, once. */
  failOnNthInsert(n: number): void {
    let seen = 0;
    this.failWhen = (sql) => {
      if (!sql.startsWith("INSERT")) return false;
      seen++;
      return seen === n;
    };
  }

  /** Stall every statement matching `predicate` until `releaseStalled()`. */
  stallOn(predicate: ((sql: string) => boolean) | null): void {
    this.stallWhen = predicate;
  }

  /** Let all stalled statements proceed. */
  releaseStalled(): void {
    const waiters = this.stalled;
    this.stalled = [];
    for (const release of waiters) release();
  }

  /** Number of statements currently stalled. */
  get stalledCount(): number {
    return this.stalled.length;
  }

  /** Committed rows of a table (empty when it does not exist). */
  rowsOf(tableName: string): Array<Array<number | null>> {
    return this.tables.get(tableName) ?? [];
  }

  /** Committed `t` values (first column) of a table, in insertion order. */
  tValuesOf(tableName: string): number[] {
    return this.rowsOf(tableName).map((row) => Number(row[0]));
  }

  async close(): Promise<void> {
    this.closed = true;
  }

  async query(sql: string): Promise<FakeResult> {
    this.queries.push(sql);
    const index = this.queries.length;

    if (this.stallWhen?.(sql)) {
      await new Promise<void>((resolve) => {
        this.stalled.push(resolve);
      });
    }
    if (this.failWhen?.(sql, index)) {
      throw new Error(`FakeDuckDBConn: injected failure for: ${sql}`);
    }

    return this.apply(sql);
  }

  private apply(sql: string): FakeResult {
    if (sql.startsWith("BEGIN")) {
      this.snapshot = new Map(
        [...this.tables].map(([name, rows]) => [name, rows.map((row) => [...row])]),
      );
      return emptyResult();
    }
    if (sql.startsWith("COMMIT")) {
      this.snapshot = null;
      return emptyResult();
    }
    if (sql.startsWith("ROLLBACK")) {
      if (this.snapshot != null) {
        this.tables.clear();
        for (const [name, rows] of this.snapshot) this.tables.set(name, rows);
        this.snapshot = null;
      }
      return emptyResult();
    }

    const created = sql.match(/^CREATE (?:OR REPLACE )?TABLE (\S+)/);
    if (created) {
      this.tables.set(created[1], []);
      return emptyResult();
    }
    if (sql.startsWith("CREATE TEMP TABLE") || sql.startsWith("DROP TABLE")) {
      return emptyResult();
    }

    const deleted = sql.match(/^DELETE FROM (\S+)\s*$/);
    if (deleted) {
      this.tables.set(deleted[1], []);
      return emptyResult();
    }

    const inserted = sql.match(/^INSERT INTO (\S+) VALUES/);
    if (inserted) {
      const table = this.tables.get(inserted[1]) ?? [];
      table.push(...parseInsertedRows(sql));
      this.tables.set(inserted[1], table);
      return emptyResult();
    }

    const counted = sql.match(/^SELECT COUNT\(\*\) AS \w+ FROM (\S+)/);
    if (counted) {
      return { getChildAt: () => vector([this.rowsOf(counted[1]).length]) };
    }

    const maxed = sql.match(/^SELECT MAX\(t\) AS \w+ FROM (\S+)/);
    if (maxed) {
      const ts = this.tValuesOf(maxed[1]);
      return { getChildAt: () => vector([ts.length > 0 ? Math.max(...ts) : Number.NaN]) };
    }

    // Anything else is treated as a derived SELECT: return the t column plus
    // one placeholder column per derived expression (the expressions
    // themselves are not evaluated).
    const from = sql.match(/FROM (\S+)/);
    const ts = from ? [...this.tValuesOf(from[1])].sort((a, b) => a - b) : [];
    return {
      getChildAt: (i) => (i === 0 ? vector(ts) : vector(ts.map(() => 0))),
    };
  }
}

function emptyResult(): FakeResult {
  return { getChildAt: () => null };
}
