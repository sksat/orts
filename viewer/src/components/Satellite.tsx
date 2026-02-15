import { Suspense } from "react";
import { OrbitPoint } from "../orbit.js";
import { type DisplayFrame } from "../frameTransform.js";
import { eci_to_ecef } from "../wasm/kanameInit.js";
import { getSatelliteModelConfig } from "../satelliteModels.js";
import { SatelliteModel } from "./SatelliteModel.js";

/** Radius of the sphere fallback marker in scene units. */
const SPHERE_RADIUS = 0.005;

interface SatelliteProps {
  /** Current interpolated orbit state (position in km). */
  position: OrbitPoint;
  /** Central body radius in km, used as the scale factor. */
  scaleRadius: number;
  /** Marker color (default: 0xff4444). */
  color?: number;
  /** Display coordinate frame (default: "eci"). */
  displayFrame?: DisplayFrame;
  /** Julian Date of the simulation epoch (needed for ECEF transform). */
  epochJd?: number;
  /** Satellite identifier for model lookup. */
  satId?: string;
  /** Satellite display name for model lookup fallback. */
  satName?: string | null;
}

function SphereMarker({ position, color }: { position: [number, number, number]; color: number }) {
  return (
    <mesh position={position}>
      <sphereGeometry args={[SPHERE_RADIUS, 16, 16]} />
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
  displayFrame = "eci",
  epochJd,
  satId,
  satName,
}: SatelliteProps) {
  let px = position.x, py = position.y, pz = position.z;
  if (displayFrame === "ecef" && epochJd != null) {
    const ecef = eci_to_ecef(px, py, pz, epochJd, position.t);
    px = ecef[0]; py = ecef[1]; pz = ecef[2];
  }

  const scenePos: [number, number, number] = [
    px / scaleRadius,
    py / scaleRadius,
    pz / scaleRadius,
  ];

  const modelConfig = satId ? getSatelliteModelConfig(satId, satName) : null;

  if (modelConfig) {
    return (
      <Suspense fallback={<SphereMarker position={scenePos} color={color} />}>
        <SatelliteModel position={scenePos} config={modelConfig} />
      </Suspense>
    );
  }

  return <SphereMarker position={scenePos} color={color} />;
}
