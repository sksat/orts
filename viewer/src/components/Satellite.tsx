import { Suspense } from "react";
import {
  displayPosition,
  displayQuaternion,
  type Quat,
  resolveDisplayFrame,
} from "../displayFrame.js";
import type { OrbitPoint } from "../orbit.js";
import { isLegacyEcef, type ReferenceFrame } from "../referenceFrame.js";
import { getSatelliteModelConfig } from "../satelliteModels.js";
import { type MarkerShape, resolveMarkerShape } from "../satelliteShapes.js";
import type { LvlhAxes } from "../sceneFrame.js";
import { earth_rotation_angle } from "../wasm/arikaInit.js";
import { BodyAxes } from "./BodyAxes.js";
import { PrimitiveMarker } from "./PrimitiveMarker.js";
import { SatelliteModel } from "./SatelliteModel.js";

/** Default radius of the sphere fallback marker in scene units. */
const DEFAULT_SPHERE_RADIUS = 0.005;

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
  /** When true, suppress the marker fallback (used for centered satellite at origin). */
  hideSphereFallback?: boolean;
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

function SphereMarker({
  position,
  color,
  radius = DEFAULT_SPHERE_RADIUS,
}: {
  position: [number, number, number];
  color: number;
  radius?: number;
}) {
  return (
    <mesh position={position}>
      <sphereGeometry args={[radius, 16, 16]} />
      <meshBasicMaterial color={color} />
    </mesh>
  );
}

/**
 * Satellite marker component: renders a 3D model for known satellites,
 * or a small sphere for unknown ones.
 */
export function Satellite({
  position,
  scaleRadius,
  color = 0xff4444,
  referenceFrame = DEFAULT_REF_FRAME,
  epochJd,
  satId,
  satName,
  originPosition = null,
  lvlhAxes = null,
  hideSphereFallback = false,
  markerShape,
}: SatelliteProps) {
  // One display frame drives both the position and the attitude, so they can
  // never end up in different bases (the LVLH axis order and the ECEF rotation
  // used to be derived independently, and disagreed).
  const era =
    isLegacyEcef(referenceFrame) && epochJd != null
      ? earth_rotation_angle(epochJd, position.t)
      : null;
  const frame = resolveDisplayFrame(referenceFrame, { era, originPosition, lvlhAxes });

  const scenePos = displayPosition(frame, position.x, position.y, position.z, scaleRadius);

  // Attitude quaternion as delivered: body-to-inertial, Hamilton [w, x, y, z].
  const rawQuaternion: Quat | undefined =
    position.qw != null
      ? [position.qw, position.qx ?? 0, position.qy ?? 0, position.qz ?? 0]
      : undefined;

  const displayQuat = displayQuaternion(frame, rawQuaternion);

  const modelConfig = satId ? getSatelliteModelConfig(satId, satName) : null;

  const bodyAxes = displayQuat ? (
    <BodyAxes
      position={scenePos}
      quaternion={displayQuat}
      axisLength={modelConfig ? modelConfig.scale * 5 : DEFAULT_SPHERE_RADIUS * 6}
      debugId={satId}
    />
  ) : null;

  if (modelConfig) {
    return (
      <>
        <Suspense fallback={<SphereMarker position={scenePos} color={color} />}>
          <SatelliteModel position={scenePos} config={modelConfig} quaternion={displayQuat} />
        </Suspense>
        {bodyAxes}
      </>
    );
  }

  if (hideSphereFallback) return bodyAxes;

  // Pick the marker shape: caller-resolved override/default, else automatic
  // (orientation-revealing cube when attitude is present — a sphere looks identical
  // at every orientation — sphere otherwise).
  const shape = resolveMarkerShape({
    override: markerShape,
    hasAttitude: displayQuat != null,
  });
  const fallbackMarker =
    shape === "sphere" ? (
      <SphereMarker position={scenePos} color={color} />
    ) : (
      <PrimitiveMarker position={scenePos} quaternion={displayQuat} />
    );
  return (
    <>
      {fallbackMarker}
      {bodyAxes}
    </>
  );
}
