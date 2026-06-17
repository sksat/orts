import type { CSSProperties } from "react";
import type { BodyDefinitions } from "../bodies.js";

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
   *
   * Treat this immutably (new array reference on change), as with any React
   * prop — in-place mutation is not detected.
   */
  trail?: readonly TrailPoint[];
  /**
   * Opaque token that, when changed, forces the trail's GPU buffer to be rebuilt
   * from scratch. The append/rebuild diff only inspects the trail's length and
   * its last point, so you MUST bump this whenever you rewrite *existing* points
   * (seeking, a new run, editing earlier history) — otherwise a same-length edit
   * to interior points is mistaken for "unchanged" and won't reach the GPU.
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
   * Body id. One of the built-in bodies (`"earth"`, `"moon"`, `"sun"`,
   * `"mars"`), or a custom body supplied via {@link OrbitViewerProps.bodies}.
   * Unknown ids fall back to a plain coloured sphere.
   */
  id: string;
  /**
   * Physical radius in km (scene scale factor). Optional when the id resolves
   * to a known/custom {@link BodyDefinition} — its `radiusKm` is used then.
   */
  radiusKm?: number;
}

/**
 * Which frame to render in. Deliberately narrower than the renderer's internal
 * frame type: only the centres/orientations actually implemented are exposed.
 *
 * - `centralBody` + `inertial` — ECI-like, central body at the origin (default).
 * - `centralBody` + `bodyFixed` — body-fixed (ECEF-like); the body is static and
 *   positions rotate with it.
 * - `{ satelliteId }` + `inertial` — that satellite at the origin with the axes
 *   star-fixed: the central body appears to move around the satellite as it
 *   orbits, and the camera does not co-rotate.
 * - `{ satelliteId }` + `localOrbital` — that satellite at the origin, LVLH axes
 *   (radial / along-track / cross-track): the central body stays "below" as the
 *   satellite orbits. Requires the satellite's velocity; without it the view
 *   falls back to a radial-up camera follow.
 */
export type ViewerReferenceFrame =
  | { center: "centralBody"; orientation?: "inertial" | "bodyFixed" }
  | { center: { satelliteId: string }; orientation?: "inertial" | "localOrbital" };

/** Props for the {@link OrbitViewer} component. */
export interface OrbitViewerProps {
  /** The central body at the origin. */
  centralBody: CentralBody;
  /**
   * Custom body definitions, keyed by body id and merged over the built-in
   * {@link DEFAULT_BODIES} (Earth / Moon / Sun / Mars) — e.g.
   * `{ pluto: { radiusKm: 1188.3, texture: { day: "/pluto.jpg" } } }`.
   *
   * The merge is per-id and shallow: overriding a default replaces its *entire*
   * definition, so a partial override drops the default's other fields (texture,
   * colour, …). Note that physical Sun lighting / rotation only apply to bodies
   * the arika model knows; a custom body renders (radius / texture / colour) but
   * does not spin.
   */
  bodies?: BodyDefinitions;
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
