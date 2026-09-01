import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ORBIT_DERIVED_STRIDE } from "./rrdOrbitDerived.js";
import type { RrdWorkerMessage } from "./rrdParseLogic.js";
import type { SourceEvent, SourceId } from "./types.js";

/// Records what the derived batch was asked to compute against.
const derivedCalls: { mu: number; bodyRadius: number; states: number }[] = [];

/// Lets a test hold `initArika()` pending across a restart.
const pendingInits: (() => void)[] = [];
let holdInit = false;

vi.mock("../wasm/arikaInit.js", () => ({
  initArika: () =>
    holdInit
      ? new Promise<void>((resolve) => {
          pendingInits.push(resolve);
        })
      : Promise.resolve(),
  orbit_derived_batch: (states: Float64Array, mu: number, bodyRadius: number) => {
    derivedCalls.push({ mu, bodyRadius, states: states.length / 6 });
    return new Float64Array((states.length / 6) * ORBIT_DERIVED_STRIDE).fill(0);
  },
}));

const { RrdFileAdapter } = await import("./RrdFileAdapter.js");

class MockWorker {
  static instances: MockWorker[] = [];
  onmessage: ((e: { data: RrdWorkerMessage }) => void) | null = null;
  onerror: ((e: ErrorEvent) => void) | null = null;
  terminated = false;

  constructor() {
    MockWorker.instances.push(this);
  }

  postMessage(_data: unknown, _transfer?: unknown[]) {}

  terminate() {
    this.terminated = true;
  }

  simulateMessage(msg: RrdWorkerMessage) {
    this.onmessage?.({ data: msg });
  }
}

class MockFileReader {
  static instances: MockFileReader[] = [];
  result: ArrayBuffer | null = null;
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;

  constructor() {
    MockFileReader.instances.push(this);
  }

  readAsArrayBuffer(_file: File) {
    this.result = new ArrayBuffer(8);
    this.onload?.();
  }

  abort() {}
}

