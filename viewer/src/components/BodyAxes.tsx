import { useEffect, useRef } from "react";
import type * as THREE from "three";
import { registerSatWorldQuat } from "../debug/satWorldQuat.js";

interface BodyAxesProps {
  /** Position in scene units (already divided by scaleRadius). */
  position: [number, number, number];
  /** Body-frame quaternion [w, x, y, z] (Hamilton scalar-first). */
  quaternion: [number, number, number, number];
  /** Length of each axis in scene units. */
  axisLength?: number;
  /**
   * Optional satellite id. When set, the rendered world quaternion is published
   * (dev/E2E only) via `window.__debug_get_sat_world_quat(id)`. BodyAxes shares the
   * parent transform + quaternion with the satellite mesh, so its world quaternion
   * equals the rendered body orientation in every display variant.
   */
  debugId?: string;
}

/**
 * Renders RGB XYZ axes oriented by a body-frame quaternion.
 *
 * Uses the same quaternion-application pattern as SatelliteModel.tsx.
 */
export function BodyAxes({ position, quaternion, axisLength = 0.03, debugId }: BodyAxesProps) {
  const groupRef = useRef<THREE.Group>(null);

  useEffect(() => {
    if (groupRef.current) {
      const [w, x, y, z] = quaternion;
      groupRef.current.quaternion.set(x, y, z, w); // Three.js: (x, y, z, w)
    }
  }, [quaternion]);

  useEffect(() => {
    if (!debugId) return;
    return registerSatWorldQuat(debugId, () => groupRef.current);
  }, [debugId]);

  return (
    <group position={position} ref={groupRef}>
      <axesHelper args={[axisLength]} />
    </group>
  );
}
