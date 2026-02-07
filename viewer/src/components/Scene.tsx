import { Canvas } from "@react-three/fiber";
import { OrbitControls } from "@react-three/drei";
import { CelestialBody } from "./CelestialBody.js";
import { OrbitTrail } from "./OrbitTrail.js";
import { Satellite } from "./Satellite.js";
import { OrbitPoint } from "../orbit.js";

interface SceneProps {
  points: OrbitPoint[] | null;
  satellitePosition: OrbitPoint | null;
  trailVisibleCount: number;
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
  centralBody,
  centralBodyRadius,
}: SceneProps) {
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

      {/* Lighting */}
      <ambientLight intensity={1.0} color={0x404040} />
      <directionalLight intensity={2.0} position={[5, 3, 5]} />

      {/* Central body */}
      <CelestialBody bodyId={centralBody} />

      {/* Axes helper (X=red, Y=green, Z=blue, length = 2 radii) */}
      <axesHelper args={[2]} />

      {/* Orbit trail and satellite (only when data is loaded) */}
      {points && points.length > 0 && (
        <OrbitTrail points={points} visibleCount={trailVisibleCount} scaleRadius={centralBodyRadius} />
      )}
      {satellitePosition && <Satellite position={satellitePosition} scaleRadius={centralBodyRadius} />}
    </Canvas>
  );
}
