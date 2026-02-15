import { Suspense } from "react";
import { OrbitPoint } from "../orbit.js";
import { type ReferenceFrame, isLegacyEcef } from "../referenceFrame.js";
import { eci_to_ecef } from "../wasm/kanameInit.js";
import { transformToLvlh } from "../coordTransform.js";
import type { LvlhAxes } from "../sceneFrame.js";
import { getSatelliteModelConfig } from "../satelliteModels.js";
import { SatelliteModel } from "./SatelliteModel.js";
import type { DisplayScaleProfile } from "../displayScale.js";

/** Default radius of the sphere fallback marker in scene units (body-centered). */
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
  /** Active display scale profile. */
  displayProfile?: DisplayScaleProfile;
}

const DEFAULT_REF_FRAME: ReferenceFrame = { center: { type: "central_body" }, orientation: "inertial" };

function SphereMarker({ position, color, radius = DEFAULT_SPHERE_RADIUS }: {
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
  displayProfile,
}: SatelliteProps) {
  let scenePos: [number, number, number];

  if (isLegacyEcef(referenceFrame) && epochJd != null) {
    // WASM fast path for ECEF
    const ecef = eci_to_ecef(position.x, position.y, position.z, epochJd, position.t);
    scenePos = [ecef[0] / scaleRadius, ecef[1] / scaleRadius, ecef[2] / scaleRadius];
  } else if (originPosition != null && lvlhAxes != null) {
    // LVLH body-frame transform (f64 precision)
    scenePos = transformToLvlh(position.x, position.y, position.z, originPosition, lvlhAxes, scaleRadius);
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
  const sphereRadius = displayProfile?.sphereFallbackRadius ?? DEFAULT_SPHERE_RADIUS;

  if (modelConfig) {
    return (
      <Suspense fallback={<SphereMarker position={scenePos} color={color} radius={sphereRadius} />}>
        <SatelliteModel
          position={scenePos}
          config={modelConfig}
          displayProfile={displayProfile}
          centralBodyRadius={scaleRadius}
        />
      </Suspense>
    );
  }

  if (hideSphereFallback) return null;
  return <SphereMarker position={scenePos} color={color} radius={sphereRadius} />;
}
