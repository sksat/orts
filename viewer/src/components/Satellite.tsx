import { OrbitPoint } from "../orbit.js";

/** Earth radius in km -- same scale factor as orbit.ts. */
const EARTH_RADIUS_KM = 6378.137;

interface SatelliteProps {
  /** Current interpolated orbit state (position in km). */
  position: OrbitPoint;
}

/**
 * Satellite marker component: a small red sphere at the current
 * interpolated orbit position.
 */
export function Satellite({ position }: SatelliteProps) {
  return (
    <mesh
      position={[
        position.x / EARTH_RADIUS_KM,
        position.y / EARTH_RADIUS_KM,
        position.z / EARTH_RADIUS_KM,
      ]}
    >
      <sphereGeometry args={[0.03, 16, 16]} />
      <meshBasicMaterial color={0xff4444} />
    </mesh>
  );
}
