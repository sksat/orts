import { describe, expect, it } from "vitest";
import type { OrbitPoint } from "../orbit.js";
import { TrailBuffer } from "../utils/TrailBuffer.js";
import {
  computeLiveSyncTime,
  computePlaybackTimeline,
  computeTrailDrawStarts,
} from "./useRealtimePlayback.js";

function makePoint(t: number, entityPath?: string): OrbitPoint {
  return {
    t,
    entityPath,
    x: 6778 + t,
    y: t * 0.1,
    z: 0,
    vx: 0,
    vy: 7.669,
    vz: 0,
    a: 6778,
    e: 0,
    inc: 0.9,
    raan: 0,
    omega: 0,
    nu: 0,
  };
}

describe("computeLiveSyncTime", () => {
  it("returns min of all satellites when none terminated", () => {
    const buffers = new Map<string, TrailBuffer>();
    const bufA = new TrailBuffer(1000);
    const bufB = new TrailBuffer(1000);

    for (let t = 0; t <= 100; t += 10) bufA.push(makePoint(t, "sat-a"));
    for (let t = 0; t <= 200; t += 10) bufB.push(makePoint(t, "sat-b"));

    buffers.set("sat-a", bufA);
    buffers.set("sat-b", bufB);

    const syncTime = computeLiveSyncTime(buffers, new Set());
    expect(syncTime).toBe(100);
  });

  it("excludes terminated satellite from sync time", () => {
    const buffers = new Map<string, TrailBuffer>();
    const bufA = new TrailBuffer(1000);
    const bufB = new TrailBuffer(1000);

    // sat-a stopped at t=100, sat-b continued to t=200
    for (let t = 0; t <= 100; t += 10) bufA.push(makePoint(t, "sat-a"));
    for (let t = 0; t <= 200; t += 10) bufB.push(makePoint(t, "sat-b"));

    buffers.set("sat-a", bufA);
    buffers.set("sat-b", bufB);

    const terminated = new Set(["sat-a"]);
    const syncTime = computeLiveSyncTime(buffers, terminated);
    expect(syncTime).toBe(200);
  });

  it("returns Infinity when all satellites are terminated", () => {
    const buffers = new Map<string, TrailBuffer>();
    const bufA = new TrailBuffer(1000);
    bufA.push(makePoint(100, "sat-a"));
    buffers.set("sat-a", bufA);

    const terminated = new Set(["sat-a"]);
    const syncTime = computeLiveSyncTime(buffers, terminated);
    expect(syncTime).toBe(Infinity);
  });

  it("returns Infinity for empty buffers", () => {
    const buffers = new Map<string, TrailBuffer>();
    const syncTime = computeLiveSyncTime(buffers, new Set());
    expect(syncTime).toBe(Infinity);
  });

  it("skips buffers with no data", () => {
    const buffers = new Map<string, TrailBuffer>();
    const bufA = new TrailBuffer(1000);
    const bufB = new TrailBuffer(1000);

    bufA.push(makePoint(50, "sat-a"));
    // bufB is empty

    buffers.set("sat-a", bufA);
    buffers.set("sat-b", bufB);

    const syncTime = computeLiveSyncTime(buffers, new Set());
    expect(syncTime).toBe(50);
  });
});

