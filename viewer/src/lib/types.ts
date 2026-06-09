/**
 * Public types for the embeddable orbit viewer.
 *
 * These are intentionally smaller than the viewer's internal `OrbitPoint`:
 * an embedder should be able to show a satellite by giving a position and
 * (optionally) a velocity / attitude / trail — not by filling in orbital
 * elements and chart-derived fields they don't have.
 *
 * Conventions (shared with the rest of orts):
 * - **Units**: distances in kilometres, velocities in km/s, angles in radians.
 * - **Frame**: positions/velocities are in the central-body-centred inertial
 *   frame (ECI-like; for Earth this is J2000). The central body sits at the
 *   scene origin.
 * - **Attitude**: a body→inertial rotation quaternion in Hamilton,
 *   scalar-first order `[w, x, y, z]`.
 */

import type { CSSProperties } from "react";

/** A 3D vector `[x, y, z]`. Distances are in kilometres. */
export type Vec3 = [number, number, number];

/** A quaternion in Hamilton scalar-first order `[w, x, y, z]`. */
export type Quat = [number, number, number, number];

/** A single past point on a satellite's trail. */
export interface TrailPoint {
  /** Position in km, central-body-centred inertial frame. */
  position: Vec3;
  /**
   * Seconds since the epoch. Required only when displaying the trail in a
   * body-fixed (ECEF) frame, where each point must be de-rotated by the
   * Earth-rotation angle at its own time. Inertial/local-orbital frames ignore it.
   */
  time?: number;
}

/** One satellite (or arbitrary point object) to display in the scene. */
export interface SatelliteState {
  /** Stable identifier. Used for React keys, default colour, and trail buffers. */
  id: string;
  /** Position in km, central-body-centred inertial frame. */
  position: Vec3;
  /** Velocity in km/s. Optional; required for the `localOrbital` frame. */
  velocity?: Vec3;
  /** Body→inertial attitude quaternion `[w, x, y, z]`. Optional. */
  attitude?: Quat;
  /**
   * Past trajectory, oldest first, rendered as a trailing line. Appending to
   * this list re-uses the existing GPU buffer (only new points are uploaded);
   * shrinking it, or bumping {@link SatelliteState.trailVersion}, forces a rebuild.
   */
  trail?: readonly TrailPoint[];
  /**
   * Opaque token that, when changed, forces the trail's GPU buffer to be rebuilt
   * from scratch. Bump it after a discontinuity (e.g. seeking, a new run) so a
   * same-length-but-different trail isn't mistaken for an append.
   */
  trailVersion?: string | number;
  /** Marker/trail colour as a 0xRRGGBB integer. Defaults to a palette colour. */
  color?: number;
  /** Human-readable name. Used for 3D model lookup and labels. */
  name?: string;
}

/** The central body rendered at the scene origin. */
export interface CentralBody {
  /**
   * Body identifier understood by the renderer, e.g. `"earth"`, `"moon"`,
   * `"mars"`. Unknown ids fall back to a plain coloured sphere.
   */
  id: string;
  /** Physical radius in km. Used as the scene scale factor. */
  radiusKm: number;
}

/**
 * Which frame to render in. Deliberately narrower than the renderer's internal
 * frame type: only the centres/orientations actually implemented are exposed.
 *
 * - `centralBody` + `inertial` — ECI-like, central body at the origin (default).
 * - `centralBody` + `bodyFixed` — body-fixed (ECEF-like); the body is static and
 *   positions rotate with it.
 * - `{ satelliteId }` + `inertial` — that satellite at the origin, inertial axes.
 * - `{ satelliteId }` + `localOrbital` — that satellite at the origin, LVLH axes
 *   (radial / along-track / cross-track). Requires the satellite's velocity;
 *   falls back to `inertial` if it's missing.
 */
export type ViewerReferenceFrame =
  | { center: "centralBody"; orientation?: "inertial" | "bodyFixed" }
  | { center: { satelliteId: string }; orientation?: "inertial" | "localOrbital" };

/** Props for the {@link OrbitViewer} component. */
export interface OrbitViewerProps {
  /** The central body at the origin. */
  centralBody: CentralBody;
  /** Satellites to display. */
  satellites: readonly SatelliteState[];
  /** Display frame. Defaults to central-body inertial (ECI-like). */
  referenceFrame?: ViewerReferenceFrame;
  /**
   * Julian Date of the epoch. When provided, the Sun direction, lighting and
   * body rotation are computed for real (via the bundled arika WASM). When
   * omitted, a fixed default Sun direction is used and the body does not spin.
   */
  epochJd?: number;
  /** Elapsed seconds since the epoch, for time-dependent Sun/rotation. Default 0. */
  time?: number;
  /**
   * Override the Sun direction as a unit vector in the inertial frame.
   * Bypasses the WASM ephemeris; useful when you have no epoch but still want
   * to control lighting.
   */
  sunDirection?: Vec3;
  /** Base URL for high-resolution body textures (e.g. `"/textures/"`). */
  textureBaseUrl?: string;
  /** Class applied to the wrapping element. */
  className?: string;
  /** Inline style applied to the wrapping element. */
  style?: CSSProperties;
}

/** Default frame when none is supplied: central-body inertial (ECI-like). */
export const DEFAULT_VIEWER_FRAME: ViewerReferenceFrame = {
  center: "centralBody",
  orientation: "inertial",
};
