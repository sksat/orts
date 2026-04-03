import { lerpPoint, type OrbitPoint } from "../orbit.js";

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
    return this.points.length > 0 ? this.points[this.points.length - 1] : null;
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

  /**
   * Interpolate the orbit state at an arbitrary time value.
   * Uses binary search + linear interpolation between bracketing points.
   * Returns null if the buffer is empty.
   * Clamps to first/last point if t is outside the data range.
   */
  interpolateAt(t: number): OrbitPoint | null {
    const pts = this.points;
    if (pts.length === 0) return null;
    if (pts.length === 1 || t <= pts[0].t) return { ...pts[0] };
    if (t >= pts[pts.length - 1].t) return { ...pts[pts.length - 1] };

    // Binary search for the bracketing interval
    let lo = 0;
    let hi = pts.length - 1;
    while (hi - lo > 1) {
      const mid = (lo + hi) >>> 1;
      if (pts[mid].t <= t) {
        lo = mid;
      } else {
        hi = mid;
      }
    }

    const dt = pts[hi].t - pts[lo].t;
    const frac = dt > 0 ? (t - pts[lo].t) / dt : 0;
    return lerpPoint(pts[lo], pts[hi], frac);
  }

  /**
   * Return the index of the last point whose time is <= t.
   * Returns -1 if the buffer is empty or all points are after t.
   */
  indexBefore(t: number): number {
    const pts = this.points;
    if (pts.length === 0) return -1;
    if (t < pts[0].t) return -1;
    if (t >= pts[pts.length - 1].t) return pts.length - 1;

    let lo = 0;
    let hi = pts.length - 1;
    while (hi - lo > 1) {
      const mid = (lo + hi) >>> 1;
      if (pts[mid].t <= t) {
        lo = mid;
      } else {
        hi = mid;
      }
    }
    return lo;
  }

  private trimIfNeeded(): void {
    if (this.points.length > this.capacity * 1.5) {
      this.points = this.points.slice(-this.capacity);
      this._generation++;
    }
  }
}
