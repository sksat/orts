import { Suspense } from "react";
import { transformToLvlh } from "../coordTransform.js";
import type { OrbitPoint } from "../orbit.js";
import { isLegacyEcef, type ReferenceFrame } from "../referenceFrame.js";
import { getSatelliteModelConfig } from "../satelliteModels.js";
import type { LvlhAxes } from "../sceneFrame.js";
import { eci_to_ecef } from "../wasm/kanameInit.js";
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
  /** When true, suppress the sphere fallback (used for centered satellite at origin). */
  hideSphereFallback?: boolean;
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
}: SatelliteProps) {
  let scenePos: [number, number, number];

  if (isLegacyEcef(referenceFrame) && epochJd != null) {
    // WASM fast path for ECEF
    const ecef = eci_to_ecef(position.x, position.y, position.z, epochJd, position.t);
    scenePos = [ecef[0] / scaleRadius, ecef[1] / scaleRadius, ecef[2] / scaleRadius];
  } else if (originPosition != null && lvlhAxes != null) {
    // LVLH body-frame transform (f64 precision)
    scenePos = transformToLvlh(
      position.x,
      position.y,
      position.z,
      originPosition,
      lvlhAxes,
      scaleRadius,
    );
  } else if (originPosition != null) {
    // Simple offset subtraction fallback
    scenePos = [
      (position.x - originPosition[0]) / scaleRadius,
      (position.y - originPosition[1]) / scaleRadius,
      (position.z - originPosition[2]) / scaleRadius,
    ];
  } else {
    scenePos = [position.x / scaleRadius, position.y / scaleRadius, position.z / scaleRadius];
  }

  const modelConfig = satId ? getSatelliteModelConfig(satId, satName) : null;

  if (modelConfig) {
    // TODO: transform quaternion for non-inertial frames (ECEF: compose ERA rotation,
    // LVLH: compose inverse LVLH quaternion). Currently correct only in ECI/inertial view.
    const quaternion: [number, number, number, number] | undefined =
      position.qw != null
        ? [position.qw, position.qx ?? 0, position.qy ?? 0, position.qz ?? 0]
        : undefined;
    return (
      <Suspense fallback={<SphereMarker position={scenePos} color={color} />}>
        <SatelliteModel position={scenePos} config={modelConfig} quaternion={quaternion} />
      </Suspense>
    );
  }

  if (hideSphereFallback) return null;
  return <SphereMarker position={scenePos} color={color} />;
}
