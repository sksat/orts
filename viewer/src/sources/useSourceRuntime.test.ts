import { beforeEach, describe, expect, it } from "vitest";
import type { OrbitPoint } from "../orbit.js";
import { TrailBuffer } from "../utils/TrailBuffer.js";
import {
  type ChartBufferLike,
  createEventDispatcher,
  type IngestBufferLike,
  isDataBumpEvent,
  type RuntimeBuffers,
  type RuntimeState,
  setIngestBufferFactory,
  setTrailBufferFactory,
} from "./eventDispatcher.js";
import type { SimInfo, SourceEvent } from "./types.js";

/** Minimal ChartBuffer stub. No Worker dependency. */
class ChartBufferStub implements ChartBufferLike {
  pushCount = 0;
  cleared = false;
  push(_values: Record<string, number>): void {
    this.pushCount++;
  }
  clear(): void {
    this.cleared = true;
    this.pushCount = 0;
  }
}

/** Minimal IngestBuffer stub. No uneri/Worker dependency. */
class IngestBufferStub implements IngestBufferLike<OrbitPoint> {
  private _points: OrbitPoint[] = [];
  private _latestT = -Infinity;
  private _rebuildData: OrbitPoint[] | null = null;

  push(point: OrbitPoint): void {
    this._points.push(point);
    if (point.t > this._latestT) this._latestT = point.t;
  }

  markRebuild(points: OrbitPoint[]): void {
    this._rebuildData = points;
    if (points.length > 0) {
      this._latestT = Math.max(...points.map((p) => p.t));
    }
  }

  get latestT(): number {
    return this._latestT;
  }

  get points(): OrbitPoint[] {
    return this._points;
  }
}

/** Minimal OrbitPoint for testing. */
function makePoint(t: number, entityPath = "default"): OrbitPoint {
  return {
    entityPath,
    t,
    x: t * 100,
    y: 0,
    z: 0,
    vx: 0,
    vy: 7.5,
    vz: 0,
    a: 7000,
    e: 0,
    inc: 0,
    raan: 0,
    omega: 0,
    nu: 0,
  };
}

function makeSimInfo(overrides: Partial<SimInfo> = {}): SimInfo {
  return {
    mu: 398600.4418,
    dt: 10,
    output_interval: 10,
    stream_interval: 10,
    central_body: "earth",
    central_body_radius: 6378.137,
    epoch_jd: 2451545.0,
    satellites: [{ id: "sat1", name: "Test", altitude: 400, period: 5400, perturbations: [] }],
    ...overrides,
  };
}

// Set up factories before tests
beforeEach(() => {
  setTrailBufferFactory(() => new TrailBuffer(50000));
  setIngestBufferFactory(() => new IngestBufferStub());
});

function createTestBuffers(): RuntimeBuffers {
  return {
    trailBuffers: new Map<string, TrailBuffer>(),
    ingestBuffers: new Map<
      string,
      IngestBufferLike<OrbitPoint>
    >() as RuntimeBuffers["ingestBuffers"],
    chartBuffer: new ChartBufferStub(),
    streamingCount: 0,
    chunkLoadStarted: false,
  };
}

function createTestState(): RuntimeState {
  return {
    simInfo: null,
    serverState: "unknown",
    terminatedSatellites: new Set<string>(),
    connectionState: "disconnected",
    textureRevision: 0,
  };
}

