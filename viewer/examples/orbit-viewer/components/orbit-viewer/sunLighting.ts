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
