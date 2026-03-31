import { useEffect, useState } from "react";
import * as THREE from "three";
import { type BodyRenderInfo, getBodyRenderInfo } from "../bodies.js";
import { type TextureResolution, useTextureResolution } from "../hooks/useTextureResolution.js";
import { EarthBody } from "./EarthBody.js";

interface CelestialBodyProps {
  /** Body identifier from the server (e.g., "earth"). */
  bodyId: string;
  /** Radius in scene units (default 1). */
  radius?: number;
  /** Normalized sun direction in world space (ECI). */
  sunDirection?: THREE.Vector3;
  /** Earth Rotation Angle in radians (for Earth self-rotation in ECI). */
  rotationAngle?: number;
  /** Position in LVLH frame (scene units). When set, body is placed here instead of origin. */
  lvlhPosition?: [number, number, number] | null;
  /** Quaternion [x,y,z,w] for body orientation in LVLH frame. Replaces ERA-based euler. */
  lvlhQuaternion?: [number, number, number, number] | null;
  /** Ambient light intensity for shader-based bodies (matches scene ambient). */
  ambientIntensity?: number;
  /** Sun intensity scale factor: (1 AU / distance)². Default 1.0. */
  sunIntensity?: number;
  /** When true, atmosphere uses physical scale. Default false (amplified). */
  physicalScale?: boolean;
  /** Bumped when server notifies high-res textures are available. Triggers re-upgrade. */
  textureRevision?: number;
  /** Base URL for fetching high-res textures. */
  textureBaseUrl?: string;
}

const FALLBACK_CHAIN: TextureResolution[] = ["16k", "8k", "4k"];

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

function TexturedBody({
  renderInfo,
  radius,
  targetResolution,
  textureRevision,
  textureBaseUrl,
}: {
  renderInfo: BodyRenderInfo;
  radius: number;
  targetResolution?: TextureResolution;
  textureRevision?: number;
  textureBaseUrl?: string;
}) {
  const [texture, setTexture] = useState<THREE.Texture | null>(null);
  const [baseLoaded, setBaseLoaded] = useState(false);
  const [upgraded, setUpgraded] = useState(false);

  // Load base texture
  useEffect(() => {
    let cancelled = false;
    setBaseLoaded(false);
    setUpgraded(false);
    new THREE.TextureLoader().load(
      renderInfo.texturePath!,
      (tex) => {
        tex.colorSpace = THREE.SRGBColorSpace;
        if (!cancelled) {
          setTexture(tex);
          setBaseLoaded(true);
        }
      },
      undefined,
      () => {},
    );
    return () => {
      cancelled = true;
    };
  }, [renderInfo.texturePath]);

  // Upgrade to higher resolution — re-runs on textureRevision bump (server notification)
  // and retries periodically until successful.
  // biome-ignore lint/correctness/useExhaustiveDependencies: textureRevision is an intentional trigger to re-attempt texture upgrade when server notifies new textures are available.
  useEffect(() => {
    if (!baseLoaded || !renderInfo.textureBaseName) return;
    if (!targetResolution || targetResolution === "2k") return;
    if (upgraded) return;

    let cancelled = false;
    const basePath = textureBaseUrl ?? `${import.meta.env.BASE_URL}textures/`;
    const startIdx = FALLBACK_CHAIN.indexOf(targetResolution);
    const candidates = startIdx >= 0 ? FALLBACK_CHAIN.slice(startIdx) : [];

    async function tryUpgrade() {
      for (const res of candidates) {
        if (cancelled) return;
        const url = `${basePath}${renderInfo.textureBaseName}_${res}.jpg`;
        const newTex = await loadTexture(url);
        if (cancelled) {
          newTex?.dispose();
          return;
        }
        if (newTex) {
          setTexture((old) => {
            old?.dispose();
            return newTex;
          });
          setUpgraded(true);
          return;
        }
      }
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
    baseLoaded,
    targetResolution,
    renderInfo.textureBaseName,
    textureRevision,
    upgraded,
    textureBaseUrl,
  ]);

  return (
    <group>
      <mesh>
        <sphereGeometry args={[radius, 64, 64]} />
        {texture ? (
          renderInfo.isSelfLuminous ? (
            <meshBasicMaterial map={texture} />
          ) : (
            <meshStandardMaterial map={texture} />
          )
        ) : (
          <meshPhongMaterial
            color={renderInfo.fallbackColor}
            emissive={renderInfo.emissiveColor}
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
  );
}

function FallbackBody({ renderInfo, radius }: { renderInfo: BodyRenderInfo; radius: number }) {
  return (
    <group>
      <mesh>
        <sphereGeometry args={[radius, 64, 64]} />
        <meshPhongMaterial
          color={renderInfo.fallbackColor}
          emissive={renderInfo.emissiveColor}
          emissiveIntensity={0.1}
          shininess={25}
        />
      </mesh>
      <mesh>
        <sphereGeometry args={[radius * 1.002, 24, 24]} />
        <meshBasicMaterial color={0x4488cc} wireframe transparent opacity={0.15} />
      </mesh>
    </group>
  );
}

/**
 * Renders a celestial body sphere with texture if available,
 * falling back to a colored Phong sphere.
 */
export function CelestialBody({
  bodyId,
  radius = 1,
  sunDirection,
  rotationAngle,
  lvlhPosition = null,
  lvlhQuaternion = null,
  ambientIntensity,
  sunIntensity,
  physicalScale,
  textureRevision,
  textureBaseUrl,
}: CelestialBodyProps) {
  const renderInfo = getBodyRenderInfo(bodyId);
  const isSatelliteCentered = lvlhPosition != null;
  const targetResolution = useTextureResolution(isSatelliteCentered);

  let body: React.ReactNode;

  if (renderInfo.nightTexturePath && renderInfo.texturePath && sunDirection) {
    body = (
      <EarthBody
        radius={radius}
        sunDirection={sunDirection}
        dayTexturePath={renderInfo.texturePath}
        nightTexturePath={renderInfo.nightTexturePath}
        rotationAngle={lvlhQuaternion != null ? undefined : rotationAngle}
        targetResolution={targetResolution}
        textureBaseName={renderInfo.textureBaseName}
        nightTextureBaseName={renderInfo.nightTextureBaseName}
        ambientIntensity={ambientIntensity}
        sunIntensity={sunIntensity}
        physicalScale={physicalScale}
        textureRevision={textureRevision}
        textureBaseUrl={textureBaseUrl}
      />
    );
  } else if (renderInfo.texturePath) {
    body = (
      <TexturedBody
        renderInfo={renderInfo}
        radius={radius}
        targetResolution={targetResolution}
        textureRevision={textureRevision}
        textureBaseUrl={textureBaseUrl}
      />
    );
  } else {
    body = <FallbackBody renderInfo={renderInfo} radius={radius} />;
  }

  // In LVLH mode: position and orient the body via an outer group
  if (lvlhPosition != null || lvlhQuaternion != null) {
    const quat = lvlhQuaternion
      ? new THREE.Quaternion(
          lvlhQuaternion[0],
          lvlhQuaternion[1],
          lvlhQuaternion[2],
          lvlhQuaternion[3],
        )
      : undefined;
    return (
      <group position={lvlhPosition ?? undefined} quaternion={quat}>
        {body}
      </group>
    );
  }

  return <>{body}</>;
}
