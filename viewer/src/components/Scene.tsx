import { useMemo } from "react";
import { Canvas } from "@react-three/fiber";
import { OrbitControls } from "@react-three/drei";
import * as THREE from "three";
import { CelestialBody } from "./CelestialBody.js";
import { OrbitTrail } from "./OrbitTrail.js";
import { Satellite } from "./Satellite.js";
import { OrbitPoint } from "../orbit.js";
import { TrailBuffer } from "../utils/TrailBuffer.js";
import { sunDirectionECI } from "../astro.js";

// Default sun direction when no epoch is provided: ECI +X (vernal equinox).
const DEFAULT_SUN_DIRECTION = new THREE.Vector3(1, 0, 0);

interface SceneProps {
  /** Points array for replay mode. */
  points?: OrbitPoint[] | null;
  satellitePosition: OrbitPoint | null;
  /** Visible count for replay mode progressive trail. */
  trailVisibleCount?: number;
  /** TrailBuffer for realtime mode. */
  trailBuffer?: TrailBuffer;
  centralBody: string;
  centralBodyRadius: number;
  /** Julian Date of the simulation epoch, or null if not set. */
  epochJd?: number | null;
}

/**
 * Main Three.js scene component using @react-three/fiber Canvas.
 * Contains camera, controls, lights, central body, orbit trail, and satellite.
 */
export function Scene({
  points,
  satellitePosition,
  trailVisibleCount,
  trailBuffer,
  centralBody,
  centralBodyRadius,
  epochJd,
}: SceneProps) {
  const hasTrailData = trailBuffer
    ? trailBuffer.length > 0
    : points != null && points.length > 0;

  // Compute sun direction from epoch + current sim time.
  // When no epoch is provided, fall back to static +X direction.
  // Quantize sim time to 60s intervals — sun moves ~1°/day so sub-minute
  // updates are imperceptible but would cause unnecessary re-renders.
  const simTime = satellitePosition?.t ?? 0;
  const quantizedSimTime = Math.floor(simTime / 60) * 60;
  const sunDirection = useMemo(() => {
    if (epochJd == null) return DEFAULT_SUN_DIRECTION;
    const [x, y, z] = sunDirectionECI(epochJd, quantizedSimTime);
    return new THREE.Vector3(x, y, z);
  }, [epochJd, quantizedSimTime]);

  // Directional light position aligned with sun direction (scaled for scene)
  const lightPosition = useMemo<[number, number, number]>(() => {
    return [sunDirection.x * 10, sunDirection.y * 10, sunDirection.z * 10];
  }, [sunDirection]);

  return (
    <Canvas
      camera={{ position: [0, 2, 5], fov: 60, near: 0.01, far: 1000 }}
      style={{ position: "absolute", top: 0, left: 0, width: "100%", height: "100%" }}
    >
      {/* Controls */}
      <OrbitControls
        enableDamping
        dampingFactor={0.1}
        minDistance={1.5}
        maxDistance={100}
      />

      {/* Lighting — follows sun direction */}
      <ambientLight intensity={0.5} color={0x404040} />
      <directionalLight intensity={2.0} position={lightPosition} />

      {/* Central body */}
      <CelestialBody bodyId={centralBody} sunDirection={sunDirection} />

      {/* Axes helper (X=red, Y=green, Z=blue, length = 2 radii) */}
      <axesHelper args={[2]} />

      {/* Orbit trail and satellite (only when data is loaded) */}
      {hasTrailData && (
        trailBuffer ? (
          <OrbitTrail trailBuffer={trailBuffer} scaleRadius={centralBodyRadius} />
        ) : (
          <OrbitTrail
            points={points!}
            visibleCount={trailVisibleCount ?? points!.length}
            scaleRadius={centralBodyRadius}
          />
        )
      )}
      {satellitePosition && <Satellite position={satellitePosition} scaleRadius={centralBodyRadius} />}
    </Canvas>
  );
}
