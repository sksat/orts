import type { TimePoint } from "../types.js";

/**
 * Staging buffer for DuckDB ingestion.
 *
 * Accumulates incoming data points and provides them in batches
 * via `drain()`. The drain pattern decouples data arrival from
 * DuckDB insertion timing (polling interval), avoiding the need
 * for index-based tracking.
 */
export class IngestBuffer<T extends TimePoint = TimePoint> {
  private pending: T[] = [];
  private _latestT = -Infinity;

  /** Push a single point. */
  push(point: T): void {
    this.pending.push(point);
    if (point.t > this._latestT) {
      this._latestT = point.t;
    }
  }

  /** Push multiple points at once. */
  pushMany(points: T[]): void {
    for (const p of points) {
      this.pending.push(p);
      if (p.t > this._latestT) {
        this._latestT = p.t;
      }
    }
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
