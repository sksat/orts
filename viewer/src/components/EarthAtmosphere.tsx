import { useFrame } from "@react-three/fiber";
import { useEffect, useMemo, useRef } from "react";
import * as THREE from "three";
import {
  ATMOSPHERE_SCALE_AMPLIFIED,
  ATMOSPHERE_SCALE_PHYSICAL,
  atmosphereFrag,
  atmosphereVert,
} from "../shaders/atmosphere.js";

/** Number of sphere segments for the atmosphere shell. */
export const ATMO_SEGMENTS = 48;

/** Compute the atmosphere radius for the scattering shader. */
export function getAtmosphereRadius(radius: number, physicalScale: boolean): number {
  const scale = physicalScale ? ATMOSPHERE_SCALE_PHYSICAL : ATMOSPHERE_SCALE_AMPLIFIED;
  return radius * scale;
}

/** Create the atmosphere ShaderMaterial with correct blending settings. */
export function createAtmosphereMaterial(): THREE.ShaderMaterial {
  return new THREE.ShaderMaterial({
    uniforms: {
      sunDirection: { value: new THREE.Vector3(1, 0, 0) },
      sunIntensity: { value: 1.0 },
      cameraWorldPos: { value: new THREE.Vector3(5, 0, 0) },
      earthRadius: { value: 1.0 },
      atmosphereRadius: { value: ATMOSPHERE_SCALE_AMPLIFIED },
    },
    vertexShader: atmosphereVert,
    fragmentShader: atmosphereFrag,
    transparent: true,
    blending: THREE.AdditiveBlending,
    side: THREE.BackSide,
    depthWrite: false,
    depthTest: true,
  });
}

interface EarthAtmosphereProps {
  radius: number;
  sunDirection: THREE.Vector3;
  sunIntensity?: number;
  physicalScale?: boolean;
}

/**
 * Renders an atmospheric scattering shell around the Earth.
 * Uses a BackSide sphere with additive blending for limb glow.
 */
export function EarthAtmosphere({
  radius,
  sunDirection,
  sunIntensity = 1.0,
  physicalScale = false,
}: EarthAtmosphereProps) {
  // Create material once via useMemo (synchronous — available on first render).
  const material = useMemo(() => createAtmosphereMaterial(), []);
  const materialRef = useRef(material);
  materialRef.current = material;

  // Dispose on unmount.
  useEffect(() => {
    return () => {
      materialRef.current.dispose();
    };
  }, []);

  // Update uniforms reactively.
  useEffect(() => {
    material.uniforms.sunDirection.value.copy(sunDirection).normalize();
  }, [material, sunDirection]);

  useEffect(() => {
    material.uniforms.sunIntensity.value = sunIntensity;
  }, [material, sunIntensity]);

  useEffect(() => {
    material.uniforms.earthRadius.value = radius;
    material.uniforms.atmosphereRadius.value = getAtmosphereRadius(radius, physicalScale);
  }, [material, radius, physicalScale]);

  // Update camera position every frame.
  useFrame(({ camera }) => {
    materialRef.current.uniforms.cameraWorldPos.value.copy(camera.position);
  });

  // Geometry radius: always use the amplified scale (larger) so the sphere
  // is big enough for both physical and amplified modes.
  const geometryRadius = radius * ATMOSPHERE_SCALE_AMPLIFIED;

  return (
    <mesh material={material}>
      <sphereGeometry args={[geometryRadius, ATMO_SEGMENTS, ATMO_SEGMENTS]} />
    </mesh>
  );
}
