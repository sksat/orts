import { OrbitPoint } from "../orbit.js";
import { eciToEcef, type DisplayFrame } from "../frameTransform.js";

interface SatelliteProps {
  /** Current interpolated orbit state (position in km). */
  position: OrbitPoint;
  /** Central body radius in km, used as the scale factor. */
  scaleRadius: number;
  /** Marker color (default: 0xff4444). */
  color?: number;
  /** Display coordinate frame (default: "eci"). */
  displayFrame?: DisplayFrame;
  /** ERA at current time (needed for ECEF transform). */
  era?: number;
}

/**
 * Satellite marker component: a small sphere at the current
 * interpolated orbit position.
 */
export function Satellite({ position, scaleRadius, color = 0xff4444, displayFrame = "eci", era }: SatelliteProps) {
  let px = position.x, py = position.y, pz = position.z;
  if (displayFrame === "ecef" && era != null) {
    [px, py, pz] = eciToEcef(px, py, pz, era);
  }

  return (
    <mesh
      position={[
        px / scaleRadius,
        py / scaleRadius,
        pz / scaleRadius,
      ]}
    >
      <sphereGeometry args={[0.03, 16, 16]} />
      <meshBasicMaterial color={color} />
    </mesh>
  );
}
