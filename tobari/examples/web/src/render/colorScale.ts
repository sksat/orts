/**
 * Color scale utilities for heatmap rendering.
 *
 * Provides a viridis-like perceptually uniform palette and
 * linear/logarithmic normalization.
 */

/** A color scale maps a normalized [0,1] value to [r, g, b] in [0,255]. */
export type ColorScale = (t: number) => [number, number, number];

/** Viridis-like palette (simplified 16-stop LUT). */
const VIRIDIS_STOPS: [number, number, number][] = [
  [68, 1, 84],
  [72, 26, 108],
  [71, 47, 125],
  [65, 68, 135],
  [57, 86, 140],
  [49, 104, 142],
  [42, 120, 142],
  [35, 136, 141],
  [31, 152, 139],
  [34, 168, 132],
  [53, 183, 121],
  [83, 197, 104],
  [122, 209, 81],
  [168, 219, 52],
  [218, 226, 20],
  [253, 231, 37],
];

/** Interpolate viridis palette. `t` should be in [0, 1]. */
export const viridis: ColorScale = (t: number) => {
  const clamped = Math.max(0, Math.min(1, t));
  const idx = clamped * (VIRIDIS_STOPS.length - 1);
  const lo = Math.floor(idx);
  const hi = Math.min(lo + 1, VIRIDIS_STOPS.length - 1);
  const frac = idx - lo;
  const a = VIRIDIS_STOPS[lo];
  const b = VIRIDIS_STOPS[hi];
  return [
    Math.round(a[0] + (b[0] - a[0]) * frac),
    Math.round(a[1] + (b[1] - a[1]) * frac),
    Math.round(a[2] + (b[2] - a[2]) * frac),
  ];
};

/** Diverging blue-white-red palette for signed values. */
export const diverging: ColorScale = (t: number) => {
  const clamped = Math.max(0, Math.min(1, t));
  if (clamped < 0.5) {
    const s = clamped * 2; // 0..1
    return [Math.round(30 + 225 * s), Math.round(60 + 195 * s), Math.round(180 + 75 * s)];
  }
  const s = (clamped - 0.5) * 2; // 0..1
  return [Math.round(255 - 25 * s), Math.round(255 - 215 * s), Math.round(255 - 230 * s)];
};

/** Normalize a value linearly to [0, 1]. */
export function linearNorm(value: number, min: number, max: number): number {
  if (max <= min) return 0;
  return (value - min) / (max - min);
}

/** Normalize a value logarithmically to [0, 1]. */
export function logNorm(value: number, min: number, max: number): number {
  if (value <= 0 || min <= 0 || max <= min) return 0;
  const logMin = Math.log10(min);
  const logMax = Math.log10(max);
  if (logMax <= logMin) return 0;
  return (Math.log10(value) - logMin) / (logMax - logMin);
}
