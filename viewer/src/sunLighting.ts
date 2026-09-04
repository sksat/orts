/**
 * Pure sun-lighting math, separated from the React/Three scene so it can be
 * unit-tested without a renderer or the arika WASM module.
 *
 * The WASM calls (sun direction/distance from a body) and Three.js wiring live
 * in the {@link useSunLighting} hook; everything here is a plain function of
 * its inputs. The sun direction's display-frame transform is not here: it is
 * `displayDirection` in displayFrame.ts, shared with every other direction the
 * scene draws so they cannot end up in different bases.
 */

/** One astronomical unit in kilometres. */
export const AU_KM = 149_597_870.7;

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
