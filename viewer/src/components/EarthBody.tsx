import { useEffect, useRef } from "react";
import { useTexture } from "@react-three/drei";
import * as THREE from "three";
import {
  earthDayNightVert,
  earthDayNightFrag,
} from "../shaders/earthDayNight.js";

/**
 * Euler rotation [rx, ry, rz] that aligns the Three.js sphere (Y-pole)
 * with the ECI coordinate system (Z = north pole).
 *
 * Rotation of +π/2 around X maps: local +Y → world +Z (north pole).
 */
export const POLE_ALIGNMENT_ROTATION: [number, number, number] = [
  Math.PI / 2,
  0,
  0,
];

interface EarthBodyProps {
  radius: number;
  sunDirection: THREE.Vector3;
  dayTexturePath: string;
  nightTexturePath: string;
  /** Earth Rotation Angle in radians (for self-rotation around Z in ECI). */
  rotationAngle?: number;
}

export function EarthBody({
  radius,
  sunDirection,
  dayTexturePath,
  nightTexturePath,
  rotationAngle,
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
    <group rotation={[0, 0, rotationAngle ?? 0]}>
      {/* Inner group: align Three.js Y-pole to ECI Z-pole (north pole → +Z) */}
      <group rotation={POLE_ALIGNMENT_ROTATION}>
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
    </group>
  );
}
