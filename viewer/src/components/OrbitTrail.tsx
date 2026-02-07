import { useRef, useEffect, useMemo } from "react";
import * as THREE from "three";
import { OrbitPoint } from "../orbit.js";

/** Earth radius in km -- same scale factor as orbit.ts. */
const EARTH_RADIUS_KM = 6378.137;

interface OrbitTrailProps {
  points: OrbitPoint[];
  /** Number of vertices to render (for progressive trail during playback). */
  visibleCount: number;
}

/**
 * Orbit trajectory line component.
 * Renders orbit points as a green line with configurable draw range
 * for progressive trail display during playback.
 */
export function OrbitTrail({ points, visibleCount }: OrbitTrailProps) {
  const lineRef = useRef<THREE.Line>(null);

  const geometry = useMemo(() => {
    const vertices: number[] = [];
    for (const p of points) {
      vertices.push(
        p.x / EARTH_RADIUS_KM,
        p.y / EARTH_RADIUS_KM,
        p.z / EARTH_RADIUS_KM
      );
    }
    const geom = new THREE.BufferGeometry();
    geom.setAttribute(
      "position",
      new THREE.Float32BufferAttribute(vertices, 3)
    );
    return geom;
  }, [points]);

  // Update draw range when visibleCount changes
  useEffect(() => {
    if (lineRef.current) {
      const clamped = Math.max(0, Math.min(visibleCount, points.length));
      lineRef.current.geometry.setDrawRange(0, clamped);
    }
  }, [visibleCount, points.length]);

  // Use "threeLine" -- R3F's alias for THREE.Line -- to avoid conflict
  // with the SVG <line> intrinsic element in React's JSX typings.
  return (
    <threeLine ref={lineRef} geometry={geometry}>
      <lineBasicMaterial color={0x00ff88} linewidth={1} />
    </threeLine>
  );
}
