/**
 * Resolve the public {@link ViewerReferenceFrame} into the wiring the renderer
 * primitives consume: an internal `ReferenceFrame`, the frame-centre's ECI
 * position, and (for local-orbital views) the LVLH axes.
 *
 * Pure and renderer-agnostic so the frame logic can be unit tested without a
 * canvas — and so the same kernel can back both the library viewer and the app's
 * richer Scene later.
 */

import type { ReferenceFrame } from "../referenceFrame.js";
import { computeLvlhAxes, type LvlhAxes } from "../sceneFrame.js";
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

  // Satellite-centred. The internal frame always uses "inertial" orientation:
  // LVLH is driven by the presence of `lvlhAxes`, not the orientation field.
  const id = frame.center.satelliteId;
  const sat = getSatellite(id);
  const originPosition = sat ? ([...sat.position] as [number, number, number]) : null;

  let lvlhAxes: LvlhAxes | null = null;
  let localOrbitalFallback = false;
  if (frame.orientation === "localOrbital") {
    lvlhAxes = sat ? computeLvlhAxes(sat.position, sat.velocity ?? null) : null;
    // Flag the fallback only when we actually had a satellite to frame on.
    if (lvlhAxes === null && sat) localOrbitalFallback = true;
  }

  return {
    referenceFrame: { center: { type: "satellite", id }, orientation: "inertial" },
    originPosition,
    lvlhAxes,
    bodyFixed: false,
    localOrbitalFallback,
  };
}
