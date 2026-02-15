import { Suspense } from "react";
import * as THREE from "three";
import { useTexture } from "@react-three/drei";
import { getBodyRenderInfo, BodyRenderInfo } from "../bodies.js";
import { EarthBody } from "./EarthBody.js";
import { useTextureResolution } from "../hooks/useTextureResolution.js";

interface CelestialBodyProps {
  /** Body identifier from the server (e.g., "earth"). */
  bodyId: string;
  /** Radius in scene units (default 1). */
  radius?: number;
  /** Normalized sun direction in world space (ECI). */
  sunDirection?: THREE.Vector3;
  /** Earth Rotation Angle in radians (for Earth self-rotation in ECI). */
  rotationAngle?: number;
}

function TexturedBody({
  renderInfo,
  radius,
}: {
  renderInfo: BodyRenderInfo;
  radius: number;
}) {
  const texture = useTexture(renderInfo.texturePath!);

  return (
    <group>
      <mesh>
        <sphereGeometry args={[radius, 64, 64]} />
        {renderInfo.isSelfLuminous ? (
          <meshBasicMaterial map={texture} />
        ) : (
          <meshStandardMaterial map={texture} />
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
  );
}

function FallbackBody({
  renderInfo,
  radius,
}: {
  renderInfo: BodyRenderInfo;
  radius: number;
}) {
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

/**
 * Renders a celestial body sphere with texture if available,
 * falling back to a colored Phong sphere.
 */
export function CelestialBody({ bodyId, radius = 1, sunDirection, rotationAngle }: CelestialBodyProps) {
  const renderInfo = getBodyRenderInfo(bodyId);
  const targetResolution = useTextureResolution();

  if (renderInfo.nightTexturePath && renderInfo.texturePath && sunDirection) {
    return (
      <Suspense
        fallback={<FallbackBody renderInfo={renderInfo} radius={radius} />}
      >
        <EarthBody
          radius={radius}
          sunDirection={sunDirection}
          dayTexturePath={renderInfo.texturePath}
          nightTexturePath={renderInfo.nightTexturePath}
          rotationAngle={rotationAngle}
          targetResolution={targetResolution}
          textureBaseName={renderInfo.textureBaseName}
          nightTextureBaseName={renderInfo.nightTextureBaseName}
        />
      </Suspense>
    );
  }

  if (renderInfo.texturePath) {
    return (
      <Suspense
        fallback={<FallbackBody renderInfo={renderInfo} radius={radius} />}
      >
        <TexturedBody renderInfo={renderInfo} radius={radius} />
      </Suspense>
    );
  }

  return <FallbackBody renderInfo={renderInfo} radius={radius} />;
}
