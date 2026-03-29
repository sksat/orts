import type { TimePoint } from "../types.js";

/**
 * Staging buffer for DuckDB ingestion.
 *
 * Accumulates incoming data points and provides them in batches
 * via `drain()`. The drain pattern decouples data arrival from
 * DuckDB insertion timing (polling interval), avoiding the need
 * for index-based tracking.
 *
 * **Contract**: `t` values pushed to this buffer MUST be strictly
 * monotonically increasing. Out-of-order data will be silently missed
 * by incremental queries. This is guaranteed by the orts simulator
 * (simulation time always advances). Consumers using uneri with other
 * data sources must enforce this property.
 */
export class IngestBuffer<T extends TimePoint = TimePoint> {
  private pending: T[] = [];
  private _latestT = -Infinity;
  private _rebuildData: T[] | null = null;

  /** Push a single point. Must satisfy t > latestT (strictly increasing). */
  push(point: T): void {
    if (
      typeof process !== "undefined" &&
      process.env.NODE_ENV !== "production" &&
      this._latestT !== -Infinity &&
      point.t <= this._latestT
    ) {
      console.warn(
        `IngestBuffer: t=${point.t} is not strictly increasing (latestT=${this._latestT})`,
      );
    }
    this.pending.push(point);
    if (point.t > this._latestT) {
      this._latestT = point.t;
    }
  }

  /** Push multiple points at once (appended to end). */
  pushMany(points: T[]): void {
    for (const p of points) {
      this.pending.push(p);
      if (p.t > this._latestT) {
        this._latestT = p.t;
      }
    }
  }

  /**
   * Prepend points to the front of the buffer.
   * Used for re-queuing failed insert batches: since new points may have
   * arrived in `pending` during the async insert, appending the failed
   * batch would break t-monotonicity. Prepending preserves order.
   */
  prependMany(points: T[]): void {
    this.pending.unshift(...points);
  }

  /**
   * Drain all pending points, returning them and clearing the buffer.
   * Returns an empty array if nothing is pending.
   */
  drain(): T[] {
    if (this.pending.length === 0) return [];
    const result = this.pending;
    this.pending = [];
    return result;
  }

  /**
   * Signal a full table rebuild. The tick loop should clear the DuckDB table
   * and insert the returned data from `consumeRebuild()`.
   *
   * Clears any stale pending points to prevent duplicates (fullData is
   * the complete replacement dataset). Points pushed after this call
   * are treated as genuinely new and will be included by consumeRebuild().
   */
  markRebuild(fullData: T[]): void {
    this._rebuildData = fullData;
    this.pending = [];
    for (const p of fullData) {
      if (p.t > this._latestT) {
        this._latestT = p.t;
      }
    }
  }

  /**
   * Consume a pending rebuild signal. Returns the rebuild data merged
   * with any points pushed since markRebuild(), or null if no rebuild
   * is pending. The rebuild flag and pending buffer are both cleared.
   */
  consumeRebuild(): T[] | null {
    if (this._rebuildData === null) return null;
    const result = [...this._rebuildData, ...this.pending];
    this._rebuildData = null;
    this.pending = [];
    return result;
  }

  /** Number of points waiting to be drained. */
  get pendingCount(): number {
    return this.pending.length;
  }

  /**
   * The latest t value seen across all pushed points.
   * Used for timeRange calculation in chart components.
   * Returns -Infinity if no points have been pushed.
   */
  get latestT(): number {
    return this._latestT;
  }
}
