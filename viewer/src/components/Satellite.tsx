import {
  displayPosition,
  displayQuaternion,
  type Quat,
  resolveDisplayFrame,
  sampleAttitude,
} from "../displayFrame.js";
import type { OrbitPoint } from "../orbit.js";
import { isLegacyEcef, type ReferenceFrame } from "../referenceFrame.js";
import type { MarkerShape } from "../satelliteShapes.js";
import type { LvlhAxes } from "../sceneFrame.js";
import { finiteOrNull } from "../utils/finite.js";
import { earth_rotation_angle } from "../wasm/arikaInit.js";
import { SpacecraftVisual } from "./SpacecraftVisual.js";

interface SatelliteProps {
  /** Current interpolated orbit state (position in km). */
  position: OrbitPoint;
  /** Central body radius in km, used as the scale factor. */
  scaleRadius: number;
  /** Marker color (default: 0xff4444). */
  color?: number;
  /** Reference frame for display (default: central-body inertial). */
  referenceFrame?: ReferenceFrame;
  /** Julian Date of the simulation epoch (needed for ECEF transform). */
  epochJd?: number;
  /**
   * The scene's own elapsed time, used when this satellite's sample carries none.
   *
   * `SatelliteState.time` is per satellite and documented to default to the
   * scene's, so a sample time that is not a number is treated as absent rather
   * than as a reason to drop the rotation: without it this marker would sit in
   * the inertial frame while the body, the trails and every other satellite use
   * body-fixed axes.
   */
  sceneTime?: number;
  /** Satellite identifier for model lookup. */
  satId?: string;
  /** Satellite display name for model lookup fallback. */
  satName?: string | null;
  /** Origin position in ECI [km] for the current frame center, or null for central body. */
  originPosition?: [number, number, number] | null;
  /** LVLH axes for satellite body-frame transform. */
  lvlhAxes?: LvlhAxes | null;
  /**
   * Resolved marker shape for satellites without a 3D model. When omitted, falls
   * back to automatic (orientation-revealing cube when attitude is present, else
   * a sphere). A GLTF model, when available, always takes precedence.
   */
  markerShape?: MarkerShape;
}

const DEFAULT_REF_FRAME: ReferenceFrame = {
  center: { type: "central_body" },
  orientation: "inertial",
};

/**
 * One satellite in the orbit scene: resolves where it is drawn and in which
 * frame, then hands the display-frame values to {@link SpacecraftVisual}.
 */
export function Satellite({
  position,
  scaleRadius,
  color,
  referenceFrame = DEFAULT_REF_FRAME,
  epochJd,
  sceneTime,
  satId,
  satName,
  originPosition = null,
  lvlhAxes = null,
  markerShape,
}: SatelliteProps) {
  // One display frame drives both the position and the attitude, so they can
  // never end up in different bases (the LVLH axis order and the ECEF rotation
  // used to be derived independently, and disagreed).
  // This satellite's own sample time when it has one, else the scene's. A `NaN`
  // would make the angle `NaN`, which `resolveDisplayOrientation` refuses — and
  // the frame it fell back to would be inertial while the rest of the scene is
  // body-fixed, so the marker would be drawn in a basis of its own.
  const sampleTime = finiteOrNull(position.t) ?? finiteOrNull(sceneTime) ?? 0;
  const era =
    isLegacyEcef(referenceFrame) && epochJd != null
      ? earth_rotation_angle(epochJd, sampleTime)
      : null;
  const frame = resolveDisplayFrame(referenceFrame, { era, originPosition, lvlhAxes });

  const scenePos = displayPosition(frame, position.x, position.y, position.z, scaleRadius);

  // Attitude as delivered — body-to-inertial, Hamilton [w, x, y, z] — brought to
  // unit norm. Three.js applies the components with `Quaternion.set`, which does
  // not normalise: a simulator's drifting quaternion scales and skews the very
  // marker it is meant to orient, and one that names no rotation at all is read
  // back as the identity, so the scene would report an orientation nobody
  // measured. What cannot be normalised is reported as no attitude, which draws
  // the marker unrotated — the answer a satellite without attitude already gets.
  const rawQuaternion: Quat | undefined = sampleAttitude(position);

  return (
    <SpacecraftVisual
      position={scenePos}
      quaternion={displayQuaternion(frame, rawQuaternion)}
      satId={satId}
      satName={satName}
      color={color}
      markerShape={markerShape}
    />
  );
}
