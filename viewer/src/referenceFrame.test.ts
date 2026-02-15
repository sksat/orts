import { describe, it, expect } from "vitest";
import {
  DEFAULT_FRAME,
  isDefaultEci,
  isLegacyEcef,
  frameCenterEquals,
  type ReferenceFrame,
  type FrameCenter,
} from "./referenceFrame.js";

describe("DEFAULT_FRAME", () => {
  it("is central-body inertial", () => {
    expect(DEFAULT_FRAME.center.type).toBe("central_body");
    expect(DEFAULT_FRAME.orientation).toBe("inertial");
  });
});

describe("isDefaultEci", () => {
  it("returns true for central_body + inertial", () => {
    expect(isDefaultEci(DEFAULT_FRAME)).toBe(true);
  });

  it("returns false for central_body + body_fixed (ECEF)", () => {
    const frame: ReferenceFrame = {
      center: { type: "central_body" },
      orientation: "body_fixed",
    };
    expect(isDefaultEci(frame)).toBe(false);
  });

  it("returns false for satellite + inertial", () => {
    const frame: ReferenceFrame = {
      center: { type: "satellite", id: "sat-0" },
      orientation: "inertial",
    };
    expect(isDefaultEci(frame)).toBe(false);
  });

  it("returns false for moon + inertial", () => {
    const frame: ReferenceFrame = {
      center: { type: "moon" },
      orientation: "inertial",
    };
    expect(isDefaultEci(frame)).toBe(false);
  });
});

describe("isLegacyEcef", () => {
  it("returns true for central_body + body_fixed", () => {
    const frame: ReferenceFrame = {
      center: { type: "central_body" },
      orientation: "body_fixed",
    };
    expect(isLegacyEcef(frame)).toBe(true);
  });

  it("returns false for central_body + inertial", () => {
    expect(isLegacyEcef(DEFAULT_FRAME)).toBe(false);
  });

  it("returns false for satellite + body_fixed", () => {
    const frame: ReferenceFrame = {
      center: { type: "satellite", id: "sat-0" },
      orientation: "body_fixed",
    };
    expect(isLegacyEcef(frame)).toBe(false);
  });
});

describe("frameCenterEquals", () => {
  it("central_body equals central_body", () => {
    const a: FrameCenter = { type: "central_body" };
    const b: FrameCenter = { type: "central_body" };
    expect(frameCenterEquals(a, b)).toBe(true);
  });

  it("satellite equals same satellite", () => {
    const a: FrameCenter = { type: "satellite", id: "sat-0" };
    const b: FrameCenter = { type: "satellite", id: "sat-0" };
    expect(frameCenterEquals(a, b)).toBe(true);
  });

  it("satellite does not equal different satellite", () => {
    const a: FrameCenter = { type: "satellite", id: "sat-0" };
    const b: FrameCenter = { type: "satellite", id: "sat-1" };
    expect(frameCenterEquals(a, b)).toBe(false);
  });

  it("central_body does not equal satellite", () => {
    const a: FrameCenter = { type: "central_body" };
    const b: FrameCenter = { type: "satellite", id: "sat-0" };
    expect(frameCenterEquals(a, b)).toBe(false);
  });

  it("moon equals moon", () => {
    const a: FrameCenter = { type: "moon" };
    const b: FrameCenter = { type: "moon" };
    expect(frameCenterEquals(a, b)).toBe(true);
  });

  it("moon does not equal sun", () => {
    const a: FrameCenter = { type: "moon" };
    const b: FrameCenter = { type: "sun" };
    expect(frameCenterEquals(a, b)).toBe(false);
  });
});
