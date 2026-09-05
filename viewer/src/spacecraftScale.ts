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
import type { MarkerShape } from "./satelliteShapes.js";

/** Apparent size of the spacecraft in a scene that has no other length scale. */
export const NOMINAL_SPACECRAFT_SPAN = 1;

/**
 * Marker half-extents in the orbit view, whose scene unit is the central body's
 * radius. Here rather than in the marker components because the *span* they
 * imply is what the axes and arrows are sized from.
 */
export const DEFAULT_SPHERE_RADIUS = 0.005;
export const DEFAULT_CUBE_HALF_EXTENT = 0.008;

/** Marker half-extent for a shape, in the orbit view's scene units. */
export function defaultMarkerSize(shape: MarkerShape): number {
  return shape === "sphere" ? DEFAULT_SPHERE_RADIUS : DEFAULT_CUBE_HALF_EXTENT;
}

/** Body-axis length, as a ratio of the spacecraft's apparent size. */
const AXIS_LENGTH_RATIO = 0.75;

/** Reference-frame axis length. Longer than the body axes so the two read apart. */
const FRAME_AXIS_LENGTH_RATIO = 2;

/**
 * Arrow proportions, as ratios of the spacecraft's apparent size.
 *
 * An arrow starts at the surface of what is drawn — see
 * {@link markerBoundingRadius} — because starting at the centre buries its first
 * half inside the spacecraft.
 */
const ARROW_LENGTH_RATIO = 1.5;
const ARROW_SHAFT_RADIUS_RATIO = 0.015;
const ARROW_HEAD_LENGTH_RATIO = 0.2;
const ARROW_HEAD_RADIUS_RATIO = 0.06;

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

/**
 * Radius of the sphere that encloses the drawn spacecraft, which is where an
 * arrow can start without beginning inside it.
 *
 * The sphere marker's radius is half the span. The cube marker is a span on a
 * side, so its corners stand at `sqrt(3)/2` of the span — half a span clears its
 * faces and not its corners, and an arrow along a body diagonal emerged partway.
 * A registered 3D model has no bounding radius here; the span is its largest
 * extent, so half of it is the closest thing available.
 */
export function markerBoundingRadius(shape: MarkerShape | null, span: number): number {
  return shape === "axes-cube" ? (Math.sqrt(3) / 2) * span : span / 2;
}

/**
 * Bounding radius of a 3D model of this apparent size.
 *
 * A registered model is known by one number, its largest extent, so all that can
 * be said is that it fits inside a cube of that side — and for a model whose
 * bounds are centred on its own origin, which the registered ones are, that
 * cube's envelope bounds it. `nativeSpanUnits` records a size and not a radius,
 * so an off-centre model would defeat this: bounds of `x = [10, 11]` are one unit
 * across and ten units out. Half the span, on the other hand, would put an
 * arrow's tail inside any model with extent on more than one axis, which is most
 * of them.
 *
 * TODO: measure each model's own bounding radius the way `nativeSpanUnits` is
 * measured, which would state the centring rather than assume it. The envelope
 * also over-corrects for a flat model — a panel reaches sqrt(2)/2 of its span,
 * not sqrt(3)/2 — which shows as a gap between the model and the tail of an arrow
 * along one of its long axes.
 */
export function modelBoundingRadius(span: number): number {
  return (Math.sqrt(3) / 2) * span;
}

/**
 * Arrow proportions for a spacecraft of this apparent size.
 *
 * `startRadius` is where the arrow's tail sits; pass
 * {@link markerBoundingRadius} for the shape actually drawn. The default is the
 * sphere's, the smallest of them.
 */
