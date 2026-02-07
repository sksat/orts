import { OrbitPoint } from "../orbit.js";

interface SatelliteProps {
  /** Current interpolated orbit state (position in km). */
  position: OrbitPoint;
  /** Central body radius in km, used as the scale factor. */
  scaleRadius: number;
}

/**
 * Satellite marker component: a small red sphere at the current
 * interpolated orbit position.
 */
export function Satellite({ position, scaleRadius }: SatelliteProps) {
  return (
    <mesh
      position={[
        position.x / scaleRadius,
        position.y / scaleRadius,
        position.z / scaleRadius,
      ]}
    >
      <sphereGeometry args={[0.03, 16, 16]} />
      <meshBasicMaterial color={0xff4444} />
    </mesh>
  );
}
