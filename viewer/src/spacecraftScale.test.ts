import { describe, expect, it } from "vitest";
import type { SatelliteModelConfig } from "./satelliteModels.js";
import {
  arrowGeometryForSpan,
  axisLengthForSpan,
  cameraDistanceForSpan,
  DEFAULT_CAMERA_FOV_DEGREES,
  drawnExtentForSpan,
  frameAxisLengthForSpan,
  NOMINAL_SPACECRAFT_SPAN,
  resolveVisualSpan,
  spanNormalizedModelScale,
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

  it("pulls further back for a narrower field of view", () => {
    expect(cameraDistanceForSpan(1, 30, 1)).toBeGreaterThan(cameraDistanceForSpan(1, 50, 1));
    expect(cameraDistanceForSpan(1, 50, 1)).toBeGreaterThan(cameraDistanceForSpan(1, 75, 1));
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