describe("createEventDispatcher", () => {
  it("info event sets simInfo and serverState", () => {
    const buffers = createTestBuffers();
    const state = createTestState();
    const dispatch = createEventDispatcher(buffers, state, "ws-0");

    dispatch("ws-0", { kind: "info", info: makeSimInfo() });

    expect(state.simInfo).not.toBeNull();
    expect(state.simInfo!.mu).toBe(398600.4418);
    expect(state.serverState).toBe("running");
    expect(state.connectionState).toBe("connected");
  });

  it("state event pushes to TrailBuffer and IngestBuffer", () => {
    const buffers = createTestBuffers();
    const state = createTestState();
    const dispatch = createEventDispatcher(buffers, state, "ws-0");

    dispatch("ws-0", { kind: "state", point: makePoint(10, "sat1") });

    expect(buffers.trailBuffers.get("sat1")?.length).toBe(1);
    expect(buffers.ingestBuffers.get("sat1")?.latestT).toBe(10);
    expect(buffers.streamingCount).toBe(1);
  });

  it("history event pushes to TrailBuffer and marks IngestBuffer rebuild", () => {
    const buffers = createTestBuffers();
    const state = createTestState();
    const dispatch = createEventDispatcher(buffers, state, "ws-0");

    const points = [makePoint(0, "sat1"), makePoint(10, "sat1"), makePoint(20, "sat1")];
    dispatch("ws-0", { kind: "history", points });

    expect(buffers.trailBuffers.get("sat1")?.length).toBe(3);
    expect(buffers.ingestBuffers.get("sat1")?.latestT).toBe(20);
    expect(buffers.streamingCount).toBe(0); // reset after history
  });

  it("history-chunk accumulates points, markRebuild on done", () => {
    const buffers = createTestBuffers();
    const state = createTestState();
    const dispatch = createEventDispatcher(buffers, state, "ws-0");

    const chunk1 = [makePoint(0, "sat1"), makePoint(10, "sat1")];
    const chunk2 = [makePoint(20, "sat1")];
    dispatch("ws-0", { kind: "history-chunk", points: chunk1, done: false });
    expect(buffers.trailBuffers.get("sat1")?.length).toBe(2);

    dispatch("ws-0", { kind: "history-chunk", points: chunk2, done: true });
    expect(buffers.trailBuffers.get("sat1")?.length).toBe(3);
    // After done, IngestBuffer should have rebuild data
    expect(buffers.ingestBuffers.get("sat1")?.latestT).toBe(20);
  });

  it("terminated event adds to set", () => {
    const buffers = createTestBuffers();
    const state = createTestState();
    const dispatch = createEventDispatcher(buffers, state, "ws-0");

    dispatch("ws-0", { kind: "terminated", entityPath: "sat1", t: 100, reason: "impact" });
    expect(state.terminatedSatellites.has("sat1")).toBe(true);
  });

  it("server-state event updates state", () => {
    const buffers = createTestBuffers();
    const state = createTestState();
    const dispatch = createEventDispatcher(buffers, state, "ws-0");

    dispatch("ws-0", { kind: "server-state", state: "paused" });
    expect(state.serverState).toBe("paused");

    dispatch("ws-0", { kind: "server-state", state: "idle" });
    expect(state.serverState).toBe("idle");
    expect(state.simInfo).toBeNull();
  });

  it("textures-ready bumps revision", () => {
    const buffers = createTestBuffers();
    const state = createTestState();
    const dispatch = createEventDispatcher(buffers, state, "ws-0");

    expect(state.textureRevision).toBe(0);
    dispatch("ws-0", { kind: "textures-ready", body: "earth" });
    expect(state.textureRevision).toBe(1);
  });

  it("complete event sets connectionState", () => {
    const buffers = createTestBuffers();
    const state = createTestState();
    const dispatch = createEventDispatcher(buffers, state, "ws-0");

    dispatch("ws-0", { kind: "complete" });
    expect(state.connectionState).toBe("complete");
  });

  it("error event sets connectionState to error", () => {
    const buffers = createTestBuffers();
    const state = createTestState();
    const dispatch = createEventDispatcher(buffers, state, "ws-0");

    dispatch("ws-0", { kind: "error", message: "connection lost" });
    expect(state.connectionState).toBe("error");
  });

  it("ignores events from non-active sourceId (stale event discard)", () => {
    const buffers = createTestBuffers();
    const state = createTestState();
    const dispatch = createEventDispatcher(buffers, state, "ws-0");

    dispatch("ws-old", { kind: "info", info: makeSimInfo() });
    expect(state.simInfo).toBeNull(); // ignored
  });

  it("multi-satellite history groups by entityPath", () => {
    const buffers = createTestBuffers();
    const state = createTestState();
    const dispatch = createEventDispatcher(buffers, state, "ws-0");

    const points = [
      makePoint(0, "sat1"),
      makePoint(0, "sat2"),
      makePoint(10, "sat1"),
      makePoint(10, "sat2"),
    ];
    dispatch("ws-0", { kind: "history", points });

    expect(buffers.trailBuffers.get("sat1")?.length).toBe(2);
    expect(buffers.trailBuffers.get("sat2")?.length).toBe(2);
  });

  it("range-response updates chartBuffer so live chart reflects enriched data (regression: I-A)", () => {
    // Before the I-A fix, range-response only wrote to trailBuffers and
    // ingestBuffers, leaving chartBuffer untouched. The chartBufferVersion
    // bump (I3 fix) then re-ran the live chart memo, which re-read the
    // same stale chartBuffer — a silent no-op.
    //
    // After the fix, range-response clear-and-rebuilds chartBuffer from
    // the (pre-merged) response points so the live-mode chart path sees
    // the enriched data on the next re-render.
    const buffers = createTestBuffers();
    const state = createTestState();
    const dispatch = createEventDispatcher(buffers, state, "ws-0");

    // Seed chartBuffer with sparse overview-equivalent points first.
    dispatch("ws-0", {
      kind: "history",
      points: [makePoint(0, "sat1"), makePoint(100, "sat1"), makePoint(200, "sat1")],
    });
    const chartBuf = buffers.chartBuffer as ChartBufferStub;
    const seededPushCount = chartBuf.pushCount;
    expect(seededPushCount).toBe(3);

    // A denser range-response for the same window. useWebSocketSource
    // pre-merges these with any recent streaming tail before dispatching.
    const dense = [
      makePoint(0, "sat1"),
      makePoint(50, "sat1"),
      makePoint(100, "sat1"),
      makePoint(150, "sat1"),
      makePoint(200, "sat1"),
    ];
    dispatch("ws-0", { kind: "range-response", tMin: 0, tMax: 200, points: dense });

    // chartBuffer must have been cleared and re-populated with the dense
    // response. Before the fix, seededPushCount would remain unchanged.
    expect(chartBuf.pushCount).toBe(dense.length);

    // Trail buffer sanity: the range-response path has always updated it.
    expect(buffers.trailBuffers.get("sat1")?.length).toBe(dense.length);
  });
});

