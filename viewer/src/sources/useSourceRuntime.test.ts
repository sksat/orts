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
  private _pending: OrbitPoint[] = [];
  private _latestT = -Infinity;
  private _rebuildData: OrbitPoint[] | null = null;

  push(point: OrbitPoint): void {
    this._points.push(point);
    this._pending.push(point);
    if (point.t > this._latestT) this._latestT = point.t;
  }

  /** As the real one does: the array is retained, not copied, and `latestT`
   * comes from it alone (an empty replacement resets it). */
  markRebuild(points: OrbitPoint[]): void {
    this._rebuildData = points;
    this._pending = [];
    this._latestT = -Infinity;
    for (const p of points) {
      if (p.t > this._latestT) this._latestT = p.t;
    }
  }

  /** As the real one does: what was pushed since the last drain or rebuild. */
  drain(): OrbitPoint[] {
    const result = this._pending;
    this._pending = [];
    return result;
  }

  /** As the real one does: the retained array plus what arrived since. */
  consumeRebuild(): OrbitPoint[] | null {
    if (this._rebuildData === null) return null;
    const result = [...this._rebuildData, ...this._pending];
    this._rebuildData = null;
    this._pending = [];
    return result;
  }

  get rebuildData(): OrbitPoint[] | null {
    return this._rebuildData;
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
    satellites: [
      { id: "sat1", name: "Test", altitude: 400, period: 5400, perturbations: [], shape: null },
    ],
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
  // Two announcements can arrive before React re-renders. The runtime hands
  // the dispatcher the snapshot it produced last, so the second merge builds
  // on the first rather than on what the connect message left.
  it("keeps both of two satellite-added events in a row", () => {
    const buffers = createTestBuffers();
    const state = createTestState();
    const dispatch = createEventDispatcher(buffers, state, "ws-0");
    dispatch("ws-0", { kind: "info", info: makeSimInfo() });

    for (const [id, model] of [
      ["sat2", "panel_srp"],
      ["sat3", "panel_drag"],
    ] as const) {
      dispatch("ws-0", {
        kind: "satellite-added",
        satellite: {
          id,
          name: id,
          altitude: 700,
          period: 5900,
          perturbations: [model],
          shape: null,
        },
        t: 10,
      });
    }

    expect(state.simInfo?.satellites.map((s) => s.id)).toEqual(["sat1", "sat2", "sat3"]);
    expect(state.simInfo?.satellites.flatMap((s) => s.perturbations)).toEqual([
      "panel_srp",
      "panel_drag",
    ]);
  });

  // A state that arrives after a file load, before the worker consumes the
  // load's rebuild, is in that rebuild once: `consumeRebuild` returns the
  // retained array and what was pushed since, concatenated.
  it("does not repeat a sample that arrives after a finished chunk load", () => {
    const buffers = createTestBuffers();
    const state = createTestState();
    const dispatch = createEventDispatcher(buffers, state, "ws-0");
    dispatch("ws-0", { kind: "info", info: makeSimInfo() });
    dispatch("ws-0", {
      kind: "history-chunk",
      points: [makePoint(0, "sat1"), makePoint(10, "sat1")],
      done: true,
    });
    // One more sample before the worker's next tick.
    dispatch("ws-0", { kind: "state", point: makePoint(20, "sat1") });

    const buf = buffers.ingestBuffers.get("sat1") as unknown as IngestBufferStub;
    const rebuilt = buf.consumeRebuild();
    expect(rebuilt?.map((p) => p.t)).toEqual([0, 10, 20]);
  });

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

  it("history-chunk accumulates points into the trail and the ingest buffer", () => {
    const buffers = createTestBuffers();
    const state = createTestState();
    const dispatch = createEventDispatcher(buffers, state, "ws-0");

    const chunk1 = [makePoint(0, "sat1"), makePoint(10, "sat1")];
    const chunk2 = [makePoint(20, "sat1")];
    dispatch("ws-0", { kind: "history-chunk", points: chunk1, done: false });
    expect(buffers.trailBuffers.get("sat1")?.length).toBe(2);

    dispatch("ws-0", { kind: "history-chunk", points: chunk2, done: true });
    expect(buffers.trailBuffers.get("sat1")?.length).toBe(3);
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

describe("a file load (history-chunk)", () => {
  // Here a trail a stream builds keeps at most 3 points (capacity 2, trimmed
  // past 1.5x), so anything taken from such a trail loses the head of the
  // six-point loads below.
  beforeEach(() => {
    setTrailBufferFactory(
      (_id, retention) => new TrailBuffer(retention === "whole" ? Number.POSITIVE_INFINITY : 2),
    );
  });

  const LOAD = [0, 10, 20, 30, 40, 50];

  /** Dispatch `ts` for `id` in chunks of two, the last chunk marked done. */
  function loadInChunks(
    dispatch: ReturnType<typeof createEventDispatcher>,
    ts: number[],
    id = "sat1",
  ): void {
    for (let i = 0; i < ts.length; i += 2) {
      dispatch("csv-file", {
        kind: "history-chunk",
        points: ts.slice(i, i + 2).map((t) => makePoint(t, id)),
        done: false,
      });
    }
    dispatch("csv-file", { kind: "history-chunk", points: [], done: true });
  }

  function ingestOf(buffers: RuntimeBuffers, id = "sat1"): IngestBufferStub {
    return buffers.ingestBuffers.get(id) as unknown as IngestBufferStub;
  }

  it("hands DuckDB every point it loads, more than a stream's trail keeps", () => {
    const buffers = createTestBuffers();
    loadInChunks(createEventDispatcher(buffers, createTestState(), "csv-file"), LOAD);

    expect(
      ingestOf(buffers)
        .consumeRebuild()
        ?.map((p) => p.t),
    ).toEqual(LOAD);
  });

  it("keeps the file's whole trail", () => {
    const buffers = createTestBuffers();
    loadInChunks(createEventDispatcher(buffers, createTestState(), "csv-file"), LOAD);

    expect(
      buffers.trailBuffers
        .get("sat1")
        ?.getAll()
        .map((p) => p.t),
    ).toEqual(LOAD);
  });

  it("leaves a streamed trail bounded", () => {
    const buffers = createTestBuffers();
    const dispatch = createEventDispatcher(buffers, createTestState(), "ws-0");
    for (const t of LOAD) dispatch("ws-0", { kind: "state", point: makePoint(t, "sat1") });

    expect(buffers.trailBuffers.get("sat1")?.length).toBeLessThanOrEqual(3);
  });

  it("drops a stream's trail instead of reusing it", () => {
    const buffers = createTestBuffers();
    const streamed = createEventDispatcher(buffers, createTestState(), "ws-0");
    streamed("ws-0", { kind: "state", point: makePoint(0, "sat1") });
    loadInChunks(createEventDispatcher(buffers, createTestState(), "csv-file"), LOAD);

    expect(buffers.trailBuffers.get("sat1")?.length).toBe(LOAD.length);
  });

  // The DuckDB table outlives a load, so a load replaces the rows the one
  // before it left rather than appending to them.
  it("replaces the rows of the load before it", () => {
    const buffers = createTestBuffers();
    loadInChunks(createEventDispatcher(buffers, createTestState(), "csv-file"), [0, 10, 20]);
    ingestOf(buffers).consumeRebuild();

    loadInChunks(createEventDispatcher(buffers, createTestState(), "csv-file"), [0, 5]);
    expect(
      ingestOf(buffers)
        .consumeRebuild()
        ?.map((p) => p.t),
    ).toEqual([0, 5]);
  });

  it("rebuilds a satellite that first appears in a later chunk", () => {
    const buffers = createTestBuffers();
    const dispatch = createEventDispatcher(buffers, createTestState(), "csv-file");
    dispatch("csv-file", {
      kind: "history-chunk",
      points: [makePoint(0, "sat1"), makePoint(10, "sat1")],
      done: false,
    });
    dispatch("csv-file", {
      kind: "history-chunk",
      points: [makePoint(0, "sat2"), makePoint(10, "sat2")],
      done: true,
    });

    expect(
      ingestOf(buffers, "sat2")
        .consumeRebuild()
        ?.map((p) => p.t),
    ).toEqual([0, 10]);
  });

  // The worker may take the rebuild while the file is still loading: the rows
  // so far replace the table, and the rest arrive as appends.
  it("sends the rest of a load as appends once its rebuild is taken", () => {
    const buffers = createTestBuffers();
    const dispatch = createEventDispatcher(buffers, createTestState(), "csv-file");
    dispatch("csv-file", {
      kind: "history-chunk",
      points: [makePoint(0, "sat1"), makePoint(10, "sat1")],
      done: false,
    });
    expect(
      ingestOf(buffers)
        .consumeRebuild()
        ?.map((p) => p.t),
    ).toEqual([0, 10]);

    dispatch("csv-file", {
      kind: "history-chunk",
      points: [makePoint(20, "sat1"), makePoint(30, "sat1")],
      done: true,
    });
    expect(ingestOf(buffers).consumeRebuild()).toBeNull();
    expect(
      ingestOf(buffers)
        .drain()
        .map((p) => p.t),
    ).toEqual([20, 30]);
  });

  it("clears what the load before it left when the file has no points", () => {
    const buffers = createTestBuffers();
    loadInChunks(createEventDispatcher(buffers, createTestState(), "csv-file"), [0, 10]);
    ingestOf(buffers).consumeRebuild();

    loadInChunks(createEventDispatcher(buffers, createTestState(), "csv-file"), []);
    expect(ingestOf(buffers).consumeRebuild()).toEqual([]);
    expect(buffers.trailBuffers.get("sat1")?.length ?? 0).toBe(0);
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
