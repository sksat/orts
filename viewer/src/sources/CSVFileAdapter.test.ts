import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CSVFileAdapter } from "./CSVFileAdapter.js";
import type { CSVWorkerMessage } from "./csvParseLogic.js";
import type { SourceEvent, SourceId } from "./types.js";

// --- Mock Worker ---
class MockWorker {
  static instances: MockWorker[] = [];
  onmessage: ((e: { data: CSVWorkerMessage }) => void) | null = null;
  terminated = false;
  postedMessages: unknown[] = [];

  constructor() {
    MockWorker.instances.push(this);
  }

  postMessage(data: unknown) {
    this.postedMessages.push(data);
  }

  terminate() {
    this.terminated = true;
  }

  // Test helper: simulate worker responses
  simulateMessage(msg: CSVWorkerMessage) {
    this.onmessage?.({ data: msg });
  }
}

// --- Mock FileReader ---
class MockFileReader {
  static instances: MockFileReader[] = [];
  result: string | null = null;
  onload: (() => void) | null = null;

  constructor() {
    MockFileReader.instances.push(this);
  }

  readAsText(_file: File) {
    // Simulate synchronous read completion for test determinism
    this.result = "mock csv content";
    this.onload?.();
  }

  abort() {
    // No-op for testing
  }
}

beforeEach(() => {
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

function collectEvents(sourceId: SourceId) {
  const events: SourceEvent[] = [];
  const handler = (id: SourceId, e: SourceEvent) => {
    expect(id).toBe(sourceId);
    events.push(e);
  };
  return { events, handler };
}

describe("CSVFileAdapter", () => {
  it("starts with disconnected state", () => {
    const { handler } = collectEvents("csv-0");
    const file = new File(["test"], "test.csv", { type: "text/csv" });
    const adapter = new CSVFileAdapter("csv-0", file, handler);
    expect(adapter.connectionState).toBe("disconnected");
    expect(adapter.capabilities.live).toBe(false);
    expect(adapter.capabilities.control).toBe(false);
  });

  it("transitions to loading on start", async () => {
    const { handler } = collectEvents("csv-0");
    const file = new File(["test"], "test.csv", { type: "text/csv" });
    const adapter = new CSVFileAdapter("csv-0", file, handler);
    adapter.start();
    expect(adapter.connectionState).toBe("loading");
  });

  it("defers info event until first chunk arrives (dt estimation)", () => {
    const { events, handler } = collectEvents("csv-0");
    const file = new File(["0,7000,0,0,0,7.5,0"], "test.csv");
    const adapter = new CSVFileAdapter("csv-0", file, handler);
    adapter.start();

    expect(MockWorker.instances).toHaveLength(1);
    const worker = MockWorker.instances[0];

    // Metadata alone should NOT emit info yet
    worker.simulateMessage({
      type: "metadata",
      metadata: {
        epochJd: 2451545.0,
        mu: 398600.4418,
        centralBody: "earth",
        centralBodyRadius: 6378.137,
        satelliteName: null,
      },
    });
    expect(events).toHaveLength(0);

    // First chunk triggers info + history-chunk
    const point1 = {
      t: 0,
      x: 7000,
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
      accel_gravity: 0,
      accel_drag: 0,
      accel_srp: 0,
      accel_third_body_sun: 0,
      accel_third_body_moon: 0,
    };
    const point2 = { ...point1, t: 5 };
    worker.simulateMessage({ type: "chunk", points: [point1, point2] });

    expect(events).toHaveLength(2); // info + history-chunk
    expect(events[0].kind).toBe("info");
    if (events[0].kind === "info") {
      expect(events[0].info.epoch_jd).toBe(2451545.0);
      expect(events[0].info.dt).toBe(5); // estimated from points
    }
    expect(events[1].kind).toBe("history-chunk");
  });

  it("emits history-chunk events from chunk worker messages", async () => {
    const { events, handler } = collectEvents("csv-0");
    const file = new File(["data"], "test.csv");
    const adapter = new CSVFileAdapter("csv-0", file, handler);
    adapter.start();

    expect(MockWorker.instances).toHaveLength(1);
    const worker = MockWorker.instances[0];

    const point = {
      t: 0,
      x: 7000,
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
      accel_gravity: 0,
      accel_drag: 0,
      accel_srp: 0,
      accel_third_body_sun: 0,
      accel_third_body_moon: 0,
    };

    worker.simulateMessage({ type: "chunk", points: [point] });
    expect(events).toHaveLength(1);
    expect(events[0].kind).toBe("history-chunk");
    if (events[0].kind === "history-chunk") {
      expect(events[0].done).toBe(false);
      expect(events[0].points).toHaveLength(1);
    }
  });

  it("emits complete on worker complete message", async () => {
    const { events, handler } = collectEvents("csv-0");
    const file = new File(["data"], "test.csv");
    const adapter = new CSVFileAdapter("csv-0", file, handler);
    adapter.start();

    expect(MockWorker.instances).toHaveLength(1);
    const worker = MockWorker.instances[0];

    worker.simulateMessage({ type: "complete", totalPoints: 100 });
    expect(events.some((e) => e.kind === "history-chunk" && e.done === true)).toBe(true);
    expect(events.some((e) => e.kind === "complete")).toBe(true);
    expect(adapter.connectionState).toBe("complete");
  });

  it("stop terminates worker", async () => {
    const { handler } = collectEvents("csv-0");
    const file = new File(["data"], "test.csv");
    const adapter = new CSVFileAdapter("csv-0", file, handler);
    adapter.start();

    expect(MockWorker.instances).toHaveLength(1);
    adapter.stop();
    expect(MockWorker.instances[0].terminated).toBe(true);
    expect(adapter.connectionState).toBe("disconnected");
  });
});
