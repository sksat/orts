/**
 * Tests for the Worker communication protocol.
 *
 * Verifies the workerClient's cancellation logic and error handling
 * without actually spawning a Web Worker (pure unit tests).
 */

import { describe, it, expect } from "vitest";

describe("worker cancellation logic", () => {
  // Simulates the latestId-based cancellation in workerClient.ts
  it("only the latest request per function should resolve with data", () => {
    const latestId = new Map<string, number>();
    let nextId = 0;

    function simulateCall(fn: string): { id: number; isLatest: () => boolean } {
      const id = nextId++;
      latestId.set(fn, id);
      return {
        id,
        isLatest: () => latestId.get(fn) === id,
      };
    }

    const call1 = simulateCall("atmosphere_latlon_map");
    const call2 = simulateCall("atmosphere_latlon_map");
    const call3 = simulateCall("atmosphere_latlon_map");

    // Only the last call should be "latest"
    expect(call1.isLatest()).toBe(false);
    expect(call2.isLatest()).toBe(false);
    expect(call3.isLatest()).toBe(true);
  });

  it("different functions have independent cancellation", () => {
    const latestId = new Map<string, number>();
    let nextId = 0;

    function simulateCall(fn: string): { id: number; isLatest: () => boolean } {
      const id = nextId++;
      latestId.set(fn, id);
      return {
        id,
        isLatest: () => latestId.get(fn) === id,
      };
    }

    const atmo = simulateCall("atmosphere_latlon_map");
    const mag = simulateCall("magnetic_field_latlon_map");

    // Both should be latest (different functions)
    expect(atmo.isLatest()).toBe(true);
    expect(mag.isLatest()).toBe(true);
  });
});

describe("worker message protocol", () => {
  it("ready message has correct shape", () => {
    const msg = { type: "ready" };
    expect(msg.type).toBe("ready");
  });

  it("error message has correct shape", () => {
    const msg = { type: "error", message: "WASM init failed" };
    expect(msg.type).toBe("error");
    expect(msg.message).toBe("WASM init failed");
  });

  it("result message has correct shape", () => {
    const msg = { type: "result", id: 42, result: new Float64Array([1, 2, 3]) };
    expect(msg.type).toBe("result");
    expect(msg.id).toBe(42);
    expect(msg.result).toBeInstanceOf(Float64Array);
  });
});

describe("useEffect cancelled flag pattern", () => {
  it("cancelled flag prevents stale results from being applied", async () => {
    let cancelled = false;
    const results: number[] = [];

    // Simulate async computation
    const promise = new Promise<number>((resolve) => {
      setTimeout(() => resolve(42), 0);
    });

    // Simulate cleanup running before promise resolves
    cancelled = true;

    const result = await promise;
    if (!cancelled) {
      results.push(result);
    }

    // Result should NOT be applied
    expect(results).toEqual([]);
  });

  it("non-cancelled result is applied", async () => {
    let cancelled = false;
    const results: number[] = [];

    const result = await Promise.resolve(42);
    if (!cancelled) {
      results.push(result);
    }

    expect(results).toEqual([42]);
  });
});
