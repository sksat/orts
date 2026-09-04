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

import { axisLabelExtent } from "./axisTriad.js";
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
 * `START_OFFSET` is half the apparent size, which clears a sphere marker and the
 * faces of a cube one; starting at the centre buries the first half of every
 * arrow. A cube's *corner* reaches `sqrt(3)/2` of the span, so an arrow aimed
 * along a body diagonal still begins inside the marker and emerges partway.
 * Closing that needs a bounding radius per marker and per model: raising the
 * constant to the circumscribed radius would push every arrow away from every
 * spacecraft to clear the one shape that reaches furthest.
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

/**
 * Axis-arrow thickness, as ratios of the spacecraft's apparent size.
 *
 * Thickness comes from the spacecraft rather than from each triad's own length:
 * the reference-frame axes are the longer pair, and sizing their shafts by their
 * own length would make the subordinate triad the heaviest thing on screen.
 *
 * The body triad is the thickest annotation in the scene — it is what the view is
 * for. The reference frame is a background against which to read it, so its
 * arrows are slender; drawn at the body's thickness, six heavy arrows leave the
 * spacecraft between them hard to see.
 *
 * Heads are shorter and narrower than a direction arrow's, since three of them
 * meet the labels at the tips.
 */
const BODY_AXIS_SHAFT_RADIUS_RATIO = 0.02;
const BODY_AXIS_HEAD_LENGTH_RATIO = 0.12;
const BODY_AXIS_HEAD_RADIUS_RATIO = 0.05;
const FRAME_AXIS_SHAFT_RADIUS_RATIO = 0.008;
const FRAME_AXIS_HEAD_LENGTH_RATIO = 0.07;
const FRAME_AXIS_HEAD_RADIUS_RATIO = 0.026;

/**
 * Body-axis arrows: `length` sets the extent, `span` the thickness.
 *
 * The extent is passed in rather than derived, because the caller that draws the
 * body axes already owns their length (the orbit view sizes them from the model
 * scale, the attitude view from the span).
 */
export function bodyAxisArrows(length: number, span: number): ArrowGeometry {
  return {
    length,
    shaftRadius: span * BODY_AXIS_SHAFT_RADIUS_RATIO,
    headLength: span * BODY_AXIS_HEAD_LENGTH_RATIO,
    headRadius: span * BODY_AXIS_HEAD_RADIUS_RATIO,
    startOffset: 0,
  };
}

/** Reference-frame axis arrows, sized as {@link bodyAxisArrows} but slender. */
export function frameAxisArrows(length: number, span: number): ArrowGeometry {
  return {
    length,
    shaftRadius: span * FRAME_AXIS_SHAFT_RADIUS_RATIO,
    headLength: span * FRAME_AXIS_HEAD_LENGTH_RATIO,
    headRadius: span * FRAME_AXIS_HEAD_RADIUS_RATIO,
    startOffset: 0,
  };
}

/**
 * Vertical field of view the attitude view frames with, and the fallback when a
 * caller supplies one that no camera could use.
 */
export const DEFAULT_CAMERA_FOV_DEGREES = 50;

/**
 * A vertical field of view a perspective camera can actually project with, or the
 * default framing when the caller's value is not one.
 *
 * Outside (0°, 180°) there is no frustum: at 0 or NaN a projection matrix comes
 * out degenerate and the canvas is blank, and past 180° the horizontal half-angle
 * turns negative and any fit inverts. `fov` reaches the viewer from a public
 * camera prop, so both the camera and the distance derived for it read it through
 * here — sanitising only one of the two leaves a camera that cannot project at a
 * distance that looks right.
 */
export function usableFovDegrees(fovDegrees: number | undefined): number {
  return fovDegrees != null && Number.isFinite(fovDegrees) && fovDegrees > 0 && fovDegrees < 180
    ? fovDegrees
    : DEFAULT_CAMERA_FOV_DEGREES;
}

/** Empty space kept between the outermost drawn thing and the viewport edge. */
const VIEW_MARGIN = 1.15;

/**
 * Distance from the origin to the furthest thing drawn around the spacecraft:
 * whichever reaches further, the reference-frame triad or a direction arrow's tip.
 *
 * The triad's term is its *labels*, not its axes — the letters sit past the tips,
 * so fitting the axes alone crops them.
 */
export function drawnExtentForSpan(span: number): number {
  const arrow = arrowGeometryForSpan(span);
  return Math.max(axisLabelExtent(frameAxisLengthForSpan(span)), arrow.startOffset + arrow.length);
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
  const halfVertical = (usableFovDegrees(fovDegrees) / 2) * (Math.PI / 180);
  // A viewport with no width or height has no aspect to fit; treat it as square
  // rather than returning an infinite distance a caller would place a camera at.
  const usable = Number.isFinite(aspect) && aspect > 0 ? aspect : 1;
  const halfHorizontal = Math.atan(Math.tan(halfVertical) * usable);
  const half = Math.min(halfVertical, halfHorizontal);
  return (drawnExtentForSpan(span) * VIEW_MARGIN) / Math.sin(half);
}
