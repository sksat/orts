import { describe, expect, it, vi } from "vitest";
import { dispatchServerMessage, type ServerMessage } from "./useWebSocket.js";

describe("dispatchServerMessage", () => {
  const noop = () => {};
  const baseCallbacks = {
    onState: noop,
    onInfo: noop,
    onHistory: noop,
  } as const;

  it("dispatches simulation_terminated message", () => {
    const onTerminated = vi.fn();
    const callbacks = { ...baseCallbacks, onSimulationTerminated: onTerminated };

    const msg: ServerMessage = {
      type: "simulation_terminated",
      entity_path: "sat-a",
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
      entity_path: "sat-a",
      t: 10,
      position: [6778, 0, 0],
      velocity: [0, 7.669, 0],
      semi_major_axis: 6778,
      eccentricity: 0,
      inclination: 0.9,
      raan: 0,
      argument_of_periapsis: 0,
      true_anomaly: 0,
      altitude: 399.863,
      specific_energy: -29.4,
      angular_momentum: 51988.882,
      velocity_mag: 7.669,
    };

    dispatchServerMessage(msg, callbacks);

    expect(onState).toHaveBeenCalledOnce();
    expect(onState.mock.calls[0][0].entityPath).toBe("sat-a");
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
          entity_path: "sat-a",
          t: 0,
          position: [6778, 0, 0] as [number, number, number],
          velocity: [0, 7.669, 0] as [number, number, number],
          semi_major_axis: 6778,
          eccentricity: 0,
          inclination: 0.9,
          raan: 0,
          argument_of_periapsis: 0,
          true_anomaly: 0,
          altitude: 399.863,
          specific_energy: -29.4,
          angular_momentum: 51988.882,
          velocity_mag: 7.669,
        },
        {
          entity_path: "sat-a",
          t: 10,
          position: [6770, 500, 0] as [number, number, number],
          velocity: [-0.5, 7.6, 0] as [number, number, number],
          semi_major_axis: 6778,
          eccentricity: 0.001,
          inclination: 0.9,
          raan: 0,
          argument_of_periapsis: 0,
          true_anomaly: 0.01,
          altitude: 410.3,
          specific_energy: -29.4,
          angular_momentum: 51988.882,
          velocity_mag: 7.616,
        },
      ],
    };

    dispatchServerMessage(msg, callbacks);

    expect(onHistory).toHaveBeenCalledOnce();
    const points = onHistory.mock.calls[0][0];
    expect(points).toHaveLength(2);
    expect(points[0].entityPath).toBe("sat-a");
    expect(points[0].t).toBe(0);
    expect(points[0].x).toBe(6778);
    expect(points[0].vy).toBe(7.669);
    expect(points[1].t).toBe(10);
    expect(points[1].x).toBe(6770);
  });

  it("dispatches status message", () => {
    const onStatus = vi.fn();
    const callbacks = { ...baseCallbacks, onStatus };

    const msg: ServerMessage = { type: "status", state: "idle" };
    dispatchServerMessage(msg, callbacks);

    expect(onStatus).toHaveBeenCalledOnce();
    // No `default_pacing` on the wire, so none is passed on.
    expect(onStatus).toHaveBeenCalledWith("idle", undefined);
  });

  it("dispatches status paused message", () => {
    const onStatus = vi.fn();
    const callbacks = { ...baseCallbacks, onStatus };

    const msg: ServerMessage = { type: "status", state: "paused" };
    dispatchServerMessage(msg, callbacks);

    expect(onStatus).toHaveBeenCalledOnce();
    // No `default_pacing` on the wire, so none is passed on.
    expect(onStatus).toHaveBeenCalledWith("paused", undefined);
  });

  it("dispatches status running message", () => {
    const onStatus = vi.fn();
    const callbacks = { ...baseCallbacks, onStatus };

    const msg: ServerMessage = { type: "status", state: "running" };
    dispatchServerMessage(msg, callbacks);

    expect(onStatus).toHaveBeenCalledOnce();
    // No `default_pacing` on the wire, so none is passed on.
    expect(onStatus).toHaveBeenCalledWith("running", undefined);
  });

  it("dispatches error message", () => {
    const onError = vi.fn();
    const callbacks = { ...baseCallbacks, onError };

    const msg: ServerMessage = { type: "error", message: "Simulation is already running" };
    dispatchServerMessage(msg, callbacks);

    expect(onError).toHaveBeenCalledOnce();
    expect(onError).toHaveBeenCalledWith("Simulation is already running");
  });

  it("dispatches textures_ready message", () => {
    const onTexturesReady = vi.fn();
    const callbacks = { ...baseCallbacks, onTexturesReady };

    const msg: ServerMessage = { type: "textures_ready", body: "earth" };
    dispatchServerMessage(msg, callbacks);

    expect(onTexturesReady).toHaveBeenCalledOnce();
    expect(onTexturesReady).toHaveBeenCalledWith("earth");
  });

  it("handles textures_ready without callback", () => {
    const callbacks = { ...baseCallbacks };

    const msg: ServerMessage = { type: "textures_ready", body: "moon" };
    expect(() => dispatchServerMessage(msg, callbacks)).not.toThrow();
  });

  it("dispatches satellite_added message with normalized info", () => {
    const onSatelliteAdded = vi.fn();
    const callbacks = { ...baseCallbacks, onSatelliteAdded };

    const msg: ServerMessage = {
      type: "satellite_added",
      satellite: { id: "iss", altitude: 420, period: 5560, perturbations: ["drag"] },
      t: 120.5,
    };
    dispatchServerMessage(msg, callbacks);

    expect(onSatelliteAdded).toHaveBeenCalledOnce();
    expect(onSatelliteAdded).toHaveBeenCalledWith(
      { id: "iss", name: null, altitude: 420, period: 5560, perturbations: ["drag"], shape: null },
      120.5,
    );
  });

  it("normalizes a sim-declared marker shape", () => {
    const onSatelliteAdded = vi.fn();
    const callbacks = { ...baseCallbacks, onSatelliteAdded };

    const msg: ServerMessage = {
      type: "satellite_added",
      satellite: { id: "s", altitude: 400, period: 5400, perturbations: [], shape: "axes-cube" },
      t: 0,
    };
    dispatchServerMessage(msg, callbacks);

    expect(onSatelliteAdded).toHaveBeenCalledWith(
      expect.objectContaining({ id: "s", shape: "axes-cube" }),
      0,
    );
  });

  it("handles satellite_added without callback", () => {
    const msg: ServerMessage = {
      type: "satellite_added",
      satellite: { id: "sat-1", altitude: 500, period: 5677, perturbations: [] },
      t: 0,
    };
    expect(() => dispatchServerMessage(msg, { ...baseCallbacks })).not.toThrow();
  });

  it("resolves the server's central body from the body it names", () => {
    // A server predating `central_body_radius` leaves it out; the body it names
    // says which radius that is, so the info is usable as it stands.
    const onInfo = vi.fn();
    const onError = vi.fn();
    const msg = {
      type: "info",
      mu: 42828.375214,
      dt: 1,
      output_interval: 1,
      central_body: "Mars",
      satellites: [],
    } as unknown as ServerMessage;

    const outcome = dispatchServerMessage(msg, { onState: vi.fn(), onInfo, onError });

    expect(outcome).toBe("continue");
    expect(onError).not.toHaveBeenCalled();
    expect(onInfo).toHaveBeenCalledWith(
      expect.objectContaining({
        mu: 42828.375214,
        central_body: "mars",
        central_body_radius: 3396.2,
      }),
    );
  });

  it("carries the server's pacing into the info, and leaves it out when absent", () => {
    // A recording (`orts replay`) or an older server sends no pacing; the
    // viewer must not invent one, since it labels how fast Live moves.
    const infoWith = (extra: object) =>
      ({
        type: "info",
        mu: 398600.4418,
        dt: 1,
        output_interval: 1,
        stream_interval: 1,
        central_body: "earth",
        central_body_radius: 6378.137,
        satellites: [],
        ...extra,
      }) as ServerMessage;

    const onInfo = vi.fn();
    dispatchServerMessage(infoWith({ pacing: "realtime" }), { onState: vi.fn(), onInfo });
    expect(onInfo.mock.calls[0][0].pacing).toBe("realtime");

    dispatchServerMessage(infoWith({}), { onState: vi.fn(), onInfo });
    expect("pacing" in onInfo.mock.calls[1][0]).toBe(false);
  });

  it("passes the idle status's default pacing to onStatus", () => {
    const onStatus = vi.fn();
    dispatchServerMessage(
      { type: "status", state: "idle", default_pacing: "realtime" },
      { onState: vi.fn(), onStatus },
    );
    expect(onStatus).toHaveBeenCalledWith("idle", "realtime");
  });

  it("rejects the source when the server's info cannot be measured", () => {
    // Reporting the error and carrying on would let the states that follow
    // reach the charts with no `SimInfo` to measure them against, which is what
    // refusing the info exists to prevent. The caller closes on "reject".
    const onInfo = vi.fn();
    const onError = vi.fn();
    const msg = {
      type: "info",
      mu: 3531600,
      dt: 1,
      output_interval: 1,
      central_body: "kerbin",
      satellites: [],
    } as unknown as ServerMessage;

    const outcome = dispatchServerMessage(msg, { onState: vi.fn(), onInfo, onError });

    expect(outcome).toBe("reject");
    expect(onInfo).not.toHaveBeenCalled();
    expect(onError).toHaveBeenCalledWith(expect.stringContaining("kerbin"));
  });
});

