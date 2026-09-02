import { describe, expect, it } from "vitest";
import {
  AXIS_COLORS,
  AXIS_DIRECTIONS,
  AXIS_LETTERS,
  axisColorCss,
  axisLabelExtent,
  axisLabelPositions,
  axisLabelScale,
} from "./axisTriad.js";
import {
  arrowGeometryForSpan,
  axisLengthForSpan,
  bodyAxisArrows,
  drawnExtentForSpan,
  frameAxisArrows,
  frameAxisLengthForSpan,
} from "./spacecraftScale.js";

describe("axis label placement", () => {
  it("puts each letter on its own axis and nowhere else", () => {
    const [x, y, z] = axisLabelPositions(2);
    expect(x[1]).toBe(0);
    expect(x[2]).toBe(0);
    expect(y[0]).toBe(0);
    expect(y[2]).toBe(0);
    expect(z[0]).toBe(0);
    expect(z[1]).toBe(0);
    expect(x[0]).toBeGreaterThan(0);
    expect(y[1]).toBeGreaterThan(0);
    expect(z[2]).toBeGreaterThan(0);
  });

  it("sits past the tip, clear of the arrow head", () => {
    // On the tip the letter overlaps the head it names; too far and it reads as a
    // separate object.
    for (const length of [0.03, 0.75, 2]) {
      const [x] = axisLabelPositions(length);
      expect(x[0]).toBeGreaterThan(length);
      expect(x[0]).toBeLessThan(length * 1.5);
    }
  });

  it("scales with the axis, and stays shorter than it", () => {
    for (const length of [0.03, 0.75, 2]) {
      expect(axisLabelScale(length)).toBeCloseTo(axisLabelScale(1) * length, 12);
      expect(axisLabelScale(length)).toBeLessThan(length);
    }
  });

  it("keeps the body letters clear of the spacecraft, inside the frame letters", () => {
    // Both triads are drawn about one spacecraft, so the two sets of letters must
    // not land on top of each other.
    const span = 1;
    const [bodyX] = axisLabelPositions(axisLengthForSpan(span));
    const [frameX] = axisLabelPositions(frameAxisLengthForSpan(span));
    expect(bodyX[0]).toBeGreaterThan(span / 2);
    expect(frameX[0]).toBeGreaterThan(bodyX[0] + axisLabelScale(axisLengthForSpan(span)));
  });

  it("reports an extent that covers the sprite, not just its centre", () => {
    for (const length of [0.03, 0.75, 2]) {
      const [x] = axisLabelPositions(length);
      expect(axisLabelExtent(length)).toBeCloseTo(x[0] + axisLabelScale(length) / 2, 12);
      expect(axisLabelExtent(length)).toBeGreaterThan(length);
    }
  });
});

describe("axis triad appearance", () => {
  it("names one letter, colour and direction per axis, in the same order", () => {
    expect(AXIS_LETTERS).toEqual(["X", "Y", "Z"]);
    expect(AXIS_COLORS).toHaveLength(3);
    expect(AXIS_DIRECTIONS).toHaveLength(3);
    // Each direction is the unit vector of the axis its letter names.
    AXIS_DIRECTIONS.forEach((dir, i) => {
      expect(dir[i]).toBe(1);
      expect(dir.filter((c) => c === 0)).toHaveLength(2);
    });
  });

  it("keeps every axis colour bright enough to read on a dark scene", () => {
    // The pure hues put two channels at 0; #0000ff in particular is near-black
    // against the scene background.
    for (const color of AXIS_COLORS) {
      const channels = [(color >> 16) & 0xff, (color >> 8) & 0xff, color & 0xff];
      expect(Math.max(...channels)).toBeGreaterThan(0xf0);
      expect(Math.min(...channels)).toBeGreaterThan(0x40);
    }
  });

  it("writes a six-digit CSS colour, zero-padded", () => {
    expect(axisColorCss(0xff4d4d)).toBe("#ff4d4d");
    expect(axisColorCss(0x0000ff)).toBe("#0000ff");
  });
});

describe("axis arrow proportions", () => {
  it("takes the extent from the caller and the thickness from the span", () => {
    const body = bodyAxisArrows(0.75, 1);
    expect(body.length).toBe(0.75);
    expect(body.startOffset).toBe(0);
    // Doubling the span thickens the arrow without moving its tip.
    const thicker = bodyAxisArrows(0.75, 2);
    expect(thicker.length).toBe(0.75);
    expect(thicker.shaftRadius).toBeCloseTo(body.shaftRadius * 2, 12);
  });

  it("draws the body triad heavier than the reference frame's", () => {
    const span = 1;
    const body = bodyAxisArrows(axisLengthForSpan(span), span);
    const frame = frameAxisArrows(frameAxisLengthForSpan(span), span);
    expect(frame.shaftRadius).toBeLessThan(body.shaftRadius);
    expect(frame.headRadius).toBeLessThan(body.headRadius);
    // The frame axes are the longer pair, so equal thickness would make the
    // subordinate triad the heaviest thing on screen.
    expect(frame.length).toBeGreaterThan(body.length);
  });

  it("keeps every head shorter than the axis it caps", () => {
    for (const span of [0.016, 1, 100]) {
      const body = bodyAxisArrows(axisLengthForSpan(span), span);
      const frame = frameAxisArrows(frameAxisLengthForSpan(span), span);
      expect(body.headLength).toBeLessThan(body.length / 2);
      expect(frame.headLength).toBeLessThan(frame.length / 2);
    }
  });

  it("keeps the axis shafts thinner than a direction arrow's is long", () => {
    // Sanity on the ratios: the annotations share one scene, and an axis shaft as
    // thick as an arrow is long would read as a block.
    const arrow = arrowGeometryForSpan(1);
    expect(bodyAxisArrows(axisLengthForSpan(1), 1).shaftRadius).toBeLessThan(arrow.length / 10);
  });
});

describe("camera fit", () => {
  it("reaches past the reference frame's letters, not just its axes", () => {
    for (const span of [0.016, 1, 100]) {
      expect(drawnExtentForSpan(span)).toBeGreaterThanOrEqual(
        axisLabelExtent(frameAxisLengthForSpan(span)),
      );
      // The letters are what makes the frame triad the binding term.
      expect(drawnExtentForSpan(span)).toBeGreaterThan(frameAxisLengthForSpan(span));
    }
  });
});
