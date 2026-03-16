import { describe, expect, it, vi } from "vitest";
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

  it("dispatches history message with parsed OrbitPoints", () => {
    const onHistory = vi.fn();
    const callbacks = { ...baseCallbacks, onHistory };

    const msg: ServerMessage = {
      type: "history",
      states: [
        {
          satellite_id: "sat-a",
          t: 0,
          position: [6778, 0, 0] as [number, number, number],
          velocity: [0, 7.669, 0] as [number, number, number],
          semi_major_axis: 6778,
          eccentricity: 0,
          inclination: 0.9,
          raan: 0,
          argument_of_periapsis: 0,
          true_anomaly: 0,
        },
        {
          satellite_id: "sat-a",
          t: 10,
          position: [6770, 500, 0] as [number, number, number],
          velocity: [-0.5, 7.6, 0] as [number, number, number],
          semi_major_axis: 6778,
          eccentricity: 0.001,
          inclination: 0.9,
          raan: 0,
          argument_of_periapsis: 0,
          true_anomaly: 0.01,
        },
      ],
    };

    dispatchServerMessage(msg, callbacks);

    expect(onHistory).toHaveBeenCalledOnce();
    const points = onHistory.mock.calls[0][0];
    expect(points).toHaveLength(2);
    expect(points[0].satelliteId).toBe("sat-a");
    expect(points[0].t).toBe(0);
    expect(points[0].x).toBe(6778);
    expect(points[0].vy).toBe(7.669);
    expect(points[1].t).toBe(10);
    expect(points[1].x).toBe(6770);
  });

  it("dispatches history_detail separately from history", () => {
    const onHistory = vi.fn();
    const onHistoryDetail = vi.fn();
    const callbacks = { ...baseCallbacks, onHistory, onHistoryDetail };

    const msg: ServerMessage = {
      type: "history_detail",
      states: [
        {
          satellite_id: "sat-a",
          t: 5,
          position: [6775, 200, 0] as [number, number, number],
          velocity: [-0.2, 7.65, 0] as [number, number, number],
          semi_major_axis: 6778,
          eccentricity: 0,
          inclination: 0.9,
          raan: 0,
          argument_of_periapsis: 0,
          true_anomaly: 0.005,
        },
      ],
    };

    dispatchServerMessage(msg, callbacks);

    expect(onHistory).not.toHaveBeenCalled();
    expect(onHistoryDetail).toHaveBeenCalledOnce();
    const points = onHistoryDetail.mock.calls[0][0];
    expect(points).toHaveLength(1);
    expect(points[0].t).toBe(5);
  });

  it("dispatches history_detail_complete", () => {
    const onComplete = vi.fn();
    const callbacks = { ...baseCallbacks, onHistoryDetailComplete: onComplete };

    const msg: ServerMessage = { type: "history_detail_complete" };
    dispatchServerMessage(msg, callbacks);

    expect(onComplete).toHaveBeenCalledOnce();
  });

  it("dispatches status message", () => {
    const onStatus = vi.fn();
    const callbacks = { ...baseCallbacks, onStatus };

    const msg: ServerMessage = { type: "status", state: "idle" };
    dispatchServerMessage(msg, callbacks);

    expect(onStatus).toHaveBeenCalledOnce();
    expect(onStatus).toHaveBeenCalledWith("idle");
  });

  it("dispatches status paused message", () => {
    const onStatus = vi.fn();
    const callbacks = { ...baseCallbacks, onStatus };

    const msg: ServerMessage = { type: "status", state: "paused" };
    dispatchServerMessage(msg, callbacks);

    expect(onStatus).toHaveBeenCalledOnce();
    expect(onStatus).toHaveBeenCalledWith("paused");
  });

  it("dispatches status running message", () => {
    const onStatus = vi.fn();
    const callbacks = { ...baseCallbacks, onStatus };

    const msg: ServerMessage = { type: "status", state: "running" };
    dispatchServerMessage(msg, callbacks);

    expect(onStatus).toHaveBeenCalledOnce();
    expect(onStatus).toHaveBeenCalledWith("running");
  });

  it("dispatches error message", () => {
    const onError = vi.fn();
    const callbacks = { ...baseCallbacks, onError };

    const msg: ServerMessage = { type: "error", message: "Simulation is already running" };
    dispatchServerMessage(msg, callbacks);

    expect(onError).toHaveBeenCalledOnce();
    expect(onError).toHaveBeenCalledWith("Simulation is already running");
  });
});