describe("torques on a state message", () => {
  const noop = () => {};
  const baseCallbacks = { onState: noop, onInfo: noop, onHistory: noop } as const;
  const baseState = {
    type: "state" as const,
    entity_path: "sat-a",
    t: 10,
    position: [6778, 0, 0] as [number, number, number],
    velocity: [0, 7.669, 0] as [number, number, number],
    semi_major_axis: 6778,
    eccentricity: 0,
    inclination: 0.9,
    raan: 0,
    argument_of_periapsis: 0,
    true_anomaly: 0,
    altitude: 399.863,
    specific_energy: -29.4,
    angular_momentum: 51988.882,
    velocity_mag: 7.669,
  };

  function stateFrom(torques: { model: string; torque_body_nm: [number, number, number] }[]) {
    const onState = vi.fn();
    dispatchServerMessage({ ...baseState, torques } as ServerMessage, {
      ...baseCallbacks,
      onState,
    });
    return onState.mock.calls[0][0];
  }

  it("flattens a model's three components into its own columns", () => {
    const point = stateFrom([
      { model: "gravity_gradient", torque_body_nm: [1e-5, -2e-5, 3e-5] },
      { model: "panel_srp", torque_body_nm: [4e-7, 5e-7, 6e-7] },
    ]);

    expect(point.torque_gravity_gradient_x).toBe(1e-5);
    expect(point.torque_gravity_gradient_y).toBe(-2e-5);
    expect(point.torque_gravity_gradient_z).toBe(3e-5);
    expect(point.torque_panel_srp_x).toBe(4e-7);
  });

  // A model the run does not carry must not read as a measured zero: the
  // charts draw a gap for `undefined` and a point at zero for 0.
  it("leaves a model the run does not carry undefined", () => {
    const point = stateFrom([{ model: "gravity_gradient", torque_body_nm: [1e-5, 0, 0] }]);

    expect(point.torque_panel_srp_x).toBeUndefined();
    expect(point.torque_panel_drag_z).toBeUndefined();
  });

  it("keeps a measured zero distinct from an absent model", () => {
    const point = stateFrom([{ model: "panel_drag", torque_body_nm: [0, 0, 0] }]);

    expect(point.torque_panel_drag_x).toBe(0);
    expect(point.torque_gravity_gradient_x).toBeUndefined();
  });

  // `Model::name` is not unique on the Rust side, so the wire can repeat one.
  it("resolves a repeated model name to its first entry", () => {
    const point = stateFrom([
      { model: "panel_srp", torque_body_nm: [1, 2, 3] },
      { model: "panel_srp", torque_body_nm: [9, 9, 9] },
    ]);

    expect(point.torque_panel_srp_x).toBe(1);
  });

  it("passes over a model no chart names", () => {
    const point = stateFrom([{ model: "some_future_model", torque_body_nm: [1, 2, 3] }]);

    expect(Object.keys(point).filter((k) => k.startsWith("torque_"))).toEqual([]);
  });

  it("carries no torque columns when the server sends none", () => {
    const onState = vi.fn();
    dispatchServerMessage(baseState as ServerMessage, { ...baseCallbacks, onState });
    const point = onState.mock.calls[0][0];

    expect(Object.keys(point).filter((k) => k.startsWith("torque_"))).toEqual([]);
  });
});
