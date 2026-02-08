import { Canvas } from "@react-three/fiber";
import { OrbitControls } from "@react-three/drei";
import * as THREE from "three";
import { CelestialBody } from "./CelestialBody.js";
import { OrbitTrail } from "./OrbitTrail.js";
import { Satellite } from "./Satellite.js";
import { OrbitPoint } from "../orbit.js";
import { TrailBuffer } from "../utils/TrailBuffer.js";

// Sun direction in ECI J2000: +X = vernal equinox.
// Static for now; will become dynamic when epoch support is added.
const SUN_DIRECTION = new THREE.Vector3(1, 0, 0);

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
}: SceneProps) {
  const hasTrailData = trailBuffer
    ? trailBuffer.length > 0
    : points != null && points.length > 0;

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

      {/* Lighting — sun direction aligned with ECI +X (vernal equinox) */}
      <ambientLight intensity={0.5} color={0x404040} />
      <directionalLight intensity={2.0} position={[10, 0, 0]} />

      {/* Central body */}
      <CelestialBody bodyId={centralBody} sunDirection={SUN_DIRECTION} />

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
