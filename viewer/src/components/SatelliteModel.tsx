import { useGLTF } from "@react-three/drei";
import { useEffect, useMemo, useRef } from "react";
import * as THREE from "three";
import { IS_DEV } from "../env.js";
import type { SatelliteModelConfig } from "../satelliteModels.js";

interface SatelliteModelProps {
  /** Position in scene units (already divided by scaleRadius). */
  position: [number, number, number];
  /** Model configuration from the registry. */
  config: SatelliteModelConfig;
  /** Body-to-inertial quaternion [w, x, y, z] (Hamilton scalar-first). */
  quaternion?: [number, number, number, number];
  /**
   * Scale in scene units per model unit, overriding `config.scale`. A view whose
   * scene units are not central-body radii (the attitude view) sets its own.
   */
  scale?: number;
}

export function SatelliteModel({
  position,
  config,
  quaternion,
  scale = config.scale,
}: SatelliteModelProps) {
  const { scene } = useGLTF(config.modelUrl);
  const cloned = useMemo(() => scene.clone(true), [scene]);
  const groupRef = useRef<THREE.Group>(null);

  // Dev-time measurement: log the model's native bounding box span
  useEffect(() => {
    if (IS_DEV && config.nativeSpanUnits == null) {
      const box = new THREE.Box3().setFromObject(scene);
      const size = box.getSize(new THREE.Vector3());
      const span = Math.max(size.x, size.y, size.z);
      console.log(
        `[SatelliteModel] Native bounding box for "${config.modelUrl}":`,
        `size=(${size.x.toFixed(2)}, ${size.y.toFixed(2)}, ${size.z.toFixed(2)})`,
        `max span=${span.toFixed(2)}`,
        `— set nativeSpanUnits to this value in satelliteModels.ts`,
      );
    }
  }, [scene, config]);

  // Apply body-to-inertial quaternion to the parent group
  useEffect(() => {
    if (!groupRef.current) return;
    if (quaternion) {
      const [w, x, y, z] = quaternion;
      groupRef.current.quaternion.set(x, y, z, w);
    } else {
      // A stream can stop carrying a usable attitude — a sample whose quaternion
      // names no rotation, or none at all after one that did. Leaving the group
      // where the last good sample put it would show that orientation as the
      // current one, so it goes back to unrotated, which is what "no attitude"
      // draws from the start.
      groupRef.current.quaternion.identity(); // Three.js: (x, y, z, w)
    }
  }, [quaternion]);

  return (
    // The ref is attached whether or not there is an attitude, because losing one
    // is the case the effect above exists for: a conditional ref is detached
    // during the commit that drops the quaternion, so the effect would find no
    // group and the model would keep the rotation the last good sample gave it.
    <group position={position} ref={groupRef}>
      <primitive object={cloned} scale={scale} rotation={config.rotation} />
    </group>
  );
}
