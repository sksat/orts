import { useEffect, useRef } from "react";
import type * as THREE from "three";

interface BodyAxesProps {
  /** Position in scene units (already divided by scaleRadius). */
  position: [number, number, number];
  /** Body-frame quaternion [w, x, y, z] (Hamilton scalar-first). */
  quaternion: [number, number, number, number];
  /** Length of each axis in scene units. */
  axisLength?: number;
}

/**
 * Renders RGB XYZ axes oriented by a body-frame quaternion.
 *
 * Uses the same quaternion-application pattern as SatelliteModel.tsx.
 */
export function BodyAxes({ position, quaternion, axisLength = 0.03 }: BodyAxesProps) {
  const groupRef = useRef<THREE.Group>(null);

  useEffect(() => {
    if (groupRef.current) {
      const [w, x, y, z] = quaternion;
      groupRef.current.quaternion.set(x, y, z, w); // Three.js: (x, y, z, w)
    }
  }, [quaternion]);

  return (
    <group position={position} ref={groupRef}>
      <axesHelper args={[axisLength]} />
    </group>
  );
}
