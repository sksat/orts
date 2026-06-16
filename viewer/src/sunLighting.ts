/**
 * Pure sun-lighting math, separated from the React/Three scene so it can be
 * unit-tested without a renderer or the arika WASM module.
 *
 * The WASM calls (sun direction/distance from a body) and Three.js wiring live
 * in the {@link useSunLighting} hook; everything here is a plain function of
 * its inputs.
 */
import { rotateZ } from "./frameTransform.js";
import type { LvlhAxes } from "./sceneFrame.js";

/** One astronomical unit in kilometres. */
export const AU_KM = 149_597_870.7;

/** Display-frame transform options for the sun direction. */
export interface SunDisplayFrameOptions {
  /** True when the display frame is Earth-fixed (ECEF). */
  isEcef: boolean;
  /** Earth rotation angle (radians); null/undefined when unavailable. */
  era: number | null | undefined;
  /** True when LVLH (local-orbital) rotation is applied to the data. */
  lvlhActive: boolean;
  /** LVLH basis when active; null falls back to the inertial/ECEF branch. */
  lvlhAxes: LvlhAxes | null;
}

/**
 * Transform a sun direction from the body-centred inertial frame (ECI) into the
 * active display frame.
 *
 * - LVLH (`lvlhActive` with axes): project onto the LVLH basis
 *   `[inTrack, crossTrack, radial]` so the sun tracks the satellite body frame.
 * - ECEF (`isEcef`, `era` given): rotate by `-era` about Z to match Earth-fixed.
 * - Otherwise: return the ECI direction unchanged.
 *
 * Works on and returns `[x, y, z]` tuples; the caller wraps the result in a
 * `THREE.Vector3` at the React boundary.
 */
export function sunDirectionInDisplayFrame(
  sunEci: [number, number, number],
  { isEcef, era, lvlhActive, lvlhAxes }: SunDisplayFrameOptions,
): [number, number, number] {
  const [sx, sy, sz] = sunEci;

  if (lvlhActive && lvlhAxes) {
    const { inTrack, crossTrack, radial } = lvlhAxes;
    return [
      inTrack[0] * sx + inTrack[1] * sy + inTrack[2] * sz,
      crossTrack[0] * sx + crossTrack[1] * sy + crossTrack[2] * sz,
      radial[0] * sx + radial[1] * sy + radial[2] * sz,
    ];
  }

  if (!isEcef || era == null) return [sx, sy, sz];

  // ECEF: rotate the sun direction by -ERA to match the Earth-fixed frame.
  return rotateZ(sx, sy, sz, -era);
}

/**
 * Sun illumination scale from the inverse-square law, normalised to 1.0 at 1 AU.
 *
 * Guards against non-positive distances (which never occur for a real body-Sun
 * distance) so a bad input can't produce Infinity/NaN intensity.
 */
export function inverseSquareIntensity(distKm: number): number {
  if (!(distKm > 0)) return 1.0;
  return (AU_KM / distKm) ** 2;
}
