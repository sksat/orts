import { describe, expect, it } from "vitest";
import { attitudeWasRefused, sampleAttitude } from "../displayFrame.js";
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
    expect(sampleAttitude(point)).toEqual([1, 0, 0, 0]);
    expect(attitudeWasRefused(point)).toBe(false);
  });

  it("leaves a row with no attitude column without one", () => {
    const point = rowToPoint(row());
    expect([point.qw, point.qx, point.qy, point.qz]).toEqual([
      undefined,
      undefined,
      undefined,
      undefined,
    ]);
    expect(attitudeWasRefused(point)).toBe(false);
  });

  it("carries a non-finite quaternion through as an attitude to refuse", () => {
    // The reachable malformed case: the decoder yields four components or none, so
    // a row cannot arrive partly decoded — but a complete tuple can hold `NaN`,
    // which a diverged simulation writes. It has to stay a claim, because a
    // satellite read as having no attitude keeps its registered 3D model, drawn at
    // the model's own orientation with the scene scaled to it.
    const point = rowToPoint(row({ quaternion: [Number.NaN, 0, 0, 0] }));
    expect(sampleAttitude(point)).toBeUndefined();
    expect(attitudeWasRefused(point)).toBe(true);
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
