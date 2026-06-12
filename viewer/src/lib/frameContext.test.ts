import { describe, expect, it } from "vitest";
import { resolveFrameContext } from "./frameContext.js";
import type { Vec3 } from "./types.js";

const sat = (position: Vec3, velocity?: Vec3) => ({ position, velocity });
const lookup = (map: Record<string, { position: Vec3; velocity?: Vec3 }>) => (id: string) =>
  map[id];

describe("resolveFrameContext — central body", () => {
  it("maps inertial to ECI (central body at origin, no LVLH)", () => {
    const ctx = resolveFrameContext(
      { center: "centralBody", orientation: "inertial" },
      () => undefined,
    );
    expect(ctx.referenceFrame).toEqual({
      center: { type: "central_body" },
      orientation: "inertial",
    });
    expect(ctx.originPosition).toBeNull();
    expect(ctx.lvlhAxes).toBeNull();
    expect(ctx.bodyFixed).toBe(false);
  });

  it("defaults missing orientation to inertial", () => {
    const ctx = resolveFrameContext({ center: "centralBody" }, () => undefined);
    expect(ctx.bodyFixed).toBe(false);
    expect(ctx.referenceFrame.orientation).toBe("inertial");
  });

  it("maps bodyFixed to ECEF", () => {
    const ctx = resolveFrameContext(
      { center: "centralBody", orientation: "bodyFixed" },
      () => undefined,
    );
    expect(ctx.referenceFrame).toEqual({
      center: { type: "central_body" },
      orientation: "body_fixed",
    });
    expect(ctx.bodyFixed).toBe(true);
  });
});

describe("resolveFrameContext — satellite centred", () => {
  const get = lookup({ a: sat([7000, 0, 0], [0, 7.5, 0]) });

  it("inertial: origin at the satellite, no LVLH axes, internal orientation inertial", () => {
    const ctx = resolveFrameContext({ center: { satelliteId: "a" }, orientation: "inertial" }, get);
    expect(ctx.referenceFrame).toEqual({
      center: { type: "satellite", id: "a" },
      orientation: "inertial",
    });
    expect(ctx.originPosition).toEqual([7000, 0, 0]);
    expect(ctx.lvlhAxes).toBeNull();
    expect(ctx.localOrbitalFallback).toBe(false);
  });

  it("localOrbital: maps to the internal local_orbital orientation (#90)", () => {
    const ctx = resolveFrameContext(
      { center: { satelliteId: "a" }, orientation: "localOrbital" },
      get,
    );
    expect(ctx.referenceFrame).toEqual({
      center: { type: "satellite", id: "a" },
      orientation: "local_orbital",
    });
  });

  it("localOrbital: computes LVLH axes (radial = normalized position)", () => {
    const ctx = resolveFrameContext(
      { center: { satelliteId: "a" }, orientation: "localOrbital" },
      get,
    );
    expect(ctx.originPosition).toEqual([7000, 0, 0]);
    expect(ctx.lvlhAxes).not.toBeNull();
    expect(ctx.lvlhAxes?.radial[0]).toBeCloseTo(1);
    expect(ctx.localOrbitalFallback).toBe(false);
  });

  it("localOrbital without velocity falls back to inertial (axes null, flagged)", () => {
    const get2 = lookup({ a: sat([7000, 0, 0]) });
    const ctx = resolveFrameContext(
      { center: { satelliteId: "a" }, orientation: "localOrbital" },
      get2,
    );
    expect(ctx.originPosition).toEqual([7000, 0, 0]);
    expect(ctx.lvlhAxes).toBeNull();
    expect(ctx.localOrbitalFallback).toBe(true);
  });

  it("unknown satellite: no origin, no axes (graceful)", () => {
    const ctx = resolveFrameContext(
      { center: { satelliteId: "ghost" }, orientation: "localOrbital" },
      get,
    );
    expect(ctx.originPosition).toBeNull();
    expect(ctx.lvlhAxes).toBeNull();
  });
});
