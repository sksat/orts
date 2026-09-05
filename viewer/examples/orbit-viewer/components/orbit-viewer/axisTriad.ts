export const AXIS_LETTERS = ["X", "Y", "Z"] as const;

/**
 * Red / green / blue by the usual convention, lightened: at the pure hues the
 * blue axis is nearly black against a dark scene, and a letter is a thinner mark
 * than a line so it reads darker still.
 */
export const AXIS_COLORS = [0xff4d4d, 0x4dff4d, 0x4d8cff] as const;

/** Unit directions, matching `AXIS_LETTERS`. */
export const AXIS_DIRECTIONS: readonly [number, number, number][] = [
  [1, 0, 0],
  [0, 1, 0],
  [0, 0, 1],
];

/**
 * How far past the tip a letter sits, as a fraction of the axis length. On the
 * tip it overlaps the arrow head it names; far past it reads as its own object.
 */
const TIP_OVERSHOOT = 1.16;

/** Letter size as a fraction of the axis length. */
const LABEL_SCALE = 0.3;

/** Where each letter sits, in the triad's own frame. */
export function axisLabelPositions(length: number): [number, number, number][] {
  const d = length * TIP_OVERSHOOT;
  return [
    [d, 0, 0],
    [0, d, 0],
    [0, 0, d],
  ];
}

/** Sprite size for a triad of this length. */
export function axisLabelScale(length: number): number {
  return length * LABEL_SCALE;
}

/**
 * Outermost point the labels of a triad reach, from the origin.
 *
 * The letters sit *past* the tips, so a camera fitted to the axes alone clips
 * them. A sprite is a quad facing the camera, so it reaches half its diagonal in
 * whichever direction the view happens to put that — and a letter's texture is
 * square, which makes the diagonal `sqrt(2)` times its side.
 */
export function axisLabelExtent(length: number): number {
  return length * TIP_OVERSHOOT + (axisLabelScale(length) * Math.SQRT2) / 2;
}

/** CSS colour for a canvas context, from one of `AXIS_COLORS`. */
export function axisColorCss(color: number): string {
  return `#${color.toString(16).padStart(6, "0")}`;
}
