import { useMemo } from "react";
import { useGLTF } from "@react-three/drei";
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

  return (
    <group position={position}>
      <primitive
        object={cloned}
        scale={config.scale}
        rotation={config.rotation}
      />
    </group>
  );
}
