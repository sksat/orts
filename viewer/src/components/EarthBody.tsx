import { useEffect, useRef } from "react";
import { useTexture } from "@react-three/drei";
import * as THREE from "three";
import {
  earthDayNightVert,
  earthDayNightFrag,
} from "../shaders/earthDayNight.js";
import type { TextureResolution } from "../hooks/useTextureResolution.js";

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

/** Resolution fallback chain: try highest first, then step down. */
const FALLBACK_CHAIN: TextureResolution[] = ["8k", "4k"];

interface EarthBodyProps {
  radius: number;
  sunDirection: THREE.Vector3;
  dayTexturePath: string;
  nightTexturePath: string;
  /** Earth Rotation Angle in radians (for self-rotation around Z in ECI). */
  rotationAngle?: number;
  /** Target texture resolution determined by GPU capabilities. */
  targetResolution?: TextureResolution;
  /** Base name for multi-resolution day textures (e.g., "earth"). */
  textureBaseName?: string;
  /** Base name for multi-resolution night textures (e.g., "earth_night"). */
  nightTextureBaseName?: string;
}

/**
 * Try loading a texture by URL. Returns the loaded texture or null on failure.
 */
function loadTexture(url: string): Promise<THREE.Texture | null> {
  return new Promise((resolve) => {
    new THREE.TextureLoader().load(
      url,
      (tex) => {
        tex.colorSpace = THREE.SRGBColorSpace;
        resolve(tex);
      },
      undefined,
      () => resolve(null),
    );
  });
}

export function EarthBody({
  radius,
  sunDirection,
  dayTexturePath,
  nightTexturePath,
  rotationAngle,
  targetResolution,
  textureBaseName,
  nightTextureBaseName,
}: EarthBodyProps) {
  // 1. Load 2K textures immediately via Suspense (guaranteed available)
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

  // 2. Async upgrade to higher-resolution textures
  useEffect(() => {
    if (
      !targetResolution ||
      targetResolution === "2k" ||
      !textureBaseName ||
      !nightTextureBaseName
    )
      return;
    if (!materialRef.current) return;

    let cancelled = false;
    const basePath = import.meta.env.BASE_URL + "textures/";

    // Build fallback chain starting from target resolution
    const startIdx = FALLBACK_CHAIN.indexOf(targetResolution);
    const candidates =
      startIdx >= 0 ? FALLBACK_CHAIN.slice(startIdx) : [];

    async function tryUpgrade() {
      for (const res of candidates) {
        if (cancelled) return;

        const dayUrl = `${basePath}${textureBaseName}_${res}.jpg`;
        const nightUrl = `${basePath}${nightTextureBaseName}_${res}.jpg`;

        const [newDay, newNight] = await Promise.all([
          loadTexture(dayUrl),
          loadTexture(nightUrl),
        ]);

        if (cancelled) {
          newDay?.dispose();
          newNight?.dispose();
          return;
        }

        // Both textures must load successfully for this resolution
        if (newDay && newNight) {
          if (materialRef.current) {
            const oldDay = materialRef.current.uniforms.dayMap
              .value as THREE.Texture;
            const oldNight = materialRef.current.uniforms.nightMap
              .value as THREE.Texture;

            materialRef.current.uniforms.dayMap.value = newDay;
            materialRef.current.uniforms.nightMap.value = newNight;
            materialRef.current.needsUpdate = true;

            // Dispose old textures to free GPU memory
            oldDay.dispose();
            oldNight.dispose();
          }
          return; // success
        }

        // Partial load: clean up and try next resolution
        newDay?.dispose();
        newNight?.dispose();
      }
      // All resolutions failed — 2K fallback remains active
    }

    tryUpgrade();
    return () => {
      cancelled = true;
    };
  }, [targetResolution, textureBaseName, nightTextureBaseName]);

  // 3. Update sun direction uniform reactively (no material recreation)
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
