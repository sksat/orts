import { describe, it, expect, vi } from "vitest";
import { dispatchServerMessage, type ServerMessage } from "./useWebSocket.js";

describe("dispatchServerMessage", () => {
  const noop = () => {};
  const baseCallbacks = {
    onState: noop,
    onInfo: noop,
    onHistory: noop,
    onHistoryDetail: noop,
    onHistoryDetailComplete: noop,
  } as const;

  it("dispatches simulation_terminated message", () => {
    const onTerminated = vi.fn();
    const callbacks = { ...baseCallbacks, onSimulationTerminated: onTerminated };

    const msg: ServerMessage = {
      type: "simulation_terminated",
      satellite_id: "sat-a",
      t: 1234.5,
      reason: "atmospheric_entry",
    };

    dispatchServerMessage(msg, callbacks);

    expect(onTerminated).toHaveBeenCalledOnce();
    expect(onTerminated).toHaveBeenCalledWith("sat-a", 1234.5, "atmospheric_entry");
  });

  it("dispatches state message", () => {
    const onState = vi.fn();
    const callbacks = { ...baseCallbacks, onState };

    const msg: ServerMessage = {
      type: "state",
      satellite_id: "sat-a",
      t: 10,
      position: [6778, 0, 0],
      velocity: [0, 7.669, 0],
      semi_major_axis: 6778,
      eccentricity: 0,
      inclination: 0.9,
      raan: 0,
      argument_of_periapsis: 0,
      true_anomaly: 0,
    };

    dispatchServerMessage(msg, callbacks);

    expect(onState).toHaveBeenCalledOnce();
    expect(onState.mock.calls[0][0].satelliteId).toBe("sat-a");
    expect(onState.mock.calls[0][0].t).toBe(10);
  });

  it("ignores unknown message types without error", () => {
    const callbacks = { ...baseCallbacks };
    // Simulate a future message type the viewer doesn't know about
    const msg = { type: "unknown_future_type" } as unknown as ServerMessage;

    expect(() => dispatchServerMessage(msg, callbacks)).not.toThrow();
  });
});
