import { describe, expect, it } from "vitest";
import { resolveAttitude } from "../attitude.js";
import { torqueModelsOf } from "../orbit.js";
import { type RrdRowIn, rowToPoint } from "./rrdParseLogic.js";

function row(overrides: Partial<RrdRowIn> = {}): RrdRowIn {
  return {
    t: 0,
    x: 7000,
    y: 0,
    z: 0,
    vx: 0,
    vy: 7.546,
    vz: 0,
    entity_path: "/world/sat/one",
    ...overrides,
  };
}

describe("rowToPoint", () => {
  it("carries a complete quaternion through unchanged", () => {
    const point = rowToPoint(row({ quaternion: [1, 0, 0, 0] }));
    expect([point.qw, point.qx, point.qy, point.qz]).toEqual([1, 0, 0, 0]);
    const attitude = resolveAttitude(point);
    expect(attitude.kind).toBe("usable");
    expect(attitude.kind === "usable" && attitude.quaternion).toEqual([1, 0, 0, 0]);
  });

  it("leaves a row with no attitude column without one", () => {
    const point = rowToPoint(row());
    expect([point.qw, point.qx, point.qy, point.qz]).toEqual([
      undefined,
      undefined,
      undefined,
      undefined,
    ]);
    expect(resolveAttitude(point).kind).toBe("absent");
  });

  it("carries a non-finite quaternion through as an attitude to refuse", () => {
    // The reachable malformed case: the decoder yields four components or none, so
    // a row cannot arrive partly decoded — but a complete tuple can hold `NaN`,
    // which a diverged simulation writes. It has to stay a claim, because a
    // satellite read as having no attitude keeps its registered 3D model, drawn at
    // the model's own orientation with the scene scaled to it.
    const point = rowToPoint(row({ quaternion: [Number.NaN, 0, 0, 0] }));
    expect(resolveAttitude(point).kind).toBe("refused");
  });

  it("copies the angular velocity when the row carries one", () => {
    const point = rowToPoint(row({ angular_velocity: [0.1, 0.2, 0.3] }));
    expect([point.wx, point.wy, point.wz]).toEqual([0.1, 0.2, 0.3]);
    expect(rowToPoint(row()).wx).toBeUndefined();
  });

  it("passes the state and the entity path straight through", () => {
    const point = rowToPoint(row({ t: 42, x: 1, y: 2, z: 3, vx: 4, vy: 5, vz: 6 }));
    expect([point.t, point.x, point.y, point.z]).toEqual([42, 1, 2, 3]);
    expect([point.vx, point.vy, point.vz]).toEqual([4, 5, 6]);
    expect(point.entityPath).toBe("/world/sat/one");
  });
});

describe("torque columns from a recording", () => {
  const row = {
    t: 10,
    x: 6778,
    y: 0,
    z: 0,
    vx: 0,
    vy: 7.669,
    vz: 0,
    entity_path: "/world/sat/sat-a",
  };

  it("flattens each model's triple into the columns the charts name", () => {
    const point = rowToPoint({
      ...row,
      torque_gravity_gradient: [1e-5, -2e-5, 3e-5],
      torque_panel_srp: [4e-7, 5e-7, 6e-7],
    });

    expect(point.torque_gravity_gradient_x).toBe(1e-5);
    expect(point.torque_gravity_gradient_y).toBe(-2e-5);
    expect(point.torque_gravity_gradient_z).toBe(3e-5);
    expect(point.torque_panel_srp_z).toBe(6e-7);
  });

  // The decoder reports a torque only where all three axes were logged, so
  // there is nothing to carry for a model it left out.
  it("leaves a model the recording does not carry unset", () => {
    const point = rowToPoint({ ...row, torque_panel_drag: null });

    expect(point.torque_panel_drag_x).toBeUndefined();
    expect(point.torque_gravity_gradient_x).toBeUndefined();
  });

  it("keeps a torque reported as zero", () => {
    const point = rowToPoint({ ...row, torque_panel_drag: [0, 0, 0] });

    expect(point.torque_panel_drag_x).toBe(0);
  });
});

describe("torqueModelsOf", () => {
  function point(entityPath: string, columns: Record<string, number>) {
    return {
      t: 0,
      x: 6778,
      y: 0,
      z: 0,
      vx: 0,
      vy: 7.669,
      vz: 0,
      a: 6778,
      e: 0,
      inc: 0,
      raan: 0,
      omega: 0,
      nu: 0,
      entityPath,
      ...columns,
    };
  }

  // A recording's columns are the union over its satellites, so what a given
  // satellite carries is decided from the values decoded for it.
  it("reports a model per satellite, not per recording", () => {
    const models = torqueModelsOf([
      point("/world/sat/sat-a", {
        torque_gravity_gradient_x: 1e-5,
        torque_gravity_gradient_y: 0,
        torque_gravity_gradient_z: 0,
      }),
      point("/world/sat/sat-b", {
        torque_panel_srp_x: 1e-7,
        torque_panel_srp_y: 0,
        torque_panel_srp_z: 0,
      }),
    ]);

    expect([...(models.get("/world/sat/sat-a") ?? [])]).toEqual(["gravity_gradient"]);
    expect([...(models.get("/world/sat/sat-b") ?? [])]).toEqual(["panel_srp"]);
  });

  it("does not report a model from a partial triple", () => {
    const models = torqueModelsOf([
      point("/world/sat/sat-a", { torque_panel_drag_x: 1e-9, torque_panel_drag_y: 0 }),
    ]);

    expect(models.get("/world/sat/sat-a")).toBeUndefined();
  });

  it("reports a model that measured zero", () => {
    const models = torqueModelsOf([
      point("/world/sat/sat-a", {
        torque_panel_drag_x: 0,
        torque_panel_drag_y: 0,
        torque_panel_drag_z: 0,
      }),
    ]);

    expect([...(models.get("/world/sat/sat-a") ?? [])]).toEqual(["panel_drag"]);
  });
});
