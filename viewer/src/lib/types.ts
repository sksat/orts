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

import type { OrbitControlsProps } from "@react-three/drei";
import type { CSSProperties } from "react";
import type { WebGLRendererParameters } from "three";
import type { BodyDefinitions } from "../bodies.js";
import type { DirectionVectorOptions } from "../directionVectors.js";
import type { MarkerShape } from "../satelliteShapes.js";
import type { TrailBufferLike } from "../utils/TrailBuffer.js";

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

/** Per-satellite display state shared by both trail input modes. */
export interface SatelliteBaseState {
  /** Stable identifier. Used for React keys, default colour, and trail buffers. */
  id: string;
  /** Position in km, central-body-centred inertial frame. */
  position: Vec3;
  /** Velocity in km/s. Optional; required for the `localOrbital` frame. */
  velocity?: Vec3;
  /** Body→inertial attitude quaternion `[w, x, y, z]`. Optional. */
  attitude?: Quat;
  /**
   * Seconds since the epoch for THIS satellite's current position (the marker),
   * overriding the scene-level {@link OrbitSceneDataProps.time}. Needed when
   * satellites are at different times — e.g. a terminated satellite frozen at its
   * last sample while others keep advancing, or scrubbing past one satellite's
   * data span — so body-fixed/ECEF marker transforms use the right epoch and the
   * marker stays aligned with its own trail. Defaults to the scene-level time.
   */
  time?: number;
  /** Marker/trail colour as a 0xRRGGBB integer. Defaults to a palette colour. */
  color?: number;
  /** Human-readable name. Used for 3D model lookup and labels. */
  name?: string;
  /**
   * Marker shape for this satellite. Overrides the scene-level
   * {@link OrbitSceneDataProps.defaultMarkerShape}; `null`/omitted falls through
   * to that default, then to an automatic shape (based on whether attitude is set).
   */
  markerShape?: MarkerShape | null;
  /**
   * Clip how much of the trail is drawn — for playback scrubbing / time windows.
   * `visibleCount` caps the number of most-recent points shown; `drawStart` is the
   * first index to draw. Omit to draw the whole trail.
   */
  trailDisplay?: {
    visibleCount?: number;
    drawStart?: number;
  };
}

/**
 * How a satellite supplies its trail. Two mutually exclusive modes:
 *
 * - **value** ({@link TrailPoint}[]): the scene owns and reconciles a buffer for
 *   you — simplest for snapshot data.
 * - **streaming** ({@link TrailBufferLike}): you own the buffer and mutate it
 *   outside React (append as data arrives); the scene reads it each frame, so
 *   points reach the GPU without a React re-render. Best for high-rate feeds.
 */
export type SatelliteTrailInput =
  | {
      /**
       * Past trajectory, oldest first, rendered as a trailing line. Appending to
       * this list re-uses the existing GPU buffer (only new points are uploaded);
       * shrinking it, or bumping {@link SatelliteTrailInput.trailVersion}, forces a
       * rebuild. Treat it immutably (new array reference on change) — in-place
       * mutation is not detected.
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
      trailBuffer?: never;
    }
  | {
      /**
       * Caller-owned trail buffer for high-rate streaming. Mutate it outside React
       * (append as data arrives); the scene reads it every frame, so points reach
       * the GPU without a React re-render. Keep its identity stable across renders;
       * the buffer's own `generation` drives full re-uploads. See {@link TrailBuffer}.
       */
      trailBuffer?: TrailBufferLike;
      trail?: never;
      trailVersion?: never;
    };

/** One satellite (or arbitrary point object) to display in the scene. */
export type SatelliteState = SatelliteBaseState & SatelliteTrailInput;

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
 * - `{ satelliteId }` + `localOrbital` — that satellite at the origin in the
 *   orbit frame {@link AttitudeFrame} spells out (scene +X in-track, +Y
 *   cross-track, +Z radially outward), so the central body stays "below" as the
 *   satellite orbits. Requires the satellite's velocity; without it the view
 *   falls back to a radial-up camera follow.
 */
export type ViewerReferenceFrame =
  | { center: "centralBody"; orientation?: "inertial" | "bodyFixed" }
  | { center: { satelliteId: string }; orientation?: "inertial" | "localOrbital" };

/**
 * Scene data + display props shared by {@link OrbitViewer} (which owns its own
 * `<Canvas>`) and {@link OrbitScene} (which you mount inside your own `<Canvas>`).
 */
