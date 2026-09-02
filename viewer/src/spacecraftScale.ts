/**
 * How large a spacecraft is drawn, and the sizes derived from it.
 *
 * Body axes and direction arrows are sized as a fixed ratio of the spacecraft's
 * *apparent* size rather than held constant on screen: zooming then scales the
 * spacecraft and its annotations together, so the ratio a reader learns stays
 * true, and nothing has to measure the camera distance every frame.
 *
 * The two views feed this from different places. The orbit view's scene unit is
 * the central body's radius, so a spacecraft's apparent size comes from the
 * model scale the registry chose (or the fallback marker's size). The attitude
 * view has no central body, so it normalises the spacecraft to
 * {@link NOMINAL_SPACECRAFT_SPAN} and works back to a model scale.
 */

import type { SatelliteModelConfig } from "./satelliteModels.js";

/** Apparent size of the spacecraft in a scene that has no other length scale. */
export const NOMINAL_SPACECRAFT_SPAN = 1;

/** Body-axis length, as a ratio of the spacecraft's apparent size. */
const AXIS_LENGTH_RATIO = 0.75;

/** Reference-frame axis length. Longer than the body axes so the two read apart. */
const FRAME_AXIS_LENGTH_RATIO = 2;

/**
 * Arrow proportions, as ratios of the spacecraft's apparent size.
 *
 * `START_OFFSET` keeps an arrow's tail outside the spacecraft; starting at the
 * centre buries the first half of every arrow inside the model.
 */
const ARROW_LENGTH_RATIO = 1.5;
const ARROW_SHAFT_RADIUS_RATIO = 0.015;
const ARROW_HEAD_LENGTH_RATIO = 0.2;
const ARROW_HEAD_RADIUS_RATIO = 0.06;
const ARROW_START_OFFSET_RATIO = 0.5;

/**
 * Apparent size (largest extent) of a drawn spacecraft, in scene units.
 *
 * A model whose native span is unknown falls back to the marker's footprint:
 * the registry's `scale` alone says nothing about how large the result looks.
 */
export function resolveVisualSpan(opts: {
  modelConfig?: SatelliteModelConfig | null;
  /** Marker half-extent (sphere radius / cube half-side) in scene units. */
  markerSize: number;
}): number {
  const nativeSpan = opts.modelConfig?.nativeSpanUnits;
  if (opts.modelConfig != null && nativeSpan != null && nativeSpan > 0) {
    return opts.modelConfig.scale * nativeSpan;
  }
  return 2 * opts.markerSize;
}

/**
 * Model scale that draws `config` at `targetSpan` scene units.
 *
 * Null when the model's native span is unknown — the caller then has nothing to
 * normalise against and should keep the registry's own scale.
 */
export function spanNormalizedModelScale(
  config: SatelliteModelConfig,
  targetSpan: number,
): number | null {
  const nativeSpan = config.nativeSpanUnits;
  if (nativeSpan == null || !(nativeSpan > 0)) return null;
  return targetSpan / nativeSpan;
}

/** Body-axis length for a spacecraft of this apparent size. */
export function axisLengthForSpan(span: number): number {
  return span * AXIS_LENGTH_RATIO;
}

/** Reference-frame axis length for a scene whose spacecraft has this apparent size. */
export function frameAxisLengthForSpan(span: number): number {
  return span * FRAME_AXIS_LENGTH_RATIO;
}

/** Arrow proportions in scene units. */
export interface ArrowGeometry {
  /** Tip distance from the tail, head included. */
  length: number;
  shaftRadius: number;
  headLength: number;
  headRadius: number;
  /** Distance from the spacecraft centre to the arrow's tail. */
  startOffset: number;
}

/** Arrow proportions for a spacecraft of this apparent size. */
export function arrowGeometryForSpan(span: number): ArrowGeometry {
  return {
    length: span * ARROW_LENGTH_RATIO,
    shaftRadius: span * ARROW_SHAFT_RADIUS_RATIO,
    headLength: span * ARROW_HEAD_LENGTH_RATIO,
    headRadius: span * ARROW_HEAD_RADIUS_RATIO,
    startOffset: span * ARROW_START_OFFSET_RATIO,
  };
}

/** Empty space kept between the outermost drawn thing and the viewport edge. */
const VIEW_MARGIN = 1.15;

/**
 * Distance from the origin to the furthest thing drawn around the spacecraft:
 * whichever reaches further, the reference-frame axes or an arrow's tip.
 */
export function drawnExtentForSpan(span: number): number {
  const arrow = arrowGeometryForSpan(span);
  return Math.max(frameAxisLengthForSpan(span), arrow.startOffset + arrow.length);
}

/**
 * Camera distance that fits everything drawn inside the viewport.
 *
 * Fits the *sphere* of radius {@link drawnExtentForSpan}, not a flat disc at the
 * origin: the arrows point in arbitrary directions, and a point on the near side
 * of that sphere projects further out than the same radius at the origin's
 * depth. A sphere of radius `R` is inside a frustum of half-angle `θ` exactly
 * when `d·sin(θ) ≥ R`, which is where the `sin` comes from — using `tan` fits
 * only the origin plane and clips at wide fields of view (at 75° it puts a point
 * at the drawn extent past the edge).
 *
 * A perspective camera's `fov` is the *vertical* angle, so the horizontal half
 * angle shrinks with the aspect ratio and a portrait viewport clips sideways at a
 * distance that frames the same scene fine in landscape. `aspect` is
 * width / height; the narrower of the two directions is the binding one. Pass the
 * camera's *effective* field of view if it carries a `zoom`.
 *
 * Deriving the distance from what is actually drawn keeps the framing correct
 * when the proportions above change.
 */
export function cameraDistanceForSpan(span: number, fovDegrees: number, aspect = 1): number {
  const halfVertical = (fovDegrees / 2) * (Math.PI / 180);
  // A viewport with no width or height has no aspect to fit; treat it as square
  // rather than returning an infinite distance a caller would place a camera at.
  const usable = Number.isFinite(aspect) && aspect > 0 ? aspect : 1;
  const halfHorizontal = Math.atan(Math.tan(halfVertical) * usable);
  const half = Math.min(halfVertical, halfHorizontal);
  return (drawnExtentForSpan(span) * VIEW_MARGIN) / Math.sin(half);
}
