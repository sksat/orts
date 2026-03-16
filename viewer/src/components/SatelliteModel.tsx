import { useGLTF } from "@react-three/drei";
import { useEffect, useMemo } from "react";
import * as THREE from "three";
import type { SatelliteModelConfig } from "../satelliteModels.js";

interface SatelliteModelProps {
  /** Position in scene units (already divided by scaleRadius). */
  position: [number, number, number];
  /** Model configuration from the registry. */
  config: SatelliteModelConfig;
}

export function SatelliteModel({ position, config }: SatelliteModelProps) {
  const { scene } = useGLTF(config.modelUrl);
  const cloned = useMemo(() => scene.clone(true), [scene]);

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

  return (
    <group position={position}>
      <primitive object={cloned} scale={config.scale} rotation={config.rotation} />
    </group>
  );
}
