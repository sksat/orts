import type { OrbitPoint } from "../orbit.js";

/**
 * Staging buffer for DuckDB ingestion.
 *
 * Accumulates incoming orbit points and provides them in batches
 * via `drain()`. The drain pattern decouples WebSocket message
 * arrival from DuckDB insertion timing (500ms polling), avoiding
 * the need for index-based tracking.
 */
export class IngestBuffer {
  private pending: OrbitPoint[] = [];
  private _latestT = -Infinity;

  /** Push a single point. */
  push(point: OrbitPoint): void {
    this.pending.push(point);
    if (point.t > this._latestT) {
      this._latestT = point.t;
    }
  }

  /** Push multiple points at once. */
  pushMany(points: OrbitPoint[]): void {
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
  drain(): OrbitPoint[] {
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
   * Used by useOrbitCharts for timeRange tMin calculation.
   * Returns -Infinity if no points have been pushed.
   */
  get latestT(): number {
    return this._latestT;
  }
}