describe("computeTrailDrawStarts", () => {
  it("returns all zeros when timeRange is null", () => {
    const buffers = new Map<string, TrailBuffer>();
    const buf = new TrailBuffer(1000);
    for (let t = 0; t <= 100; t += 10) buf.push(makePoint(t));
    buffers.set("sat-a", buf);

    const starts = computeTrailDrawStarts(buffers, 100, null);
    expect(starts.get("sat-a")).toBe(0);
  });

  it("returns 0 when timeRange covers entire buffer", () => {
    const buffers = new Map<string, TrailBuffer>();
    const buf = new TrailBuffer(1000);
    for (let t = 0; t <= 100; t += 10) buf.push(makePoint(t));
    buffers.set("sat-a", buf);

    // timeRange=200 > total duration 100
    const starts = computeTrailDrawStarts(buffers, 100, 200);
    expect(starts.get("sat-a")).toBe(0);
  });

  it("clips start for timeRange shorter than buffer duration", () => {
    const buffers = new Map<string, TrailBuffer>();
    const buf = new TrailBuffer(1000);
    // Points at t=0,10,20,...,100
    for (let t = 0; t <= 100; t += 10) buf.push(makePoint(t));
    buffers.set("sat-a", buf);

    // currentTime=100, timeRange=30 → startT=70 → indexBefore(70)=7
    const starts = computeTrailDrawStarts(buffers, 100, 30);
    expect(starts.get("sat-a")).toBe(7); // point at t=70
  });

  it("clips start when paused in the middle", () => {
    const buffers = new Map<string, TrailBuffer>();
    const buf = new TrailBuffer(1000);
    for (let t = 0; t <= 100; t += 10) buf.push(makePoint(t));
    buffers.set("sat-a", buf);

    // Paused at currentTime=50, timeRange=20 → startT=30 → indexBefore(30)=3
    const starts = computeTrailDrawStarts(buffers, 50, 20);
    expect(starts.get("sat-a")).toBe(3); // point at t=30
  });

  it("handles multiple satellites independently", () => {
    const buffers = new Map<string, TrailBuffer>();
    const bufA = new TrailBuffer(1000);
    const bufB = new TrailBuffer(1000);

    // sat-a: t=0,10,...,100
    for (let t = 0; t <= 100; t += 10) bufA.push(makePoint(t, "sat-a"));
    // sat-b: t=50,60,...,100
    for (let t = 50; t <= 100; t += 10) bufB.push(makePoint(t, "sat-b"));

    buffers.set("sat-a", bufA);
    buffers.set("sat-b", bufB);

    // currentTime=100, timeRange=30 → startT=70
    const starts = computeTrailDrawStarts(buffers, 100, 30);
    // sat-a: indexBefore(70)=7 (t=70)
    expect(starts.get("sat-a")).toBe(7);
    // sat-b: has [50,60,70,80,90,100], indexBefore(70)=2 (t=70)
    expect(starts.get("sat-b")).toBe(2);
  });

  it("returns 0 for empty buffers", () => {
    const buffers = new Map<string, TrailBuffer>();
    const buf = new TrailBuffer(1000);
    buffers.set("sat-a", buf);

    const starts = computeTrailDrawStarts(buffers, 100, 30);
    expect(starts.get("sat-a")).toBe(0);
  });

  it("returns 0 when startT is before all points", () => {
    const buffers = new Map<string, TrailBuffer>();
    const buf = new TrailBuffer(1000);
    for (let t = 50; t <= 100; t += 10) buf.push(makePoint(t));
    buffers.set("sat-a", buf);

    // currentTime=60, timeRange=30 → startT=30, which is before t=50
    const starts = computeTrailDrawStarts(buffers, 60, 30);
    expect(starts.get("sat-a")).toBe(0);
  });
});

describe("computePlaybackTimeline", () => {
  it("spans the whole received history while Live", () => {
    // Live has no frozen end: the slider follows the newest state.
    const tl = computePlaybackTimeline(0, 200, 200, null);
    expect(tl).toEqual({ start: 0, end: 200, fraction: 1 });
  });

  it("keeps a paused thumb where it is while newer states arrive", () => {
    // Paused at t = 100 out of [0, 100]. The server runs on to t = 200; the
    // slider used to measure against that growing end, so the thumb slid
    // back to the middle without anyone touching it.
    const frozenEnd = 100;
    for (const tMax of [100, 150, 200]) {
      const tl = computePlaybackTimeline(0, tMax, 100, frozenEnd);
      expect(tl.end).toBe(100);
      expect(tl.fraction).toBe(1);
    }
  });

  it("never moves a playing thumb backwards while the server outruns it", () => {
    // dt = 0.1 s runs about 9 sim s per wall s; replay at 1x advances the
    // view 1 sim s per wall s. Measured against the growing end, the thumb
    // fell back on every step even though Play moves time forward.
    let currentTime = 50;
    let tMax = 100;
    let frozenEnd = 100;
    let previous = computePlaybackTimeline(0, tMax, currentTime, frozenEnd).fraction;
    for (let step = 0; step < 100; step++) {
      currentTime += 1;
      tMax += 9;
      frozenEnd = Math.max(frozenEnd, currentTime);
      const { fraction } = computePlaybackTimeline(0, tMax, currentTime, frozenEnd);
      expect(fraction).toBeGreaterThanOrEqual(previous);
      previous = fraction;
    }
  });

  it("stretches a frozen end that playback has passed, up to the newest state", () => {
    expect(computePlaybackTimeline(0, 300, 250, 200).end).toBe(250);
    // A frozen end beyond what the buffers still hold (rebuilt from a range
    // response) is clamped to what is there.
    expect(computePlaybackTimeline(0, 150, 120, 200).end).toBe(150);
  });

  it("lets go of a frozen end the trail buffer has dropped", () => {
    // Paused at t = 100 while the buffer keeps filling: once it drops its old
    // points, the oldest one kept (2500) is past the frozen end. Kept as it
    // was, the span ran backwards (start 2500, end 100), and dragging the
    // thumb right moved the view back into points no longer there.
    const tl = computePlaybackTimeline(2500, 10000, 100, 100);
    expect(tl.end).toBeGreaterThanOrEqual(tl.start);
    expect(tl).toEqual({ start: 2500, end: 10000, fraction: 0 });
  });

  it("puts the thumb at the end of an empty span", () => {
    expect(computePlaybackTimeline(10, 10, 10, null).fraction).toBe(1);
  });
});