beforeEach(() => {
  derivedCalls.length = 0;
  pendingInits.length = 0;
  holdInit = false;
  MockWorker.instances = [];
  MockFileReader.instances = [];
  vi.stubGlobal("Worker", function MockWorkerConstructor() {
    return new MockWorker();
  });
  vi.stubGlobal("FileReader", MockFileReader);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

/// The worker is started from a promise chain, so it exists a microtask later.
async function latestWorker(): Promise<MockWorker> {
  await Promise.resolve();
  await Promise.resolve();
  const worker = MockWorker.instances.at(-1);
  expect(worker).toBeDefined();
  return worker as MockWorker;
}

function metadata(mu: number | null, bodyRadius: number | null) {
  return {
    type: "metadata" as const,
    metadata: {
      epoch_jd: null,
      epoch_iso: null,
      mu,
      body_radius: bodyRadius,
      altitude: null,
      period: null,
      body_name: null,
      orbit_description: null,
    },
  };
}

function chunk(): RrdWorkerMessage {
  return {
    type: "chunk",
    points: [
      {
        t: 0,
        x: 7000,
        y: 0,
        z: 0,
        vx: 0,
        vy: 7.5,
        vz: 0,
        entityPath: "world/sat/default",
      },
    ],
  } as RrdWorkerMessage;
}

describe("RrdFileAdapter", () => {
  it("derives against the central body its own metadata names", async () => {
    const events: SourceEvent[] = [];
    const handler = (_id: SourceId, e: SourceEvent) => events.push(e);
    const adapter = new RrdFileAdapter("rrd-0", new File([new Uint8Array(8)], "a.rrd"), handler);

    adapter.start();
    const worker = await latestWorker();
    worker.simulateMessage(metadata(42_000, 1234) as unknown as RrdWorkerMessage);
    worker.simulateMessage(chunk());

    expect(derivedCalls).toHaveLength(1);
    expect(derivedCalls[0].mu).toBe(42_000);
    expect(derivedCalls[0].bodyRadius).toBe(1234);
  });

  it("does not carry the previous load's central body into the next one", async () => {
    const events: SourceEvent[] = [];
    const handler = (_id: SourceId, e: SourceEvent) => events.push(e);
    const adapter = new RrdFileAdapter("rrd-0", new File([new Uint8Array(8)], "a.rrd"), handler);

    // A first recording that names its own central body.
    adapter.start();
    const first = await latestWorker();
    first.simulateMessage(metadata(42_000, 1234) as unknown as RrdWorkerMessage);
    first.simulateMessage(chunk());
    expect(derivedCalls[0].mu).toBe(42_000);

    // A second load whose first chunk arrives before any metadata. The worker
    // posts metadata first, so nothing says which body these points are around:
    // they are reported as an error rather than measured against the recording
    // before, which is what carrying the resolved body over would have done.
    adapter.start();
    const second = await latestWorker();
    expect(second).not.toBe(first);
    events.length = 0;
    second.simulateMessage(chunk());

    expect(derivedCalls).toHaveLength(1);
    expect(events).toEqual([
      {
        kind: "error",
        message: expect.stringContaining("before it said which body"),
      },
    ]);
  });

  it("a load restarted while the WASM was loading does not start a second worker", async () => {
    holdInit = true;
    const events: SourceEvent[] = [];
    const handler = (_id: SourceId, e: SourceEvent) => events.push(e);
    const adapter = new RrdFileAdapter("rrd-0", new File([new Uint8Array(8)], "a.rrd"), handler);

    // Both loads read the file and then wait on the WASM.
    adapter.start();
    adapter.start();
    expect(pendingInits).toHaveLength(2);
    expect(MockWorker.instances).toHaveLength(0);

    // The first load's wait finishes after the restart owns the adapter. It
    // must not put its own buffer behind the current load's worker reference:
    // `stopped` is false again by then, so it alone let the stale load through.
    pendingInits[0]();
    await Promise.resolve();
    await Promise.resolve();
    expect(MockWorker.instances).toHaveLength(0);

    // The current load still starts, and it is the only worker.
    pendingInits[1]();
    const worker = await latestWorker();
    expect(MockWorker.instances).toHaveLength(1);
    worker.simulateMessage(metadata(42_000, 1234) as unknown as RrdWorkerMessage);
    worker.simulateMessage(chunk());
    expect(derivedCalls).toHaveLength(1);
  });

  it("measures against the body the recording names, not the one it omits", async () => {
    const events: SourceEvent[] = [];
    const handler = (_id: SourceId, e: SourceEvent) => events.push(e);
    const adapter = new RrdFileAdapter("rrd-0", new File([new Uint8Array(8)], "a.rrd"), handler);

    // Mars, with neither constant of its own. Taking Earth's radius here would
    // put altitude out by about 3000 km with nothing on the chart to say so.
    adapter.start();
    const worker = await latestWorker();
    worker.simulateMessage({
      type: "metadata",
      metadata: {
        epoch_jd: null,
        epoch_iso: null,
        mu: null,
        body_radius: null,
        altitude: null,
        period: null,
        body_name: "mars",
        orbit_description: null,
      },
    } as unknown as RrdWorkerMessage);
    worker.simulateMessage(chunk());

    expect(derivedCalls).toHaveLength(1);
    expect(derivedCalls[0].mu).toBeCloseTo(42828.375214, 6);
    expect(derivedCalls[0].bodyRadius).toBeCloseTo(3396.2, 6);
  });

  it("reports a recording it cannot measure instead of reading it as Earth", async () => {
    const events: SourceEvent[] = [];
    const handler = (_id: SourceId, e: SourceEvent) => events.push(e);
    const adapter = new RrdFileAdapter("rrd-0", new File([new Uint8Array(8)], "a.rrd"), handler);

    adapter.start();
    const worker = await latestWorker();
    worker.simulateMessage({
      type: "metadata",
      metadata: {
        epoch_jd: null,
        epoch_iso: null,
        mu: null,
        body_radius: null,
        altitude: null,
        period: null,
        body_name: "kerbin",
        orbit_description: null,
      },
    } as unknown as RrdWorkerMessage);

    expect(events).toEqual([{ kind: "error", message: expect.stringContaining("kerbin") }]);
    // Nothing of the recording reaches the charts, and the worker is done.
    worker.simulateMessage(chunk());
    expect(derivedCalls).toHaveLength(0);
    expect(worker.terminated).toBe(true);
  });

  it("takes a body of the consumer's own from the catalog it is given", async () => {
    const events: SourceEvent[] = [];
    const handler = (_id: SourceId, e: SourceEvent) => events.push(e);
    const adapter = new RrdFileAdapter("rrd-0", new File([new Uint8Array(8)], "a.rrd"), handler, {
      kerbin: { mu: 3531600, radiusKm: 600 },
    });

    adapter.start();
    const worker = await latestWorker();
    worker.simulateMessage({
      type: "metadata",
      metadata: {
        epoch_jd: null,
        epoch_iso: null,
        mu: null,
        body_radius: null,
        altitude: null,
        period: null,
        body_name: "kerbin",
        orbit_description: null,
      },
    } as unknown as RrdWorkerMessage);
    worker.simulateMessage(chunk());

    expect(events.filter((e) => e.kind === "error")).toEqual([]);
    expect(derivedCalls).toEqual([{ mu: 3531600, bodyRadius: 600, states: 1 }]);
  });
});
