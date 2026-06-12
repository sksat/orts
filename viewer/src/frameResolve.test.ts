import { describe, expect, it } from "vitest";
import { resolveSceneFrame } from "./frameResolve.js";
import type { ReferenceFrame } from "./referenceFrame.js";

type V3 = [number, number, number];
const entity = (map: Record<string, { position: V3; velocity: V3 | null }>) => (id: string) =>
  map[id] ?? null;
const noBody = () => false;

const SAT = { position: [7000, 0, 0] as V3, velocity: [0, 7.5, 0] as V3 };

describe("resolveSceneFrame — central body", () => {
  it("inertial: no origin, no axes, nothing active", () => {
    const f: ReferenceFrame = { center: { type: "central_body" }, orientation: "inertial" };
    const ctx = resolveSceneFrame(f, entity({}), noBody);
    expect(ctx).toEqual({
      centeredSatId: null,
      originPosition: null,
      originVelocity: null,
      lvlhAxes: null,
      lvlhActive: false,
      cameraTracking: false,
    });
  });

  it("body_fixed: same nulls (ECEF is handled by the primitives, not the frame context)", () => {
    const f: ReferenceFrame = { center: { type: "central_body" }, orientation: "body_fixed" };
    const ctx = resolveSceneFrame(f, entity({}), noBody);
    expect(ctx.lvlhActive).toBe(false);
    expect(ctx.originPosition).toBeNull();
  });
});

describe("resolveSceneFrame — satellite centred, local_orbital", () => {
  const f: ReferenceFrame = {
    center: { type: "satellite", id: "a" },
    orientation: "local_orbital",
  };

  it("with velocity: data-LVLH active (axes set, no camera tracking)", () => {
    const ctx = resolveSceneFrame(f, entity({ a: SAT }), noBody);
    expect(ctx.centeredSatId).toBe("a");
    expect(ctx.originPosition).toEqual([7000, 0, 0]);
    expect(ctx.originVelocity).toEqual([0, 7.5, 0]);
    expect(ctx.lvlhAxes?.radial[0]).toBeCloseTo(1);
    expect(ctx.lvlhActive).toBe(true);
    expect(ctx.cameraTracking).toBe(false); // rotation lives in the data, not the camera
  });

  it("without velocity: falls back to camera tracking (radial-up), data stays inertial", () => {
    const ctx = resolveSceneFrame(
      f,
      entity({ a: { position: [7000, 0, 0], velocity: null } }),
      noBody,
    );
    expect(ctx.originPosition).toEqual([7000, 0, 0]);
    expect(ctx.lvlhAxes).toBeNull();
    expect(ctx.lvlhActive).toBe(false);
    expect(ctx.cameraTracking).toBe(true);
  });

  it("centred on a body entity (e.g. moon): no data-LVLH, camera co-rotates", () => {
    const ctx = resolveSceneFrame(f, entity({ a: SAT }), () => true);
    expect(ctx.lvlhAxes).toBeNull();
    expect(ctx.lvlhActive).toBe(false);
    expect(ctx.cameraTracking).toBe(true);
  });

  it("unknown entity: inert (no origin, nothing active)", () => {
    const ctx = resolveSceneFrame(f, entity({}), noBody);
    expect(ctx.originPosition).toBeNull();
    expect(ctx.lvlhActive).toBe(false);
    expect(ctx.cameraTracking).toBe(false);
  });
});

describe("resolveSceneFrame — snapshot semantics", () => {
  it("returns copies of the caller-owned tuples (in-place mutation can't alias)", () => {
    const state = { position: [7000, 0, 0] as V3, velocity: [0, 7.5, 0] as V3 };
    const f: ReferenceFrame = {
      center: { type: "satellite", id: "a" },
      orientation: "local_orbital",
    };
    const ctx = resolveSceneFrame(f, () => state, noBody);
    state.position[0] = 9999; // caller reuses its array
    state.velocity[1] = -1;
    expect(ctx.originPosition).toEqual([7000, 0, 0]);
    expect(ctx.originVelocity).toEqual([0, 7.5, 0]);
  });
});

describe("resolveSceneFrame — satellite centred, inertial (#90)", () => {
  const f: ReferenceFrame = { center: { type: "satellite", id: "a" }, orientation: "inertial" };

  it("origin set but axes star-fixed: no LVLH transform, no camera tracking", () => {
    const ctx = resolveSceneFrame(f, entity({ a: SAT }), noBody);
    expect(ctx.centeredSatId).toBe("a");
    expect(ctx.originPosition).toEqual([7000, 0, 0]); // satellite at scene origin…
    expect(ctx.lvlhAxes).toBeNull(); // …but axes stay inertial
    expect(ctx.lvlhActive).toBe(false);
    expect(ctx.cameraTracking).toBe(false); // camera must NOT co-rotate with the orbit
  });

  it("inertial on a body entity: also offset-only", () => {
    const ctx = resolveSceneFrame(f, entity({ a: SAT }), () => true);
    expect(ctx.originPosition).toEqual([7000, 0, 0]);
    expect(ctx.lvlhActive).toBe(false);
    expect(ctx.cameraTracking).toBe(false);
  });
});
