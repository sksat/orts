import { useRef, useEffect, useMemo } from "react";
import { useFrame } from "@react-three/fiber";
import * as THREE from "three";
import { OrbitPoint } from "../orbit.js";
import { TrailBuffer } from "../utils/TrailBuffer.js";

/** Initial capacity for the streaming vertex buffer. Grows as needed. */
const INITIAL_CAPACITY = 2048;

interface OrbitTrailProps {
  /** Points array for replay mode. */
  points?: OrbitPoint[];
  /** Number of vertices to render (replay mode progressive trail). */
  visibleCount?: number;
  /** TrailBuffer for realtime mode (bounded, generation-based invalidation). */
  trailBuffer?: TrailBuffer;
  /** Central body radius in km, used as the scale factor. */
  scaleRadius: number;
}

/**
 * Orbit trajectory line component.
 *
 * Supports two data sources:
 *   - `points` + `visibleCount`: replay mode (progressive trail)
 *   - `trailBuffer`: realtime mode (bounded, generation-based GPU invalidation)
 */
export function OrbitTrail({ points, visibleCount, trailBuffer, scaleRadius }: OrbitTrailProps) {
  const writtenCountRef = useRef(0);
  const capacityRef = useRef(INITIAL_CAPACITY);
  const bufferRef = useRef(new Float32Array(INITIAL_CAPACITY * 3));
  const generationRef = useRef(-1);

  // Determine data source identity for geometry recreation
  const sourceIdentity = trailBuffer ?? points;

  const geometry = useMemo(() => {
    bufferRef.current = new Float32Array(INITIAL_CAPACITY * 3);
    capacityRef.current = INITIAL_CAPACITY;
    writtenCountRef.current = 0;
    generationRef.current = trailBuffer ? trailBuffer.generation : -1;

    const geom = new THREE.BufferGeometry();
    const attr = new THREE.BufferAttribute(bufferRef.current, 3);
    attr.setUsage(THREE.DynamicDrawUsage);
    geom.setAttribute("position", attr);
    geom.setDrawRange(0, 0);
    return geom;
  }, [sourceIdentity]);

  const material = useMemo(
    () => new THREE.LineBasicMaterial({ color: 0x00ff88, linewidth: 1 }),
    []
  );
  const lineObject = useMemo(() => new THREE.Line(geometry, material), [geometry, material]);

  useEffect(() => {
    return () => {
      geometry.dispose();
    };
  }, [geometry]);

  useFrame(() => {
    if (trailBuffer) {
      // --- TrailBuffer mode (realtime) ---
      const currentGen = trailBuffer.generation;
      const allPoints = trailBuffer.getAll();
      const totalPoints = allPoints.length;

      if (currentGen !== generationRef.current) {
        // Generation changed (trim or clear) — full rewrite
        generationRef.current = currentGen;
        ensureCapacity(totalPoints);

        const buf = bufferRef.current;
        for (let i = 0; i < totalPoints; i++) {
          const p = allPoints[i];
          const off = i * 3;
          buf[off] = p.x / scaleRadius;
          buf[off + 1] = p.y / scaleRadius;
          buf[off + 2] = p.z / scaleRadius;
        }
        writtenCountRef.current = totalPoints;

        const attr = geometry.getAttribute("position") as THREE.BufferAttribute;
        attr.needsUpdate = true;
      } else if (totalPoints > writtenCountRef.current) {
        // Incremental append
        appendPoints(allPoints, writtenCountRef.current, totalPoints);
      }

      const vc = visibleCount != null ? Math.min(visibleCount, totalPoints) : totalPoints;
      geometry.setDrawRange(0, vc);
    } else if (points) {
      // --- Legacy points mode (replay) ---
      const totalPoints = points.length;
      if (totalPoints > writtenCountRef.current) {
        appendPoints(points, writtenCountRef.current, totalPoints);
      }

      const vc = visibleCount ?? totalPoints;
      geometry.setDrawRange(0, Math.max(0, Math.min(vc, totalPoints)));
    }
  });

  /** Ensure GPU buffer can hold `needed` points; grows if necessary. */
  function ensureCapacity(needed: number): void {
    if (needed <= capacityRef.current) return;
    const newCap = Math.max(needed * 2, capacityRef.current * 2);
    const newBuf = new Float32Array(newCap * 3);
    newBuf.set(bufferRef.current);
    bufferRef.current = newBuf;
    capacityRef.current = newCap;

    const attr = new THREE.BufferAttribute(newBuf, 3);
    attr.setUsage(THREE.DynamicDrawUsage);
    geometry.setAttribute("position", attr);
  }

  /** Append points[from..to) to the GPU buffer. */
  function appendPoints(src: OrbitPoint[], from: number, to: number): void {
    ensureCapacity(to);
    const buf = bufferRef.current;
    for (let i = from; i < to; i++) {
      const p = src[i];
      const off = i * 3;
      buf[off] = p.x / scaleRadius;
      buf[off + 1] = p.y / scaleRadius;
      buf[off + 2] = p.z / scaleRadius;
    }
    writtenCountRef.current = to;

    const attr = geometry.getAttribute("position") as THREE.BufferAttribute;
    attr.needsUpdate = true;
  }

  return <primitive object={lineObject} />;
}
