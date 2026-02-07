import { Canvas } from "@react-three/fiber";
import { OrbitControls } from "@react-three/drei";
import { Earth } from "./Earth.js";
import { OrbitTrail } from "./OrbitTrail.js";
import { Satellite } from "./Satellite.js";
import { OrbitPoint } from "../orbit.js";

interface SceneProps {
  points: OrbitPoint[] | null;
  satellitePosition: OrbitPoint | null;
  trailVisibleCount: number;
}

/**
 * Main Three.js scene component using @react-three/fiber Canvas.
 * Contains camera, controls, lights, Earth, orbit trail, and satellite.
 */
export function Scene({
  points,
  satellitePosition,
  trailVisibleCount,
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

      {/* Earth */}
      <Earth />

      {/* Axes helper (X=red, Y=green, Z=blue, length = 2 Earth radii) */}
      <axesHelper args={[2]} />

      {/* Orbit trail and satellite (only when data is loaded) */}
      {points && points.length > 0 && (
        <OrbitTrail points={points} visibleCount={trailVisibleCount} />
      )}
      {satellitePosition && <Satellite position={satellitePosition} />}
    </Canvas>
  );
}
