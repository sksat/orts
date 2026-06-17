import { resolveSceneFrame } from "../frameResolve.js";
import type { ReferenceFrame } from "../referenceFrame.js";
import type { LvlhAxes } from "../sceneFrame.js";
import type { Vec3, ViewerReferenceFrame } from "./types.js";

/** Minimal satellite state the frame resolver needs. */
export interface FrameSatellite {
  position: Vec3;
  velocity?: Vec3;
}

/** Look up a satellite's current state by id (returns undefined if absent). */
export type FrameSatelliteLookup = (satelliteId: string) => FrameSatellite | undefined;

/** Everything the primitives need to render in the requested frame. */
export interface FrameContext {
  /** Internal reference frame handed to Satellite/OrbitTrail/CelestialBody. */
  referenceFrame: ReferenceFrame;
  /** ECI position [km] of the frame centre, or null when the central body is centred. */
  originPosition: [number, number, number] | null;
  /** LVLH axes when local-orbital framing is active, else null. */
  lvlhAxes: LvlhAxes | null;
  /** True when the central body is rendered body-fixed (ECEF-like). */
  bodyFixed: boolean;
  /** True when `localOrbital` was requested but velocity was unavailable. */
  localOrbitalFallback: boolean;
}

export function resolveFrameContext(
  frame: ViewerReferenceFrame,
  getSatellite: FrameSatelliteLookup,
): FrameContext {
  if (frame.center === "centralBody") {
    const bodyFixed = frame.orientation === "bodyFixed";
    return {
      referenceFrame: {
        center: { type: "central_body" },
        orientation: bodyFixed ? "body_fixed" : "inertial",
      },
      originPosition: null,
      lvlhAxes: null,
      bodyFixed,
      localOrbitalFallback: false,
    };
  }

  // Satellite-centred: map the public orientation onto the internal one and
  // let the shared kernel resolve the geometry. The public API has no body
  // entities, so the body predicate is constantly false.
  const localOrbital = frame.orientation === "localOrbital";
  const referenceFrame: ReferenceFrame = {
    center: { type: "satellite", id: frame.center.satelliteId },
    orientation: localOrbital ? "local_orbital" : "inertial",
  };
  const ctx = resolveSceneFrame(
    referenceFrame,
    (id) => {
      const sat = getSatellite(id);
      return sat ? { position: sat.position, velocity: sat.velocity ?? null } : null;
    },
    () => false,
  );

  return {
    referenceFrame,
    originPosition: ctx.originPosition,
    lvlhAxes: ctx.lvlhAxes,
    bodyFixed: false,
    // LVLH was requested but the axes weren't computable (no velocity).
    localOrbitalFallback: localOrbital && !ctx.lvlhActive && ctx.originPosition != null,
  };
}
