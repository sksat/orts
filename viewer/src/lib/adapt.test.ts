import { describe, expect, it } from "vitest";
import { toOrbitPoint, toTrailBuffer, trailPointToOrbitPoint } from "./adapt.js";
import type { SatelliteState, TrailPoint } from "./types.js";

describe("toOrbitPoint", () => {
  it("maps position and stamps the given time", () => {
    const sat: SatelliteState = { id: "sat-1", position: [7000, 0, 100] };
    const p = toOrbitPoint(sat, 42);
    expect([p.x, p.y, p.z]).toEqual([7000, 0, 100]);
    expect(p.t).toBe(42);
  });

  it("defaults velocity to zero when omitted, carries it when present", () => {
    expect(
      [toOrbitPoint({ id: "s", position: [1, 2, 3] }, 0)].map((p) => [p.vx, p.vy, p.vz]),
    ).toEqual([[0, 0, 0]]);
    const p = toOrbitPoint({ id: "s", position: [0, 0, 0], velocity: [1, -2, 3] }, 0);
    expect([p.vx, p.vy, p.vz]).toEqual([1, -2, 3]);
  });

  it("maps a scalar-first attitude quaternion onto qw,qx,qy,qz", () => {
    const p = toOrbitPoint({ id: "s", position: [0, 0, 0], attitude: [0.1, 0.2, 0.3, 0.4] }, 0);
    expect([p.qw, p.qx, p.qy, p.qz]).toEqual([0.1, 0.2, 0.3, 0.4]);
  });

  it("leaves attitude undefined when not provided", () => {
    expect(toOrbitPoint({ id: "s", position: [0, 0, 0] }, 0).qw).toBeUndefined();
  });
});

describe("trailPointToOrbitPoint", () => {
  it("uses the point's own time when present", () => {
    const tp: TrailPoint = { position: [1, 2, 3], time: 99 };
    const p = trailPointToOrbitPoint(tp, 5);
    expect([p.x, p.y, p.z]).toEqual([1, 2, 3]);
    expect(p.t).toBe(99);
  });

  it("falls back to the index when the point has no time", () => {
    const p = trailPointToOrbitPoint({ position: [0, 0, 0] }, 7);
    expect(p.t).toBe(7);
  });
});

describe("toTrailBuffer", () => {
  it("preserves trail point positions in order", () => {
    const trail: TrailPoint[] = [
      { position: [1, 0, 0] },
      { position: [2, 0, 0] },
      { position: [3, 0, 0] },
    ];
    const buf = toTrailBuffer(trail);
    expect(buf.length).toBe(3);
    expect(buf.getAll().map((p) => p.x)).toEqual([1, 2, 3]);
  });

  it("carries real per-point times (needed for body-fixed trails)", () => {
    const buf = toTrailBuffer([
      { position: [0, 0, 0], time: 100 },
      { position: [0, 0, 1], time: 160 },
    ]);
    expect(buf.getAll().map((p) => p.t)).toEqual([100, 160]);
  });
});
