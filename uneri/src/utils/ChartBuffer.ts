import type { ChartDataMap } from "../types.js";

/**
 * Column-oriented ring buffer for real-time chart display.
 *
 * Stores time-series data as Float64Arrays keyed by column name,
 * matching the ChartDataMap shape expected by uPlot. Supports
 * efficient append and left-trim operations.
 *
 * Used as a DuckDB bypass: during live streaming, data flows
 * directly from WebSocket → ChartBuffer → uPlot.setData().
 */
export class ChartBuffer {
  private _columns: string[];
  private _data: Map<string, Float64Array>;
  private _length = 0;
  private _capacity: number;

  /**
   * @param columns Column names (must include "t" as the time axis).
   * @param capacity Maximum number of points before oldest are dropped.
   */
  constructor(columns: string[], capacity: number) {
    if (!columns.includes("t")) {
      throw new Error('ChartBuffer: columns must include "t"');
    }
    this._columns = columns;
    this._capacity = capacity;
    this._data = new Map();
    for (const col of columns) {
      this._data.set(col, new Float64Array(capacity));
    }
  }

  get length(): number {
    return this._length;
  }

  get capacity(): number {
    return this._capacity;
  }

  get columns(): readonly string[] {
    return this._columns;
  }

  /**
   * Append a single row of values.
   * @param values Object with a value for each column name.
   */
  push(values: Record<string, number>): void {
    if (this._length >= this._capacity) {
      this._trimOldest();
    }
    const idx = this._length;
    for (const col of this._columns) {
      this._data.get(col)![idx] = values[col] ?? 0;
    }
    this._length++;
  }

  /**
   * Append multiple rows at once.
   * @param rows Array of value objects, one per row.
   */
  pushMany(rows: Record<string, number>[]): void {
    for (const row of rows) {
      this.push(row);
    }
  }

  /**
   * Get a snapshot of the current data as ChartDataMap.
   * Returns subarray views (zero-copy) into the internal buffers.
   */
  toChartData(): ChartDataMap {
    const result: Record<string, Float64Array> = {};
    for (const col of this._columns) {
      result[col] = this._data.get(col)!.subarray(0, this._length);
    }
    return result as ChartDataMap;
  }

  /**
   * Get a windowed view of data for a specific time range.
   * Returns subarray views (zero-copy) for points where tMin <= t <= tMax.
   */
  getWindow(tMin: number, tMax: number): ChartDataMap {
    const tArr = this._data.get("t")!;
    const start = this._lowerBound(tArr, tMin);
    const end = this._upperBound(tArr, tMax);
    const result: Record<string, Float64Array> = {};
    for (const col of this._columns) {
      result[col] = this._data.get(col)!.subarray(start, end);
    }
    return result as ChartDataMap;
  }

  /** Clear all data. */
  clear(): void {
    this._length = 0;
  }

  /** The latest time value, or -Infinity if empty. */
  get latestT(): number {
    if (this._length === 0) return -Infinity;
    return this._data.get("t")![this._length - 1];
  }

  /** The earliest time value, or Infinity if empty. */
  get earliestT(): number {
    if (this._length === 0) return Infinity;
    return this._data.get("t")![0];
  }

  /** Drop the oldest half of the data when capacity is reached. */
  private _trimOldest(): void {
    const keep = Math.floor(this._length / 2);
    const drop = this._length - keep;
    for (const col of this._columns) {
      const arr = this._data.get(col)!;
      arr.copyWithin(0, drop, this._length);
    }
    this._length = keep;
  }

  /** Binary search: first index where arr[i] >= value. */
  private _lowerBound(arr: Float64Array, value: number): number {
    let lo = 0;
    let hi = this._length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (arr[mid] < value) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  /** Binary search: first index where arr[i] > value. */
  private _upperBound(arr: Float64Array, value: number): number {
    let lo = 0;
    let hi = this._length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (arr[mid] <= value) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }
}
