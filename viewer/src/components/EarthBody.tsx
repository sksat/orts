import { useEffect, useRef } from "react";
import { useTexture } from "@react-three/drei";
import * as THREE from "three";
import {
  earthDayNightVert,
  earthDayNightFrag,
} from "../shaders/earthDayNight.js";

interface EarthBodyProps {
  radius: number;
  sunDirection: THREE.Vector3;
  dayTexturePath: string;
  nightTexturePath: string;
}

export function EarthBody({
  radius,
  sunDirection,
  dayTexturePath,
  nightTexturePath,
}: EarthBodyProps) {
  const [dayMap, nightMap] = useTexture([dayTexturePath, nightTexturePath]);
  const materialRef = useRef<THREE.ShaderMaterial | null>(null);

  // Create material once when textures load
  if (!materialRef.current) {
    materialRef.current = new THREE.ShaderMaterial({
      uniforms: {
        dayMap: { value: dayMap },
        nightMap: { value: nightMap },
        sunDirection: { value: sunDirection.clone().normalize() },
      },
      vertexShader: earthDayNightVert,
      fragmentShader: earthDayNightFrag,
    });
  }

  // Update sun direction uniform reactively (no material recreation)
  useEffect(() => {
    if (materialRef.current) {
      materialRef.current.uniforms.sunDirection.value
        .copy(sunDirection)
        .normalize();
    }
  }, [sunDirection]);

  return (
    <group>
      <mesh material={materialRef.current}>
        <sphereGeometry args={[radius, 64, 64]} />
      </mesh>
      <mesh>
        <sphereGeometry args={[radius * 1.002, 24, 24]} />
        <meshBasicMaterial
          color={0x4488cc}
          wireframe
          transparent
          opacity={0.15}
        />
      </mesh>
    </group>
  );
}
