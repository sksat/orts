import { describe, expect, it } from "vitest";
import { orbitPointToChartRow } from "./eventDispatcher.js";
import { ORBIT_DERIVED_STRIDE, packStates, toOrbitPoints } from "./rrdOrbitDerived.js";
import type { RrdPointOut } from "./rrdParseLogic.js";

function point(overrides: Partial<RrdPointOut> = {}): RrdPointOut {
  return {
    t: 0,
    x: 6778.137,
    y: 0,
    z: 0,
    vx: 0,
    vy: 7.6686,
    vz: 0,
    entityPath: "/world/sat/sat-1",
    ...overrides,
  };
}


describe("packStates", () => {
  it("lays out six values per point in order", () => {
    const packed = packStates([
      point({ x: 1, y: 2, z: 3, vx: 4, vy: 5, vz: 6 }),
      point({ x: 7, y: 8, z: 9, vx: 10, vy: 11, vz: 12 }),
    ]);
    expect(Array.from(packed)).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
  });

  it("packs nothing for no points", () => {
    expect(packStates([]).length).toBe(0);
  });
});

describe("toOrbitPoints", () => {
  /** One state's worth of derived values, distinct so a misplaced index shows. */
  const derived = [6778.137, 0.001, 0.9, 1.1, 1.2, 1.3, 400, -29.4, 51981, 7.6686];

  it("puts each derived value in its own field", () => {
    const [p] = toOrbitPoints([point()], derived);
    expect(p.a).toBe(6778.137);
    expect(p.e).toBe(0.001);
    expect(p.inc).toBe(0.9);
    expect(p.raan).toBe(1.1);
    expect(p.omega).toBe(1.2);
    expect(p.nu).toBe(1.3);
    expect(p.altitude).toBe(400);
    expect(p.specific_energy).toBe(-29.4);
    expect(p.angular_momentum).toBe(51981);
    expect(p.velocity_mag).toBe(7.6686);
  });

  it("keeps the state vector and attitude the decoder recovered", () => {
    const [p] = toOrbitPoints([point({ t: 10, qw: 1, qx: 0, wx: 0.01 })], derived);
    expect(p.t).toBe(10);
    expect(p.x).toBe(6778.137);
    expect(p.vy).toBe(7.6686);
    expect(p.entityPath).toBe("/world/sat/sat-1");
    expect(p.qw).toBe(1);
    expect(p.wx).toBe(0.01);
  });

  it("reads the nth point from the nth block", () => {
    const second = derived.map((v) => v * 2);
    const points = toOrbitPoints([point(), point({ t: 10 })], [...derived, ...second]);
    expect(points[1].a).toBe(6778.137 * 2);
    expect(points[1].t).toBe(10);
  });

  it("rejects a batch whose length does not match the points", () => {
    // A silent mismatch would shift every satellite's elements by one block.
    expect(() => toOrbitPoints([point(), point()], derived)).toThrow(/expected 20/);
  });

  /**
   * The chart row reads these four straight off `OrbitPoint`, so what arrives
   * here is what a state with no orbital plane reports. It stays non-finite
   * rather than becoming zero, because zero is a real reading — a circular
   * equatorial orbit sitting at the body's surface — and would be plotted as
   * one.
   */
  it("carries a state with no orbital plane through as non-finite", () => {
    const undefinedState = new Array(ORBIT_DERIVED_STRIDE).fill(Number.NaN);
    const [p] = toOrbitPoints([point({ vy: 0 })], undefinedState);
    expect(p.a).toBeNaN();
    expect(p.altitude).toBeNaN();

    const row = orbitPointToChartRow(p);
    expect(row.altitude).toBeNaN();
    expect(row.energy).toBeNaN();
    expect(row.angular_momentum).toBeNaN();
  });

  /**
   * The whole point of the change: these four used to be hardcoded zeros in the
   * adapter, and the chart row reads them directly rather than recomputing from
   * the state vector.
   */
  it("gives the chart row the derived values rather than zeros", () => {
    const [p] = toOrbitPoints([point()], derived);
    const row = orbitPointToChartRow(p);
    expect(row.altitude).toBe(400);
    expect(row.energy).toBe(-29.4);
    expect(row.angular_momentum).toBe(51981);
    expect(row.a).toBe(6778.137);
    // Angles reach the chart in degrees.
    expect(row.inc_deg).toBeCloseTo((0.9 * 180) / Math.PI, 9);
  });
});