describe("isDataBumpEvent", () => {
  // Events that modify trail/ingest/chart buffers must trigger a chart
  // re-render, otherwise data arrives silently in refs and the UI goes
  // stale. Notably `range-response` — the payload of both the proactive
  // initial query_range and user chart-zoom requests — must be included.

  it("returns true for state events", () => {
    const event: SourceEvent = {
      kind: "state",
      point: {
        t: 0,
        x: 0,
        y: 0,
        z: 0,
        vx: 0,
        vy: 0,
        vz: 0,
        a: 0,
        e: 0,
        inc: 0,
        raan: 0,
        omega: 0,
        nu: 0,
      },
    };
    expect(isDataBumpEvent(event)).toBe(true);
  });

  it("returns true for history events", () => {
    expect(isDataBumpEvent({ kind: "history", points: [] })).toBe(true);
  });

  it("returns true for history-chunk events", () => {
    expect(isDataBumpEvent({ kind: "history-chunk", points: [], done: false })).toBe(true);
  });

  it("returns true for range-response events (regression: I3)", () => {
    // The initial proactive query_range response and user chart-zoom
    // responses both arrive as range-response events. Without this, the
    // UI would not re-render after receiving enriched historical data.
    expect(isDataBumpEvent({ kind: "range-response", tMin: 0, tMax: 100, points: [] })).toBe(true);
  });

  it("returns false for info events", () => {
    const info: SimInfo = {
      mu: 398600,
      dt: 10,
      output_interval: 10,
      stream_interval: 10,
      central_body: "earth",
      central_body_radius: 6378,
      epoch_jd: null,
      satellites: [],
    };
    expect(isDataBumpEvent({ kind: "info", info })).toBe(false);
  });

  it("returns false for terminated events", () => {
    expect(
      isDataBumpEvent({ kind: "terminated", entityPath: "/sat/a", t: 0, reason: "test" }),
    ).toBe(false);
  });

  it("returns false for server-state events", () => {
    expect(isDataBumpEvent({ kind: "server-state", state: "paused" })).toBe(false);
  });

  it("returns false for textures-ready events", () => {
    expect(isDataBumpEvent({ kind: "textures-ready", body: "earth" })).toBe(false);
  });
});
