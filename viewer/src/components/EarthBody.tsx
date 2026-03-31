import { useEffect, useRef, useState } from "react";
import * as THREE from "three";
import type { TextureResolution } from "../hooks/useTextureResolution.js";
import { earthDayNightFrag, earthDayNightVert } from "../shaders/earthDayNight.js";
import { EarthAtmosphere } from "./EarthAtmosphere.js";

/**
 * Euler rotation [rx, ry, rz] that aligns the Three.js sphere (Y-pole)
 * with the ECI coordinate system (Z = north pole).
 *
 * Rotation of +π/2 around X maps: local +Y → world +Z (north pole).
 */
export const POLE_ALIGNMENT_ROTATION: [number, number, number] = [Math.PI / 2, 0, 0];

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
  /** Bumped when server notifies high-res textures are available. Triggers re-upgrade. */
  textureRevision?: number;
  /** Base URL for fetching high-res textures. */
  textureBaseUrl?: string;
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
  textureRevision,
  textureBaseUrl,
}: EarthBodyProps) {
  const materialRef = useRef<THREE.ShaderMaterial | null>(null);
  const [ready, setReady] = useState(false);
  const [upgraded, setUpgraded] = useState(false);

  // 1. Load 2K textures manually (no Suspense — keeps Canvas interactive)
  // biome-ignore lint/correctness/useExhaustiveDependencies: uniform values are synced by separate effects below; recreating the material on every uniform change would reload textures unnecessarily.
  useEffect(() => {
    let cancelled = false;
    setReady(false);
    Promise.all([loadTexture(dayTexturePath), loadTexture(nightTexturePath)]).then(
      ([dayMap, nightMap]) => {
        if (cancelled || !dayMap || !nightMap) return;
        materialRef.current = new THREE.ShaderMaterial({
          uniforms: {
            dayMap: { value: dayMap },
            nightMap: { value: nightMap },
            sunDirection: { value: sunDirection.clone().normalize() },
            ambientIntensity: { value: ambientIntensity },
            sunIntensity: { value: sunIntensity },
          },
          vertexShader: earthDayNightVert,
          fragmentShader: earthDayNightFrag,
        });
        setReady(true);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [dayTexturePath, nightTexturePath]);

  // 2. Async upgrade to higher-resolution textures — re-runs on textureRevision bump
  //    and retries periodically until successful.
  // biome-ignore lint/correctness/useExhaustiveDependencies: textureRevision is an intentional trigger to re-attempt texture upgrade when server notifies new textures are available.
  useEffect(() => {
    if (!ready) return;
    if (!targetResolution || targetResolution === "2k" || !textureBaseName || !nightTextureBaseName)
      return;
    if (!materialRef.current) return;
    if (upgraded) return;

    let cancelled = false;
    const basePath = textureBaseUrl ?? `${import.meta.env.BASE_URL}textures/`;

    // Build fallback chain starting from target resolution
    const startIdx = FALLBACK_CHAIN.indexOf(targetResolution);
    const candidates = startIdx >= 0 ? FALLBACK_CHAIN.slice(startIdx) : [];

    async function tryUpgrade() {
      for (const res of candidates) {
        if (cancelled) return;

        const dayUrl = `${basePath}${textureBaseName}_${res}.jpg`;
        const nightUrl = `${basePath}${nightTextureBaseName}_${res}.jpg`;

        const [newDay, newNight] = await Promise.all([loadTexture(dayUrl), loadTexture(nightUrl)]);

        if (cancelled) {
          newDay?.dispose();
          newNight?.dispose();
          return;
        }

        // Both textures must load successfully for this resolution
        if (newDay && newNight) {
          if (materialRef.current) {
            const oldDay = materialRef.current.uniforms.dayMap.value as THREE.Texture;
            const oldNight = materialRef.current.uniforms.nightMap.value as THREE.Texture;

            materialRef.current.uniforms.dayMap.value = newDay;
            materialRef.current.uniforms.nightMap.value = newNight;
            materialRef.current.needsUpdate = true;

            // Dispose old textures to free GPU memory
            oldDay.dispose();
            oldNight.dispose();
          }
          setUpgraded(true);
          return; // success
        }

        // Partial load: clean up and try next resolution
        newDay?.dispose();
        newNight?.dispose();
      }
      // All resolutions failed — 2K fallback remains active
    }

    tryUpgrade();

    // Periodic retry every 10s until upgrade succeeds or component unmounts.
    const timer = setInterval(() => {
      if (!cancelled) tryUpgrade();
    }, 10_000);

    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [
    ready,
    targetResolution,
    textureBaseName,
    nightTextureBaseName,
    textureRevision,
    upgraded,
    textureBaseUrl,
  ]);

  // 3. Update uniforms reactively (no material recreation)
  // `ready` dependency ensures uniforms are set after material creation
  // (materialRef is populated asynchronously and not tracked by React).
  // biome-ignore lint/correctness/useExhaustiveDependencies: ready signals that materialRef.current is available.
  useEffect(() => {
    if (materialRef.current) {
      materialRef.current.uniforms.sunDirection.value.copy(sunDirection).normalize();
    }
  }, [sunDirection, ready]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: ready signals that materialRef.current is available.
  useEffect(() => {
    if (materialRef.current) {
      materialRef.current.uniforms.ambientIntensity.value = ambientIntensity;
    }
  }, [ambientIntensity, ready]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: ready signals that materialRef.current is available.
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
            <meshBasicMaterial color={0x4488cc} wireframe transparent opacity={0.15} />
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
