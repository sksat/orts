/**
 * Reference frame types for the 3D viewer.
 *
 * A reference frame has two components:
 * - **center**: which object is at the origin (central body, satellite, etc.)
 * - **orientation**: how the axes are aligned (inertial or body-fixed)
 */

/** Center of the reference frame (what object is at the origin). */
export type FrameCenter =
  | { type: "central_body" }
  | { type: "moon" }
  | { type: "sun" }
  | { type: "satellite"; id: string };

/** Orientation of the reference frame axes. */
export type FrameOrientation = "inertial" | "body_fixed";

/** Complete reference frame specification. */
export interface ReferenceFrame {
  center: FrameCenter;
  orientation: FrameOrientation;
}

/** Default frame: central-body-centered inertial (equivalent to legacy "eci"). */
export const DEFAULT_FRAME: ReferenceFrame = {
  center: { type: "central_body" },
  orientation: "inertial",
};

/** Whether the frame is equivalent to legacy ECI (central body + inertial). */
export function isDefaultEci(frame: ReferenceFrame): boolean {
  return frame.center.type === "central_body" && frame.orientation === "inertial";
}

/** Whether the frame is equivalent to legacy ECEF (central body + body-fixed). */
export function isLegacyEcef(frame: ReferenceFrame): boolean {
  return frame.center.type === "central_body" && frame.orientation === "body_fixed";
}

/** Structural equality check for FrameCenter values. */
export function frameCenterEquals(a: FrameCenter, b: FrameCenter): boolean {
  if (a.type !== b.type) return false;
  if (a.type === "satellite" && b.type === "satellite") {
    return a.id === b.id;
  }
  return true;
}
