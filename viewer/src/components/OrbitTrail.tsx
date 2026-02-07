import { useRef, useEffect, useMemo } from "react";
import { useFrame } from "@react-three/fiber";
import * as THREE from "three";
import { OrbitPoint } from "../orbit.js";

/** Initial capacity for the streaming vertex buffer. Grows as needed. */
const INITIAL_CAPACITY = 2048;

interface OrbitTrailProps {
  points: OrbitPoint[];
  /** Number of vertices to render (for progressive trail during playback). */
  visibleCount: number;
  /** Central body radius in km, used as the scale factor. */
  scaleRadius: number;
}

/**
 * Orbit trajectory line component.
 *
 * Uses a pre-allocated buffer with dynamic draw usage so that new points
 * can be appended cheaply during realtime streaming (via `useFrame`),
 * instead of recreating the entire geometry on every React render.
 */
export function OrbitTrail({ points, visibleCount, scaleRadius }: OrbitTrailProps) {
  const writtenCountRef = useRef(0);
  const capacityRef = useRef(INITIAL_CAPACITY);
  const bufferRef = useRef(new Float32Array(INITIAL_CAPACITY * 3));

  // Create geometry synchronously (useMemo) so it's available on first render.
  // Recreated only when the points array identity changes (reconnect / CSV load).
  const geometry = useMemo(() => {
    bufferRef.current = new Float32Array(INITIAL_CAPACITY * 3);
    capacityRef.current = INITIAL_CAPACITY;
    writtenCountRef.current = 0;

    const geom = new THREE.BufferGeometry();
    const attr = new THREE.BufferAttribute(bufferRef.current, 3);
    attr.setUsage(THREE.DynamicDrawUsage);
    geom.setAttribute("position", attr);
    geom.setDrawRange(0, 0);
    return geom;
  }, [points]);

  // Create the Line object imperatively. useMemo ensures it's only created
  // when the geometry changes.
  const material = useMemo(
    () => new THREE.LineBasicMaterial({ color: 0x00ff88, linewidth: 1 }),
    []
  );
  const lineObject = useMemo(() => new THREE.Line(geometry, material), [geometry, material]);

  // Dispose old geometry when a new one is created.
  useEffect(() => {
    return () => {
      geometry.dispose();
    };
  }, [geometry]);

  // Sync points into the buffer each frame — O(new points) per frame.
  useFrame(() => {
    const totalPoints = points.length;
    const written = writtenCountRef.current;

    if (totalPoints > written) {
      // Grow buffer if needed
      if (totalPoints > capacityRef.current) {
        const newCap = Math.max(totalPoints * 2, capacityRef.current * 2);
        const newBuf = new Float32Array(newCap * 3);
        newBuf.set(bufferRef.current);
        bufferRef.current = newBuf;
        capacityRef.current = newCap;

        const attr = new THREE.BufferAttribute(newBuf, 3);
        attr.setUsage(THREE.DynamicDrawUsage);
        geometry.setAttribute("position", attr);
      }

      const buf = bufferRef.current;
      for (let i = written; i < totalPoints; i++) {
        const p = points[i];
        const off = i * 3;
        buf[off] = p.x / scaleRadius;
        buf[off + 1] = p.y / scaleRadius;
        buf[off + 2] = p.z / scaleRadius;
      }

      writtenCountRef.current = totalPoints;

      const attr = geometry.getAttribute("position") as THREE.BufferAttribute;
      attr.needsUpdate = true;
    }

    const clamped = Math.max(0, Math.min(visibleCount, totalPoints));
    geometry.setDrawRange(0, clamped);
  });

  return <primitive object={lineObject} />;
}
