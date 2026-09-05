import {
  displayPosition,
  displayQuaternion,
  type Quat,
  resolveDisplayFrame,
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
  satId,
  satName,
  originPosition = null,
  lvlhAxes = null,
  markerShape,
}: SatelliteProps) {
  // One display frame drives both the position and the attitude, so they can
  // never end up in different bases (the LVLH axis order and the ECEF rotation
  // used to be derived independently, and disagreed).
  // This satellite's own sample time, which is optional per satellite and can
  // arrive as `NaN` from a source. A non-finite time is no time: the angle
  // computed from it is `NaN`, and while `resolveDisplayOrientation` refuses that
  // and falls back to inertial, the fallback would be silent and this satellite
  // would sit in a different frame from the rest of the scene.
  const sampleTime = finiteOrNull(position.t);
  const era =
    isLegacyEcef(referenceFrame) && epochJd != null && sampleTime != null
      ? earth_rotation_angle(epochJd, sampleTime)
      : null;
  const frame = resolveDisplayFrame(referenceFrame, { era, originPosition, lvlhAxes });

  const scenePos = displayPosition(frame, position.x, position.y, position.z, scaleRadius);

  // Attitude quaternion as delivered: body-to-inertial, Hamilton [w, x, y, z].
  const rawQuaternion: Quat | undefined =
    position.qw != null
      ? [position.qw, position.qx ?? 0, position.qy ?? 0, position.qz ?? 0]
      : undefined;

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
