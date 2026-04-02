import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SourceEvent, SourceId } from "./types.js";
import { WebSocketAdapter } from "./WebSocketAdapter.js";

// --- Mock WebSocket ---
class MockWebSocket {
  static instances: MockWebSocket[] = [];
  readyState = 0; // CONNECTING
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  onerror: (() => void) | null = null;
  sentMessages: string[] = [];

  constructor(public url: string) {
    MockWebSocket.instances.push(this);
  }

  send(data: string) {
    this.sentMessages.push(data);
  }

  close() {
    this.readyState = 3; // CLOSED
    this.onclose?.();
  }

  // Test helpers
  simulateOpen() {
    this.readyState = 1; // OPEN
    this.onopen?.();
  }

  simulateMessage(data: unknown) {
    this.onmessage?.({ data: JSON.stringify(data) });
  }

  simulateClose() {
    this.readyState = 3;
    this.onclose?.();
  }

  static readonly OPEN = 1;
}

beforeEach(() => {
  MockWebSocket.instances = [];
  vi.stubGlobal("WebSocket", MockWebSocket);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

function collectEvents(sourceId: SourceId): {
  events: SourceEvent[];
  handler: (id: SourceId, e: SourceEvent) => void;
} {
  const events: SourceEvent[] = [];
  const handler = (id: SourceId, e: SourceEvent) => {
    expect(id).toBe(sourceId);
    events.push(e);
  };
  return { events, handler };
}

describe("WebSocketAdapter", () => {
  it("starts with disconnected state", () => {
    const { handler } = collectEvents("ws-0");
    const adapter = new WebSocketAdapter("ws-0", "ws://localhost:9001/ws", handler);
    expect(adapter.connectionState).toBe("disconnected");
    expect(adapter.capabilities.live).toBe(true);
    expect(adapter.capabilities.control).toBe(true);
  });

  it("connects on start and emits connecting state", () => {
    const { handler } = collectEvents("ws-0");
    const adapter = new WebSocketAdapter("ws-0", "ws://localhost:9001/ws", handler);
    adapter.start();
    expect(adapter.connectionState).toBe("connecting");
    expect(MockWebSocket.instances).toHaveLength(1);
    expect(MockWebSocket.instances[0].url).toBe("ws://localhost:9001/ws");
  });

  it("transitions to connected on WebSocket open", () => {
    const { handler } = collectEvents("ws-0");
    const adapter = new WebSocketAdapter("ws-0", "ws://localhost:9001/ws", handler);
    adapter.start();
    MockWebSocket.instances[0].simulateOpen();
    expect(adapter.connectionState).toBe("connected");
  });

  it("emits info event from info message", () => {
    const { events, handler } = collectEvents("ws-0");
    const adapter = new WebSocketAdapter("ws-0", "ws://localhost:9001/ws", handler);
    adapter.start();
    MockWebSocket.instances[0].simulateOpen();

    MockWebSocket.instances[0].simulateMessage({
      type: "info",
      mu: 398600,
      dt: 10,
      output_interval: 10,
      stream_interval: 10,
      central_body: "earth",
      central_body_radius: 6378.137,
      epoch_jd: 2451545.0,
      satellites: [{ id: "sat1", name: "ISS", altitude: 400, period: 5400, perturbations: [] }],
    });

    expect(events).toHaveLength(1);
    expect(events[0].kind).toBe("info");
  });

  it("emits state event from state message", () => {
    const { events, handler } = collectEvents("ws-0");
    const adapter = new WebSocketAdapter("ws-0", "ws://localhost:9001/ws", handler);
    adapter.start();
    MockWebSocket.instances[0].simulateOpen();

    MockWebSocket.instances[0].simulateMessage({
      type: "state",
      entity_path: "sat1",
      t: 10,
      position: [7000, 0, 0],
      velocity: [0, 7.5, 0],
      semi_major_axis: 7000,
      eccentricity: 0,
      inclination: 0,
      raan: 0,
      argument_of_periapsis: 0,
      true_anomaly: 0,
    });

    expect(events).toHaveLength(1);
    expect(events[0].kind).toBe("state");
    if (events[0].kind === "state") {
      expect(events[0].point.x).toBe(7000);
    }
  });

  it("emits terminated event", () => {
    const { events, handler } = collectEvents("ws-0");
    const adapter = new WebSocketAdapter("ws-0", "ws://localhost:9001/ws", handler);
    adapter.start();
    MockWebSocket.instances[0].simulateOpen();

    MockWebSocket.instances[0].simulateMessage({
      type: "simulation_terminated",
      entity_path: "sat1",
      t: 100,
      reason: "impact",
    });

    expect(events).toHaveLength(1);
    expect(events[0].kind).toBe("terminated");
  });

  it("emits server-state from status message", () => {
    const { events, handler } = collectEvents("ws-0");
    const adapter = new WebSocketAdapter("ws-0", "ws://localhost:9001/ws", handler);
    adapter.start();
    MockWebSocket.instances[0].simulateOpen();

    MockWebSocket.instances[0].simulateMessage({ type: "status", state: "paused" });

    expect(events).toHaveLength(1);
    expect(events[0].kind).toBe("server-state");
    if (events[0].kind === "server-state") {
      expect(events[0].state).toBe("paused");
    }
  });

  it("send() sends JSON to WebSocket", () => {
    const { handler } = collectEvents("ws-0");
    const adapter = new WebSocketAdapter("ws-0", "ws://localhost:9001/ws", handler);
    adapter.start();
    MockWebSocket.instances[0].simulateOpen();

    adapter.send({ type: "pause_simulation" });
    expect(MockWebSocket.instances[0].sentMessages).toHaveLength(1);
    expect(JSON.parse(MockWebSocket.instances[0].sentMessages[0])).toEqual({
      type: "pause_simulation",
    });
  });

  it("stop() closes WebSocket and transitions to disconnected", () => {
    const { handler } = collectEvents("ws-0");
    const adapter = new WebSocketAdapter("ws-0", "ws://localhost:9001/ws", handler);
    adapter.start();
    MockWebSocket.instances[0].simulateOpen();

    adapter.stop();
    expect(adapter.connectionState).toBe("disconnected");
  });

  it("emits textures-ready event", () => {
    const { events, handler } = collectEvents("ws-0");
    const adapter = new WebSocketAdapter("ws-0", "ws://localhost:9001/ws", handler);
    adapter.start();
    MockWebSocket.instances[0].simulateOpen();

    MockWebSocket.instances[0].simulateMessage({ type: "textures_ready", body: "moon" });

    expect(events).toHaveLength(1);
    expect(events[0].kind).toBe("textures-ready");
  });
});
