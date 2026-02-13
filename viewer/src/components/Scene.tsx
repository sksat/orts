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

/** Color palette for multiple satellites. */
const SATELLITE_COLORS = [0x00ff88, 0xff4488, 0x44aaff, 0xffaa44, 0xaa44ff];

interface SceneProps {
  /** Points array for replay mode. */
  points?: OrbitPoint[] | null;
  /** Single satellite position (replay mode). */
  satellitePosition?: OrbitPoint | null;
  /** Visible count for replay mode progressive trail. */
  trailVisibleCount?: number;
  /** TrailBuffer for single-satellite realtime mode (backward compat). */
  trailBuffer?: TrailBuffer;
  /** Per-satellite TrailBuffers for multi-satellite realtime mode. */
  trailBuffers?: Map<string, TrailBuffer>;
  /** Per-satellite positions for multi-satellite mode. */
  satellitePositions?: Map<string, OrbitPoint | null>;
  /** Per-satellite visible counts for multi-satellite mode. */
  trailVisibleCounts?: Map<string, number>;
  centralBody: string;
  centralBodyRadius: number;
  /** Julian Date of the simulation epoch, or null if not set. */
  epochJd?: number | null;
}

/**
 * Main Three.js scene component using @react-three/fiber Canvas.
 * Contains camera, controls, lights, central body, orbit trail(s), and satellite(s).
 */
export function Scene({
  points,
  satellitePosition,
  trailVisibleCount,
  trailBuffer,
  trailBuffers,
  satellitePositions,
  trailVisibleCounts,
  centralBody,
  centralBodyRadius,
  epochJd,
}: SceneProps) {
  // Determine sim time for sun direction from first available satellite position
  const firstPosition = satellitePosition
    ?? (satellitePositions ? Array.from(satellitePositions.values()).find((p) => p != null) ?? null : null);
  const simTime = firstPosition?.t ?? 0;
  const quantizedSimTime = Math.floor(simTime / 60) * 60;

  const sunDirection = useMemo(() => {
    if (epochJd == null) return DEFAULT_SUN_DIRECTION;
    const [x, y, z] = sunDirectionECI(epochJd, quantizedSimTime);
    return new THREE.Vector3(x, y, z);
  }, [epochJd, quantizedSimTime]);

  const lightPosition = useMemo<[number, number, number]>(() => {
    return [sunDirection.x * 10, sunDirection.y * 10, sunDirection.z * 10];
  }, [sunDirection]);

  // No useMemo: the trailBuffers Map reference (from useRef) never changes,
  // but Scene re-renders each frame via satellitePositions, so reading entries
  // inline picks up newly-added satellites.
  const multiSatEntries = trailBuffers
    ? Array.from(trailBuffers.entries()).filter(([, buf]) => buf.length > 0)
    : null;

  // Single-satellite backward compat
  const hasTrailData = trailBuffer
    ? trailBuffer.length > 0
    : points != null && points.length > 0;

  return (
    <Canvas
      camera={{ position: [0, 2, 5], fov: 60, near: 0.01, far: 1000 }}
      style={{ position: "absolute", top: 0, left: 0, width: "100%", height: "100%" }}
    >
      <OrbitControls
        enableDamping
        dampingFactor={0.1}
        minDistance={1.5}
        maxDistance={100}
      />

      <ambientLight intensity={0.5} color={0x404040} />
      <directionalLight intensity={2.0} position={lightPosition} />

      <CelestialBody bodyId={centralBody} sunDirection={sunDirection} />
      <axesHelper args={[2]} />

      {/* Multi-satellite mode */}
      {multiSatEntries && multiSatEntries.map(([satId, buf], index) => {
        const color = SATELLITE_COLORS[index % SATELLITE_COLORS.length];
        const vc = trailVisibleCounts?.get(satId);
        const pos = satellitePositions?.get(satId);
        return (
          <group key={satId}>
            <OrbitTrail
              trailBuffer={buf}
              visibleCount={vc}
              scaleRadius={centralBodyRadius}
              color={color}
            />
            {pos && <Satellite position={pos} scaleRadius={centralBodyRadius} color={color} />}
          </group>
        );
      })}

      {/* Single-satellite fallback (replay mode or legacy) */}
      {!multiSatEntries && hasTrailData && (
        trailBuffer ? (
          <OrbitTrail trailBuffer={trailBuffer} visibleCount={trailVisibleCount} scaleRadius={centralBodyRadius} />
        ) : (
          <OrbitTrail
            points={points!}
            visibleCount={trailVisibleCount ?? points!.length}
            scaleRadius={centralBodyRadius}
          />
        )
      )}
      {!multiSatEntries && satellitePosition && (
        <Satellite position={satellitePosition} scaleRadius={centralBodyRadius} />
      )}
    </Canvas>
  );
}
