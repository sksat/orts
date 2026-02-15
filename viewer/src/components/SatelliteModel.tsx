import { useMemo, useEffect } from "react";
import { useGLTF } from "@react-three/drei";
import * as THREE from "three";
import { computeTrueModelScale, type SatelliteModelConfig } from "../satelliteModels.js";
import type { DisplayScaleProfile } from "../displayScale.js";

interface SatelliteModelProps {
  /** Position in scene units (already divided by scaleRadius). */
  position: [number, number, number];
  /** Model configuration from the registry. */
  config: SatelliteModelConfig;
  /** Active display scale profile. */
  displayProfile?: DisplayScaleProfile;
  /** Central body radius in km (needed for true-scale computation). */
  centralBodyRadius?: number;
}

export function SatelliteModel({ position, config, displayProfile, centralBodyRadius }: SatelliteModelProps) {
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

  const effectiveScale = useMemo(() => {
    if (displayProfile?.trueScale && centralBodyRadius) {
      const trueScale = computeTrueModelScale(config, centralBodyRadius);
      if (trueScale != null) return trueScale;
    }
    return config.scale;
  }, [config, displayProfile?.trueScale, centralBodyRadius]);

  return (
    <group position={position}>
      <primitive
        object={cloned}
        scale={effectiveScale}
        rotation={config.rotation}
      />
    </group>
  );
}
