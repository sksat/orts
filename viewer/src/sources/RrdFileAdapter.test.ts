import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ORBIT_DERIVED_STRIDE } from "./rrdOrbitDerived.js";
import type { RrdWorkerMessage } from "./rrdParseLogic.js";
import type { SourceEvent, SourceId } from "./types.js";

/// Records what the derived batch was asked to compute against.
const derivedCalls: { mu: number; bodyRadius: number; states: number }[] = [];

vi.mock("../wasm/arikaInit.js", () => ({
  initArika: () => Promise.resolve(),
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
    // posts metadata first, so this is the ordering `start()` must not depend
    // on: `start()` resets every other per-load field, and leaving the central
    // body behind derived the new recording against the old one's.
    adapter.start();
    const second = await latestWorker();
    expect(second).not.toBe(first);
    second.simulateMessage(chunk());

    expect(derivedCalls).toHaveLength(2);
    expect(derivedCalls[1].mu).not.toBe(42_000);
    expect(derivedCalls[1].bodyRadius).not.toBe(1234);
  });
});
