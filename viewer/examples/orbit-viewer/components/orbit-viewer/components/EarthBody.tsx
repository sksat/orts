import { useEffect, useRef, useState } from "react";
import * as THREE from "three";
import { IS_DEV } from "../env.js";
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

// Decode textures off the main thread via createImageBitmap, so a large texture
// doesn't decode synchronously while uploading (the dominant source of frame
// hitches on big textures). The bitmap is pre-flipped to match TextureLoader's
// default; WebGL can't flip an ImageBitmap at upload, so texture.flipY is off.
const bitmapLoader = new THREE.ImageBitmapLoader();
bitmapLoader.setOptions({ imageOrientation: "flipY" });

/**
 * Try loading a texture by URL. Returns the loaded texture or null on failure.
 */
function loadTexture(url: string): Promise<THREE.Texture | null> {
  return new Promise((resolve) => {
    bitmapLoader.load(
      url,
      (bitmap) => {
        const tex = new THREE.Texture(bitmap);
        tex.colorSpace = THREE.SRGBColorSpace;
        tex.flipY = false;
        tex.needsUpdate = true;
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
  const poleGroupRef = useRef<THREE.Group>(null);
  const [ready, setReady] = useState(false);
  const [upgraded, setUpgraded] = useState(false);
  // Guards against overlapping high-res loads. A ref (not an effect-local) so it
  // persists across effect re-runs — e.g. a textureRevision bump that re-runs the
  // upgrade effect while a previous load is still decoding — not just within one run.
  const inFlightRef = useRef(false);

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
    // Only upgrade when a real texture source is provided (a connected orts
    // server, or an explicit base URL). No source → keep the bundled 2K.
    if (!textureBaseUrl) return;

    let cancelled = false;
    const basePath = textureBaseUrl;

    // Build fallback chain starting from target resolution
    const startIdx = FALLBACK_CHAIN.indexOf(targetResolution);
    const candidates = startIdx >= 0 ? FALLBACK_CHAIN.slice(startIdx) : [];

    async function tryUpgrade() {
      // Don't stack a second load while one is in flight: decode is off-thread
      // (ImageBitmapLoader) but the GPU upload of a large texture is still
      // synchronous on the main thread, so overlapping loads pile up into hitches.
      if (inFlightRef.current) return;
      inFlightRef.current = true;
      try {
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
        // No resolution available right now — 2K stays. A textureRevision bump
        // (server "textures_ready") re-runs this effect to try again.
      } finally {
        inFlightRef.current = false;
      }
    }

    tryUpgrade();

    // Bounded fallback poll. The textureRevision bump is the primary re-trigger;
    // this just covers a missed signal. Stop after MAX_RETRIES so a permanently
    // unavailable resolution doesn't loop forever (e.g. static hosting without
    // the high-res files), and never re-upload while a load is already running.
    let attempts = 0;
    const MAX_RETRIES = 3;
    const timer = setInterval(() => {
      if (cancelled) return;
      // A load is still in flight — skip without spending a retry, so a slow load
      // (>10s) doesn't exhaust the budget on ticks that tryUpgrade would bail on.
      if (inFlightRef.current) return;
      if (attempts >= MAX_RETRIES) {
        clearInterval(timer);
        return;
      }
      attempts += 1;
      tryUpgrade();
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

  // Dev/E2E-only: expose the rendered Earth orientation (mesh world quaternion,
  // which includes the internal POLE_ALIGNMENT_ROTATION) so E2E tests can assert
  // the central-body orientation from the live scene graph without reading pixels.
  // See viewer/tests/lvlh-orientation.spec.ts.
  useEffect(() => {
    if (!IS_DEV) return;
    const w = window as unknown as Record<string, unknown>;
    w.__debug_get_earth_world_quat = (): [number, number, number, number] | null => {
      const g = poleGroupRef.current;
      if (!g) return null;
      const q = g.getWorldQuaternion(new THREE.Quaternion());
      return [q.x, q.y, q.z, q.w];
    };
    return () => {
      delete w.__debug_get_earth_world_quat;
    };
  }, []);

  return (
    <group>
      <group rotation={[0, 0, rotationAngle ?? 0]}>
        {/* Inner group: align Three.js Y-pole to ECI Z-pole (north pole → +Z) */}
        <group ref={poleGroupRef} rotation={POLE_ALIGNMENT_ROTATION}>
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