export interface OrbitSceneDataProps {
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
  /**
   * Cache-invalidation token for body textures: bump it (any new value) to make
   * the bodies re-fetch their textures — e.g. when higher-resolution textures
   * become available at {@link OrbitSceneDataProps.textureBaseUrl}.
   */
  textureVersion?: string | number;
  /**
   * Default marker shape for satellites without their own
   * {@link SatelliteBaseState.markerShape}. `null`/omitted means automatic
   * (chosen per satellite from whether it has attitude).
   */
  defaultMarkerShape?: MarkerShape | null;
  /**
   * Atmosphere sizing for bodies that have one (e.g. Earth). `"visual"` keeps a
   * visible shell at body-centred scale; `"physical"` uses the true scale-height
   * thickness (useful satellite-centred); `"auto"` (default) picks per view.
   */
  atmosphereScale?: "visual" | "physical" | "auto";
  /**
   * Reference-direction arrows (Sun, nadir) to draw. Omitted — the default —
   * draws none.
   *
   * Drawn only when the reference frame is centred on a satellite, and only at
   * that satellite. This view can hold many satellites, and a pair of arrows on
   * each of them fills the screen; in a central-body view the body itself is on
   * screen, so a nadir arrow repeats what the picture already shows. To annotate
   * a satellite other than the centred one, ask for a scene-level target — a
   * per-satellite flag would put the scene's display policy in the data.
   */
  directionVectors?: DirectionVectorOptions;
}

/**
 * Enable/disable the default orbit camera controls, or pass an
 * {@link OrbitControlsProps} object to configure them. Default: enabled.
 */
export type ControlsProp = boolean | Partial<OrbitControlsProps>;

/**
 * Props for {@link OrbitScene}: the orbit scene graph rendered inside a
 * caller-supplied @react-three/fiber `<Canvas>`. Bring your own Canvas to
 * compose the scene with your own lights, meshes or post-processing.
 *
 * The caller's Canvas camera should be initialised with `up = SCENE_UP`
 * (exported); the default camera rig keeps `camera.up` correct each frame, but
 * OrbitControls reads the initial camera state at mount.
 */
export interface OrbitSceneProps extends OrbitSceneDataProps {
  /** Default orbit camera/controls. `true` (default) | `false` | an OrbitControls config. */
  controls?: ControlsProp;
  /** Render the reference axes helper. Default `true`. */
  axes?: boolean;
}

/** Props for the {@link OrbitViewer} component. */
export interface OrbitViewerProps extends OrbitSceneDataProps {
  /** Class applied to the wrapping element. */
  className?: string;
  /** Inline style applied to the wrapping element. */
  style?: CSSProperties;
  /**
   * Overrides merged onto the internal `<Canvas>` setup: perspective-camera
   * framing and WebGL renderer flags. For full control (a custom WebGLRenderer
   * or camera instance, your own controls, extra meshes/lights), drop
   * {@link OrbitScene} into your own `<Canvas>` instead.
   */
  canvas?: {
    /**
     * Perspective camera overrides merged onto the defaults. `position`/`up` are
     * in Three.js scene units/directions (NOT the km-valued {@link Vec3}); the
     * central body is one scene unit in radius.
     */
    camera?: {
      position?: [number, number, number];
      up?: [number, number, number];
      fov?: number;
      near?: number;
      far?: number;
      zoom?: number;
    };
    /** WebGL renderer flags merged onto the defaults. */
    gl?: Partial<WebGLRendererParameters>;
  };
  /** See {@link OrbitSceneProps.controls}. */
  controls?: ControlsProp;
  /** See {@link OrbitSceneProps.axes}. */
  axes?: boolean;
}

/** Default frame when none is supplied: central-body inertial (ECI-like). */
export const DEFAULT_VIEWER_FRAME: ViewerReferenceFrame = {
  center: "centralBody",
  orientation: "inertial",
};

/**
 * Display orientation for the attitude view.
 *
 * There is no centre to choose: the spacecraft is at the origin, which is what
 * makes it the attitude view. `bodyFixed` needs an epoch (and an Earth central
 * body, whose rotation angle is the one the viewer models); `localOrbital` needs
 * the spacecraft's position and velocity. A requested orientation whose inputs
 * are absent falls back to `inertial`.
 *
 * `localOrbital` is an orbit frame, and its name does not pin the axes down —
 * LVLH and RSW conventions differ in both order and sign, and the view draws
 * letters on these axes for a reader to interpret. This renderer maps
 *
 * - scene **+X** to in-track, `crossTrack × radial`, which is the velocity
 *   direction for a circular orbit;
 * - scene **+Y** to cross-track, `normalize(r × v)`, the orbit normal;
 * - scene **+Z** to radial *outward*, `normalize(r)`, so nadir points along
 *   scene −Z.
 *
 * Read against the convention that puts +Z at nadir, the labelled axes would
 * come out inverted.
 */