export function arrowGeometryForSpan(span: number, startRadius = span / 2): ArrowGeometry {
  return {
    length: span * ARROW_LENGTH_RATIO,
    shaftRadius: span * ARROW_SHAFT_RADIUS_RATIO,
    headLength: span * ARROW_HEAD_LENGTH_RATIO,
    headRadius: span * ARROW_HEAD_RADIUS_RATIO,
    startOffset: startRadius,
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

/** Narrowest field of view the camera fit still returns a usable distance for. */
const MIN_CAMERA_FOV_DEGREES = 0.1;

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
  if (fovDegrees == null || !Number.isFinite(fovDegrees) || fovDegrees <= 0 || fovDegrees >= 180) {
    return DEFAULT_CAMERA_FOV_DEGREES;
  }
  // A narrow view is a legitimate request — a telescope framing of the same
  // spacecraft — so it is clamped rather than replaced. The floor is where a
  // *camera* stops being one: the distance fitted for it goes as 1/sin(fov/2),
  // so 1e-300 degrees asks for 3e302 spans, a finite number and no use to a
  // depth buffer, while a tenth of a degree asks for some 3000. A `zoom` narrows
  // the view further, and that is fitted as it stands — see
  // {@link cameraDistanceForSpan}.
  return Math.max(fovDegrees, MIN_CAMERA_FOV_DEGREES);
}

/** Empty space kept between the outermost drawn thing and the viewport edge. */
const VIEW_MARGIN = 1.15;

/**
 * How far past an arrow's tip its name sits, as a fraction of the arrow's length.
 * On the tip the text overlaps the head it names.
 */
const ARROW_LABEL_OVERSHOOT = 0.14;

/** Name height as a fraction of the spacecraft's apparent size. */
const ARROW_LABEL_SCALE = 0.22;

/** Distance from the arrow's tail group to where its name is drawn. */
export function arrowLabelDistance(geometry: ArrowGeometry): number {
  return geometry.startOffset + geometry.length * (1 + ARROW_LABEL_OVERSHOOT);
}

/** Name height in scene units for a spacecraft of this apparent size. */
export function arrowLabelHeight(span: number): number {
  return span * ARROW_LABEL_SCALE;
}

/**
 * Distance from the origin to the furthest thing drawn around the spacecraft:
 * whichever reaches further, the reference-frame triad or a direction arrow's tip.
 *
 * The triad's term is its *labels*, not its axes — the letters sit past the tips,
 * so fitting the axes alone crops them.
 */
export function drawnExtentForSpan(span: number): number {
  // The widest marker decides, so the framing holds whichever shape is drawn.
  const arrow = arrowGeometryForSpan(span, markerBoundingRadius("axes-cube", span));
  // Each arrow's name sits past its tip, as each axis letter sits past its own.
  const arrowReach = arrowLabelDistance(arrow) + arrowLabelHeight(span) / 2;
  return Math.max(axisLabelExtent(frameAxisLengthForSpan(span)), arrowReach);
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
  // A viewport with no width or height has no aspect to fit; treat it as square
  // rather than returning an infinite distance a caller would place a camera at.
  const usable = Number.isFinite(aspect) && aspect > 0 ? aspect : 1;
  // Any angle a frustum can have is fitted as asked, including one far narrower
  // than a camera prop may be — a zoom of 1000 leaves an *effective* fov of
  // 0.05°, and fitting that as though it were wider puts the camera too close and
  // clips the scene. What is rejected is an angle that is no angle, and a result
  // that comes back unusable: below roughly 1e-300 degrees the sine underflows
  // and the quotient is infinite.
  const angled = Number.isFinite(fovDegrees) && fovDegrees > 0 && fovDegrees < 180;
  const distance = angled
    ? fittedDistance(span, fovDegrees, usable)
    : fittedDistance(span, DEFAULT_CAMERA_FOV_DEGREES, usable);
  return Number.isFinite(distance) && distance > 0
    ? distance
    : fittedDistance(span, DEFAULT_CAMERA_FOV_DEGREES, 1);
}

/**
 * Camera distance for the opening view: far enough to fit the scene in the
 * viewport, and far enough that the near plane falls in front of it.
 *
 * Those are separate constraints. A `near` reaches the camera from a public prop,
 * chosen without knowing this view's scale, and one that sits past the scene is a
 * perfectly buildable frustum with nothing inside it: at the default field of
 * view the fit stands some seven spans off with the nearest drawn point four and
 * a half spans away, so a `near` of ten spans renders an empty canvas. Clearing
 * the sphere of radius {@link drawnExtentForSpan} takes `d >= near + R`, the same
 * sphere and the same reasoning the far plane is pushed out for.
 */
export function initialCameraDistance(
  span: number,
  fovDegrees: number,
  aspect: number,
  near: number,
): number {
  const fitted = cameraDistanceForSpan(span, fovDegrees, aspect);
  // A near plane that is not one imposes no constraint; the camera prop is
  // checked before it reaches the camera, so this only guards the arithmetic.
  const cleared = Number.isFinite(near) && near > 0 ? near + drawnExtentForSpan(span) : 0;
  return Math.max(fitted, cleared);
}

/** Distance that fits the drawn sphere at this field of view and aspect. */
function fittedDistance(span: number, fovDegrees: number, aspect: number): number {
  const halfVertical = (fovDegrees / 2) * (Math.PI / 180);
  const halfHorizontal = Math.atan(Math.tan(halfVertical) * aspect);
  const half = Math.min(halfVertical, halfHorizontal);
  return (drawnExtentForSpan(span) * VIEW_MARGIN) / Math.sin(half);
}
