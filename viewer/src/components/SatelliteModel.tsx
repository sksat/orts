import { useGLTF } from "@react-three/drei";
import { useEffect, useMemo, useRef } from "react";
import * as THREE from "three";
import type { SatelliteModelConfig } from "../satelliteModels.js";

interface SatelliteModelProps {
  /** Position in scene units (already divided by scaleRadius). */
  position: [number, number, number];
  /** Model configuration from the registry. */
  config: SatelliteModelConfig;
  /** Body-to-inertial quaternion [w, x, y, z] (Hamilton scalar-first). */
  quaternion?: [number, number, number, number];
}

export function SatelliteModel({ position, config, quaternion }: SatelliteModelProps) {
  const { scene } = useGLTF(config.modelUrl);
  const cloned = useMemo(() => scene.clone(true), [scene]);
  const groupRef = useRef<THREE.Group>(null);

  // Dev-time measurement: log the model's native bounding box span
  useEffect(() => {
    if (import.meta.env.DEV && config.nativeSpanUnits == null) {
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
    if (groupRef.current && quaternion) {
      const [w, x, y, z] = quaternion;
      groupRef.current.quaternion.set(x, y, z, w); // Three.js: (x, y, z, w)
    }
  }, [quaternion]);

  return (
    <group position={position} ref={quaternion ? groupRef : undefined}>
      <primitive object={cloned} scale={config.scale} rotation={config.rotation} />
    </group>
  );
}
