import { describe, expect, it } from "vitest";
import type { SatelliteModelConfig } from "./satelliteModels.js";
import {
  arrowGeometryForSpan,
  axisLengthForSpan,
  cameraDistanceForSpan,
  DEFAULT_CAMERA_FOV_DEGREES,
  drawnExtentForSpan,
  frameAxisLengthForSpan,
  initialCameraDistance,
  markerBoundingRadius,
  modelBoundingRadius,
  NOMINAL_SPACECRAFT_SPAN,
  resolveVisualSpan,
  spanNormalizedModelScale,
  usableFovDegrees,
} from "./spacecraftScale.js";

/** A model with a measured native span (the ISS entry's numbers). */
const MEASURED: SatelliteModelConfig = {
  modelUrl: "iss.glb",
  scale: 0.0003,
  rotation: [0, 0, 0],
  physicalSpanKm: 0.109,
  nativeSpanUnits: 111.99,
};

/** A model whose native span has not been measured. */
const UNMEASURED: SatelliteModelConfig = {
  modelUrl: "unknown.glb",
  scale: 0.0002,
  rotation: [0, 0, 0],
};

describe("resolveVisualSpan", () => {
  it("multiplies a measured model's native span by its scale", () => {
    expect(resolveVisualSpan({ modelConfig: MEASURED, markerSize: 0.008 })).toBeCloseTo(
      0.0003 * 111.99,
      12,
    );
  });

  it("puts the body axes' tips outside the spacecraft's own footprint", () => {
    // Axes buried inside the model show nothing. The contract is that the tips
    // clear the half-span, whatever the model's or marker's size.
    for (const modelConfig of [MEASURED, UNMEASURED, null]) {
      const span = resolveVisualSpan({ modelConfig, markerSize: 0.008 });
      expect(axisLengthForSpan(span)).toBeGreaterThan(span / 2);
    }
  });

  it("falls back to the marker footprint when the native span is unusable", () => {
    // The registry's `scale` alone says nothing about apparent size, so an
    // unmeasured model has to fall back to the marker it would be replaced by.
    for (const nativeSpanUnits of [undefined, 0, -1]) {
      const config = { ...UNMEASURED, nativeSpanUnits };
      expect(resolveVisualSpan({ modelConfig: config, markerSize: 0.008 })).toBeCloseTo(0.016, 12);
    }
  });

  it("uses the marker footprint when there is no model", () => {
    expect(resolveVisualSpan({ modelConfig: null, markerSize: 0.005 })).toBeCloseTo(0.01, 12);
    expect(resolveVisualSpan({ markerSize: 0.008 })).toBeCloseTo(0.016, 12);
  });
});

describe("spanNormalizedModelScale", () => {
  it("draws a measured model at the requested span", () => {
    const scale = spanNormalizedModelScale(MEASURED, NOMINAL_SPACECRAFT_SPAN);
    if (scale == null) throw new Error("expected a scale for a measured model");
    expect(resolveVisualSpan({ modelConfig: { ...MEASURED, scale }, markerSize: 1 })).toBeCloseTo(
      NOMINAL_SPACECRAFT_SPAN,
      12,
    );
  });

  it("returns null for an unmeasured model, leaving the registry's scale in place", () => {
    expect(spanNormalizedModelScale(UNMEASURED, 1)).toBeNull();
    expect(spanNormalizedModelScale({ ...UNMEASURED, nativeSpanUnits: 0 }, 1)).toBeNull();
    expect(spanNormalizedModelScale({ ...UNMEASURED, nativeSpanUnits: -3 }, 1)).toBeNull();
    expect(spanNormalizedModelScale({ ...UNMEASURED, nativeSpanUnits: Number.NaN }, 1)).toBeNull();
  });
});

