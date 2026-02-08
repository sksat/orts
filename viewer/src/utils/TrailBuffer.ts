import type { OrbitPoint } from "../orbit.js";

/**
 * Bounded buffer for orbit trail rendering.
 *
 * Keeps at most `capacity` points. When the buffer grows beyond
 * `capacity * 1.5`, the oldest points are trimmed and `generation`
 * is incremented so that consumers (e.g. OrbitTrail GPU buffer)
 * know to do a full rewrite instead of an incremental append.
 */
export class TrailBuffer {
  private points: OrbitPoint[] = [];
  private _generation = 0;

  constructor(public readonly capacity: number) {}

  /** Push a single point. Trims if over capacity threshold. */
  push(point: OrbitPoint): void {
    this.points.push(point);
    this.trimIfNeeded();
  }

  /** Push multiple points at once. Trims once at the end. */
  pushMany(points: OrbitPoint[]): void {
    for (const p of points) {
      this.points.push(p);
    }
    this.trimIfNeeded();
  }

  get length(): number {
    return this.points.length;
  }

  /**
   * Generation counter. Incremented on trim or clear.
   * OrbitTrail uses this to detect when a full GPU buffer rewrite is needed
   * (as opposed to incremental append).
   */
  get generation(): number {
    return this._generation;
  }

  /** The most recently pushed point, or null if empty. */
  get latest(): OrbitPoint | null {
    return this.points.length > 0
      ? this.points[this.points.length - 1]
      : null;
  }

  /**
   * Returns the internal array reference (no copy).
   * Callers must not mutate the returned array.
   */
  getAll(): OrbitPoint[] {
    return this.points;
  }

  /** Clear all points and increment generation. */
  clear(): void {
    this.points = [];
    this._generation++;
  }

  private trimIfNeeded(): void {
    if (this.points.length > this.capacity * 1.5) {
      this.points = this.points.slice(-this.capacity);
      this._generation++;
    }
  }
}
