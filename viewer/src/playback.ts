import { OrbitPoint } from "./orbit.js";

/** Playback state snapshot. */
export interface PlaybackState {
  points: OrbitPoint[];
  currentIndex: number;
  isPlaying: boolean;
  speed: number;
  elapsedTime: number;
}

/**
 * Linearly interpolate between two OrbitPoints at the given fraction (0..1)
 * between them.
 */
export function lerpPoint(a: OrbitPoint, b: OrbitPoint, frac: number): OrbitPoint {
  const inv = 1 - frac;
  return {
    t: a.t * inv + b.t * frac,
    x: a.x * inv + b.x * frac,
    y: a.y * inv + b.y * frac,
    z: a.z * inv + b.z * frac,
    vx: a.vx * inv + b.vx * frac,
    vy: a.vy * inv + b.vy * frac,
    vz: a.vz * inv + b.vz * frac,
    a: a.a * inv + b.a * frac,
    e: a.e * inv + b.e * frac,
    inc: a.inc * inv + b.inc * frac,
    raan: a.raan * inv + b.raan * frac,
    omega: a.omega * inv + b.omega * frac,
    nu: a.nu * inv + b.nu * frac,
  };
}

/**
 * Controller for time-based orbit data playback.
 *
 * Manages play/pause state, speed multiplier, seeking, and interpolation
 * between discrete data points for smooth animation.
 */
export class PlaybackController {
  private points: OrbitPoint[];
  private _isPlaying: boolean = false;
  private _speed: number = 1;
  private _elapsedTime: number = 0;

  /** Total simulation time span covered by the loaded data. */
  readonly totalDuration: number;

  /** Simulation start time (first point's t value). */
  readonly startTime: number;

  /** Simulation end time (last point's t value). */
  readonly endTime: number;

  /** Callback invoked on every state change (play/pause/seek/update). */
  onChange: (() => void) | null = null;

  constructor(points: OrbitPoint[]) {
    if (points.length === 0) {
      throw new Error("PlaybackController requires at least one orbit point");
    }
    this.points = points;
    this.startTime = points[0].t;
    this.endTime = points[points.length - 1].t;
    this.totalDuration = this.endTime - this.startTime;
    this._elapsedTime = 0;
  }

  // --- Getters ---

  get isPlaying(): boolean {
    return this._isPlaying;
  }

  get speed(): number {
    return this._speed;
  }

  /** Elapsed simulation time since the start of the data. */
  get elapsedTime(): number {
    return this._elapsedTime;
  }

  /** Current absolute simulation time. */
  get currentTime(): number {
    return this.startTime + this._elapsedTime;
  }

  /** Current position as a fraction 0..1 through the data. */
  get fraction(): number {
    if (this.totalDuration <= 0) return 1;
    return Math.min(1, Math.max(0, this._elapsedTime / this.totalDuration));
  }

  // --- Playback controls ---

  play(): void {
    this._isPlaying = true;
    this.emitChange();
  }

  pause(): void {
    this._isPlaying = false;
    this.emitChange();
  }

  togglePlayPause(): void {
    if (this._isPlaying) {
      this.pause();
    } else {
      // If at end, restart from beginning
      if (this._elapsedTime >= this.totalDuration) {
        this._elapsedTime = 0;
      }
      this.play();
    }
  }

  setSpeed(speed: number): void {
    this._speed = Math.max(0.1, speed);
    this.emitChange();
  }

  /**
   * Seek to a specific absolute simulation time.
   * The time is clamped to the data range.
   */
  seekToTime(t: number): void {
    const clamped = Math.min(this.endTime, Math.max(this.startTime, t));
    this._elapsedTime = clamped - this.startTime;
    this.emitChange();
  }

  /**
   * Seek to a fractional position (0.0 = start, 1.0 = end).
   */
  seekToFraction(fraction: number): void {
    const f = Math.min(1, Math.max(0, fraction));
    this._elapsedTime = f * this.totalDuration;
    this.emitChange();
  }

  /**
   * Advance playback by real-world elapsed time (in seconds).
   * The speed multiplier is applied so that `realDt * speed` of simulation
   * time passes.
   *
   * @returns true if the state changed (i.e. playback is active)
   */
  update(realDt: number): boolean {
    if (!this._isPlaying) return false;

    this._elapsedTime += realDt * this._speed;

    // Clamp at end and auto-pause
    if (this._elapsedTime >= this.totalDuration) {
      this._elapsedTime = this.totalDuration;
      this._isPlaying = false;
    }

    this.emitChange();
    return true;
  }

  /**
   * Return the interpolated orbit state at the current playback time.
   */
  getCurrentState(): OrbitPoint {
    return this.getStateAtTime(this.currentTime);
  }

  /**
   * Return the index of the last data point whose time is <= currentTime.
   * Useful for determining how much of the trail to draw.
   */
  getCurrentTrailIndex(): number {
    const t = this.currentTime;

    // Binary search for the last point with time <= t
    let lo = 0;
    let hi = this.points.length - 1;

    if (t <= this.points[lo].t) return 0;
    if (t >= this.points[hi].t) return hi;

    while (hi - lo > 1) {
      const mid = (lo + hi) >>> 1;
      if (this.points[mid].t <= t) {
        lo = mid;
      } else {
        hi = mid;
      }
    }

    return lo;
  }

  /** Get a snapshot of the current playback state. */
  getState(): PlaybackState {
    return {
      points: this.points,
      currentIndex: this.getCurrentTrailIndex(),
      isPlaying: this._isPlaying,
      speed: this._speed,
      elapsedTime: this._elapsedTime,
    };
  }

  // --- Internal ---

  private getStateAtTime(absTime: number): OrbitPoint {
    const pts = this.points;

    // Edge cases
    if (absTime <= pts[0].t) return { ...pts[0] };
    if (absTime >= pts[pts.length - 1].t) return { ...pts[pts.length - 1] };

    // Binary search for the bracketing interval
    let lo = 0;
    let hi = pts.length - 1;
    while (hi - lo > 1) {
      const mid = (lo + hi) >>> 1;
      if (pts[mid].t <= absTime) {
        lo = mid;
      } else {
        hi = mid;
      }
    }

    // Interpolate within [lo, hi]
    const dt = pts[hi].t - pts[lo].t;
    const frac = dt > 0 ? (absTime - pts[lo].t) / dt : 0;

    return lerpPoint(pts[lo], pts[hi], frac);
  }

  private emitChange(): void {
    if (this.onChange) {
      this.onChange();
    }
  }
}