describe("sizes derived from the span", () => {
  it("keeps the body axes inside the reference-frame axes", () => {
    // Both triads are RGB and share the origin, so the reader tells them apart by
    // length: the frame's axes must clearly outrun the body's.
    for (const span of [0.016, 1, 42]) {
      expect(axisLengthForSpan(span)).toBeLessThan(frameAxisLengthForSpan(span));
    }
  });

  it("scales every arrow proportion with the span", () => {
    const small = arrowGeometryForSpan(1);
    const large = arrowGeometryForSpan(10);
    for (const key of Object.keys(small) as (keyof typeof small)[]) {
      expect(large[key]).toBeCloseTo(small[key] * 10, 12);
    }
  });

  it("frames the drawn sphere, not just the plane through the origin", () => {
    // Independent check: project the worst-case point *on* the sphere of drawn
    // radius R, whose normalised device coordinate is
    // `R / sqrt(d² - R²) / tan(halfAngle)` — the silhouette of a sphere seen
    // under perspective, which sits further out than R at the origin's depth.
    // Fitting with `tan` alone passes the plane check and clips this one (at
    // fov 75 it reaches 1.167).
    for (const span of [0.016, 1, 42]) {
      for (const fov of [30, 50, 75]) {
        for (const aspect of [0.4, 1, 16 / 9]) {
          const R = drawnExtentForSpan(span);
          const d = cameraDistanceForSpan(span, fov, aspect);
          expect(d).toBeGreaterThan(R);
          const halfVertical = (fov / 2) * (Math.PI / 180);
          const halfHorizontal = Math.atan(Math.tan(halfVertical) * aspect);
          const silhouette = R / Math.sqrt(d * d - R * R);
          expect(silhouette / Math.tan(halfVertical)).toBeLessThan(1);
          expect(silhouette / Math.tan(halfHorizontal)).toBeLessThan(1);
        }
      }
    }
  });

  it("treats an unusable aspect as square rather than returning an infinite distance", () => {
    // A canvas with no width or height has no aspect to fit; a caller must not be
    // handed Infinity or NaN to place a camera at.
    const square = cameraDistanceForSpan(1, 50, 1);
    for (const aspect of [0, -2, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(cameraDistanceForSpan(1, 50, aspect)).toBeCloseTo(square, 12);
    }
  });

  it("does not pull the camera closer than the square fit on a wide viewport", () => {
    // A landscape viewport is already covered by the square fit; widening it must
    // not shrink the spacecraft by pushing the camera in.
    const square = cameraDistanceForSpan(1, 50, 1);
    expect(cameraDistanceForSpan(1, 50, 16 / 9)).toBeCloseTo(square, 12);
    expect(cameraDistanceForSpan(1, 50, 0.5)).toBeGreaterThan(square);
  });

  it("starts an arrow outside the shape actually drawn, corners included", () => {
    // The cube marker is a span on a side, so its corners stand at sqrt(3)/2 of
    // the span: an arrow that started at half the span began inside it and
    // emerged 0.366 span later, along every body diagonal.
    const span = 1;
    const sphere = arrowGeometryForSpan(span, markerBoundingRadius("sphere", span));
    const cube = arrowGeometryForSpan(span, markerBoundingRadius("axes-cube", span));
    expect(sphere.startOffset).toBeCloseTo(span / 2, 12);
    expect(cube.startOffset).toBeCloseTo((Math.sqrt(3) / 2) * span, 12);
    // A model has no bounding radius here, so it gets the sphere's.
    expect(markerBoundingRadius(null, span)).toBeCloseTo(span / 2, 12);
    // The arrow keeps its length; only its tail moves, so the tip follows.
    expect(cube.length).toBeCloseTo(sphere.length, 12);
    expect(cube.startOffset + cube.length).toBeGreaterThan(sphere.startOffset + sphere.length);
  });

  it("frames for the widest marker, so no shape is cropped", () => {
    const span = 1;
    const cube = arrowGeometryForSpan(span, markerBoundingRadius("axes-cube", span));
    expect(drawnExtentForSpan(span)).toBeGreaterThanOrEqual(cube.startOffset + cube.length);
  });

  it("treats a field of view no camera could use as the default framing", () => {
    // `fov` arrives from a public camera prop. At 0 or NaN the fit divides by a
    // sine of 0 or NaN; past 180° the horizontal term goes negative and the fit
    // inverts. Each case must land on the documented framing instead.
    const fallback = cameraDistanceForSpan(1, DEFAULT_CAMERA_FOV_DEGREES, 1);
    for (const fov of [0, -30, 180, 200, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(cameraDistanceForSpan(1, fov, 1)).toBeCloseTo(fallback, 12);
    }
    // A usable field of view is still honoured, in both directions.
    expect(cameraDistanceForSpan(1, 30, 1)).not.toBeCloseTo(fallback, 6);
    expect(cameraDistanceForSpan(1, 179, 1)).toBeLessThan(fallback);
  });

  it("fits any angle a frustum can have, however narrow", () => {
    // A `zoom` narrows the *effective* field of view below anything a camera prop
    // would be given — zoom 1000 on 50° leaves about 0.05° — and fitting that as
    // though it were wider puts the camera too close and clips the scene. So the
    // fit takes the angle as it stands and the distance grows with it.
    const wide = cameraDistanceForSpan(1, DEFAULT_CAMERA_FOV_DEGREES, 1);
    const narrow = cameraDistanceForSpan(1, 0.1, 1);
    const narrower = cameraDistanceForSpan(1, 0.05, 1);
    expect(narrow).toBeGreaterThan(wide * 100);
    expect(narrower).toBeGreaterThan(narrow * 1.9);
    expect(Number.isFinite(narrower)).toBe(true);
  });

  it("falls back when an angle leaves no distance to compute", () => {
    // Below roughly 1e-300 degrees the sine underflows and the quotient is
    // infinite; there is no camera placement to return, so the default framing is.
    const fallback = cameraDistanceForSpan(1, DEFAULT_CAMERA_FOV_DEGREES, 1);
    for (const fov of [Number.MIN_VALUE, 1e-320]) {
      const d = cameraDistanceForSpan(1, fov, 1);
      expect(Number.isFinite(d)).toBe(true);
      expect(d).toBeCloseTo(fallback, 12);
    }
  });

  it("keeps a camera's own field of view inside what a camera can project", () => {
    // The prop that reaches `PerspectiveCamera` is clamped, unlike the fit: a
    // projection matrix built from 1e-300 degrees is degenerate whatever distance
    // accompanies it.
    expect(usableFovDegrees(0.01)).toBeCloseTo(0.1, 12);
    expect(usableFovDegrees(1e-300)).toBeCloseTo(0.1, 12);
    expect(usableFovDegrees(50)).toBe(50);
    expect(usableFovDegrees(Number.NaN)).toBe(DEFAULT_CAMERA_FOV_DEGREES);
    expect(usableFovDegrees(0)).toBe(DEFAULT_CAMERA_FOV_DEGREES);
    expect(usableFovDegrees(180)).toBe(DEFAULT_CAMERA_FOV_DEGREES);
  });

  it("pulls further back for a narrower field of view", () => {
    expect(cameraDistanceForSpan(1, 30, 1)).toBeGreaterThan(cameraDistanceForSpan(1, 50, 1));
    expect(cameraDistanceForSpan(1, 50, 1)).toBeGreaterThan(cameraDistanceForSpan(1, 75, 1));
  });

  it("bounds a model by the cube its largest extent fits in", () => {
    const span = 1;
    // All a registered model reports is its largest extent, so it fits in a cube
    // of that side and the cube's corner is the furthest it can reach. Half the
    // span would sit inside any model with extent on more than one axis.
    expect(modelBoundingRadius(span)).toBeCloseTo((Math.sqrt(3) / 2) * span, 12);
    expect(modelBoundingRadius(span)).toBe(markerBoundingRadius("axes-cube", span));
    expect(modelBoundingRadius(span)).toBeGreaterThan(markerBoundingRadius("sphere", span));
  });

  it("clears a near plane that the fitted distance alone would leave in front of the scene", () => {
    const span = 1;
    const fitted = cameraDistanceForSpan(span, DEFAULT_CAMERA_FOV_DEGREES, 1);
    const extent = drawnExtentForSpan(span);
    // A near plane inside the framing changes nothing: the fit already binds.
    expect(initialCameraDistance(span, DEFAULT_CAMERA_FOV_DEGREES, 1, 0.01)).toBeCloseTo(
      fitted,
      12,
    );
    // One past the scene does bind, and the whole drawn sphere ends up beyond it:
    // this is the case that framed the scene correctly and drew none of it.
    for (const near of [fitted, 10, 1000]) {
      const d = initialCameraDistance(span, DEFAULT_CAMERA_FOV_DEGREES, 1, near);
      expect(d - extent).toBeGreaterThanOrEqual(near);
      expect(d).toBeGreaterThanOrEqual(fitted);
    }
  });

  it("imposes no near-plane constraint for a value that is not a near plane", () => {
    // The camera prop is checked before it reaches the camera, so these never
    // arrive; the arithmetic must not turn them into a NaN camera position.
    const fitted = cameraDistanceForSpan(1, DEFAULT_CAMERA_FOV_DEGREES, 1);
    for (const near of [0, -5, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(initialCameraDistance(1, DEFAULT_CAMERA_FOV_DEGREES, 1, near)).toBeCloseTo(fitted, 12);
    }
  });

  it("measures the drawn extent from the outermost thing in the scene", () => {
    const span = 1;
    const arrow = arrowGeometryForSpan(span);
    expect(drawnExtentForSpan(span)).toBeGreaterThanOrEqual(frameAxisLengthForSpan(span));
    expect(drawnExtentForSpan(span)).toBeGreaterThanOrEqual(arrow.startOffset + arrow.length);
  });

  it("starts an arrow outside the spacecraft and leaves room for the head", () => {
    const span = 1;
    const g = arrowGeometryForSpan(span);
    // The tail clears the spacecraft's own half-extent, so the shaft is visible.
    expect(g.startOffset).toBeGreaterThanOrEqual(span / 2);
    // The head is a part of the arrow, not longer than it.
    expect(g.headLength).toBeLessThan(g.length);
    // A head wider than it is long would read as a disc.
    expect(g.headRadius).toBeLessThan(g.headLength);
    // The shaft is thinner than the head, so the arrow has a direction.
    expect(g.shaftRadius).toBeLessThan(g.headRadius);
  });
});
