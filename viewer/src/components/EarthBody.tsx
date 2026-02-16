import { useEffect, useRef, useState } from "react";
import * as THREE from "three";
import {
  earthDayNightVert,
  earthDayNightFrag,
} from "../shaders/earthDayNight.js";
import type { TextureResolution } from "../hooks/useTextureResolution.js";
import { EarthAtmosphere } from "./EarthAtmosphere.js";

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
const FALLBACK_CHAIN: TextureResolution[] = ["16k", "8k", "4k"];

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
  /** Ambient light intensity (matches scene ambient). Default 0.15. */
  ambientIntensity?: number;
  /** Sun intensity scale factor: (1 AU / distance)². Default 1.0. */
  sunIntensity?: number;
  /** When true, atmosphere uses physical scale (~100km). Default false (amplified). */
  physicalScale?: boolean;
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
  ambientIntensity = 0.15,
  sunIntensity = 1.0,
  physicalScale = false,
}: EarthBodyProps) {
  const materialRef = useRef<THREE.ShaderMaterial | null>(null);
  const [ready, setReady] = useState(false);

  // 1. Load 2K textures manually (no Suspense — keeps Canvas interactive)
  useEffect(() => {
    let cancelled = false;
    Promise.all([
      loadTexture(dayTexturePath),
      loadTexture(nightTexturePath),
    ]).then(([dayMap, nightMap]) => {
      if (cancelled || !dayMap || !nightMap) return;
      materialRef.current = new THREE.ShaderMaterial({
        uniforms: {
          dayMap: { value: dayMap },
          nightMap: { value: nightMap },
          sunDirection: { value: new THREE.Vector3(0, 0, 1) },
          ambientIntensity: { value: ambientIntensity },
          sunIntensity: { value: sunIntensity },
        },
        vertexShader: earthDayNightVert,
        fragmentShader: earthDayNightFrag,
      });
      setReady(true);
    });
    return () => { cancelled = true; };
  }, [dayTexturePath, nightTexturePath]);

  // 2. Async upgrade to higher-resolution textures
  useEffect(() => {
    if (!ready) return;
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
  }, [ready, targetResolution, textureBaseName, nightTextureBaseName]);

  // 3. Update uniforms reactively (no material recreation)
  // `ready` dependency ensures uniforms are set after material creation.
  useEffect(() => {
    if (materialRef.current) {
      materialRef.current.uniforms.sunDirection.value
        .copy(sunDirection)
        .normalize();
    }
  }, [sunDirection, ready]);

  useEffect(() => {
    if (materialRef.current) {
      materialRef.current.uniforms.ambientIntensity.value = ambientIntensity;
    }
  }, [ambientIntensity, ready]);

  useEffect(() => {
    if (materialRef.current) {
      materialRef.current.uniforms.sunIntensity.value = sunIntensity;
    }
  }, [sunIntensity, ready]);

  return (
    <group>
      <group rotation={[0, 0, rotationAngle ?? 0]}>
        {/* Inner group: align Three.js Y-pole to ECI Z-pole (north pole → +Z) */}
        <group rotation={POLE_ALIGNMENT_ROTATION}>
          <mesh material={materialRef.current ?? undefined}>
            <sphereGeometry args={[radius, 64, 64]} />
            {!ready && (
              <meshPhongMaterial
                color={0x2244aa}
                emissive={0x112244}
                emissiveIntensity={0.1}
                shininess={25}
              />
            )}
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
      {/* Atmosphere: uniform sphere, no rotation needed */}
      <EarthAtmosphere
        radius={radius}
        sunDirection={sunDirection}
        sunIntensity={sunIntensity}
        physicalScale={physicalScale}
      />
    </group>
  );
}