export type AttitudeFrame = "inertial" | "bodyFixed" | "localOrbital";

/**
 * One spacecraft's attitude, for {@link AttitudeScene}.
 *
 * The required and optional fields are the mirror of {@link SatelliteState}'s:
 * the attitude is what this view exists to show, and the position is needed only
 * by the things that reference the orbit — the nadir arrow, and the
 * `localOrbital` frame (which also needs the velocity).
 */
export interface AttitudeBodyState {
  id: string;
  /** Body→inertial rotation, Hamilton scalar-first `[w, x, y, z]`. */
  attitude: Quat;
  /** Position in km, central-body-centred inertial frame. */
  position?: Vec3;
  /** Velocity in km/s, central-body-centred inertial frame. */
  velocity?: Vec3;
  /** Seconds since the epoch, for the Sun direction and the body-fixed rotation. */
  time?: number;
  /** Display name, also used to look up a 3D model. */
  name?: string;
  /** Marker colour as a hex number (e.g. `0x00ff88`). */
  color?: number;
  /** Marker shape override for a spacecraft with no 3D model. */
  markerShape?: MarkerShape | null;
}

/** Data props shared by {@link AttitudeScene} and {@link AttitudeViewer}. */
export interface AttitudeSceneDataProps {
  /**
   * The central body the spacecraft orbits. Only its `id` is read — for the Sun
   * direction, and to decide whether the body-fixed frame is available. No radius
   * is needed: this view has no physical length scale.
   */
  centralBody: CentralBody;
  /** The spacecraft whose attitude is shown. */
  body: AttitudeBodyState;
  /** Display orientation. Default `"inertial"`. */
  orientation?: AttitudeFrame;
  /**
   * Julian Date of the simulation epoch, **in UTC** — `arika`'s `Epoch::from_jd`
   * reads it as a UTC JD before deriving the Sun direction and Earth's rotation,
   * so a TT or TDB value shifts both. Without an epoch there is no Sun direction.
   *
   * The body-fixed frame this feeds is visualization-grade. Earth's rotation
   * angle is defined on UT1, and the angle used here comes from arika's legacy
   * `Epoch<Utc>::gmst`, which takes UTC for UT1 and ignores dUT1 — up to 0.9 s,
   * or some 13 arcseconds of rotation. Treat the drawn Earth-fixed orientation
   * as a picture, not as an EOP-correct frame.
   */
  epochJd?: number;
  /**
   * Seconds since the epoch, when `body.time` is not given. Default 0.
   *
   * Added to the epoch by arika's `Epoch<Utc>::add_seconds`, which is naive
   * UTC-JD arithmetic: across a leap second the instant it names is one second
   * away from the elapsed-SI-time answer, another 15 arcseconds of Earth
   * rotation on top of the dUT1 approximation above. Every wasm time binding
   * shares that arithmetic, so this is a property of the module rather than of
   * this view.
   */
  time?: number;
  /** Marker shape default for a spacecraft with no 3D model. */
  defaultMarkerShape?: MarkerShape | null;
  /** Which reference-direction arrows to draw. Default: all of them. */
  directionVectors?: DirectionVectorOptions;
}

/**
 * Props for {@link AttitudeScene}: the attitude scene graph rendered inside a
 * caller-supplied @react-three/fiber `<Canvas>`.
 *
 * The spacecraft is drawn one scene unit across, at the origin, so a camera a
 * few units out frames it whatever the real spacecraft's size.
 */
export interface AttitudeSceneProps extends AttitudeSceneDataProps {
  /** Default camera controls. `true` (default) | `false` | an OrbitControls config. */
  controls?: ControlsProp;
  /** Render the reference-frame axes. Default `true`. */
  axes?: boolean;
}

/** Props for the {@link AttitudeViewer} component. */
export interface AttitudeViewerProps extends AttitudeSceneDataProps {
  /** Class applied to the wrapping element. */
  className?: string;
  /** Inline style applied to the wrapping element. */
  style?: CSSProperties;
  /** Overrides merged onto the internal `<Canvas>` setup. */
  canvas?: {
    /**
     * Perspective camera overrides. `position`/`up` are in scene units, where the
     * spacecraft is one unit across.
     */
    camera?: {
      position?: [number, number, number];
      up?: [number, number, number];
      fov?: number;
      near?: number;
      far?: number;
      zoom?: number;
    };
    /** WebGL renderer flags merged onto the defaults. */
    gl?: Partial<WebGLRendererParameters>;
  };
  /** See {@link AttitudeSceneProps.controls}. */
  controls?: ControlsProp;
  /** See {@link AttitudeSceneProps.axes}. */
  axes?: boolean;
}
