import { useFrame } from "@react-three/fiber";
import { useEffect, useMemo, useRef } from "react";
import * as THREE from "three";
import {
  batchEciToEcef,
  batchTransformToLvlh,
  batchTransformWithOffset,
} from "../coordTransform.js";
import type { OrbitPoint } from "../orbit.js";
import { frameCenterEquals, isLegacyEcef, type ReferenceFrame } from "../referenceFrame.js";
import type { LvlhAxes } from "../sceneFrame.js";
import type { TrailBuffer } from "../utils/TrailBuffer.js";

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
  /** Trail color (default: 0x00ff88). */
  color?: number;
  /** Reference frame for display (default: central-body inertial). */
  referenceFrame?: ReferenceFrame;
  /** Julian Date of the simulation epoch (needed for ECEF transform). */
  epochJd?: number | null;
  /** Starting vertex index for the draw range (default 0). */
  drawStart?: number;
  /** Origin position in ECI [km] for the current frame center, or null for central body. */
  originPosition?: [number, number, number] | null;
  /** LVLH axes for satellite body-frame transform. When provided with originPosition,
   *  trail points are transformed into the satellite's LVLH frame for better f32 precision. */
  lvlhAxes?: LvlhAxes | null;
}

const DEFAULT_REF_FRAME: ReferenceFrame = {
  center: { type: "central_body" },
  orientation: "inertial",
};

/**
 * Orbit trajectory line component.
 *
 * Supports two data sources:
 *   - `points` + `visibleCount`: replay mode (progressive trail)
 *   - `trailBuffer`: realtime mode (bounded, generation-based GPU invalidation)
 */
export function OrbitTrail({
  points,
  visibleCount,
  trailBuffer,
  scaleRadius,
  color = 0x00ff88,
  referenceFrame = DEFAULT_REF_FRAME,
  epochJd,
  drawStart = 0,
  originPosition = null,
  lvlhAxes = null,
}: OrbitTrailProps) {
  const writtenCountRef = useRef(0);
  const capacityRef = useRef(INITIAL_CAPACITY);
  const bufferRef = useRef(new Float32Array(INITIAL_CAPACITY * 3));
  const generationRef = useRef(-1);
  const prevFrameRef = useRef<ReferenceFrame>(referenceFrame);

  // Determine data source identity for geometry recreation
  const _sourceIdentity = trailBuffer ?? points;

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
  }, [trailBuffer]);

  const material = useMemo(() => new THREE.LineBasicMaterial({ color, linewidth: 1 }), [color]);
  const lineObject = useMemo(() => new THREE.Line(geometry, material), [geometry, material]);

  useEffect(() => {
    return () => {
      geometry.dispose();
    };
  }, [geometry]);

  /** Write points[from..to) into the GPU buffer. */
  function writePoints(buf: Float32Array, src: OrbitPoint[], from: number, to: number): void {
    if (isLegacyEcef(referenceFrame) && epochJd != null) {
      // WASM fast path for central-body ECEF
      batchEciToEcef(src, from, to, epochJd, buf, from, scaleRadius);
    } else if (originPosition != null && lvlhAxes != null) {
      // Satellite body-frame: full LVLH rotation + translation in f64
      batchTransformToLvlh(src, from, to, originPosition, lvlhAxes, buf, from, scaleRadius);
    } else {
      // Generic path: subtract origin offset + scale
      batchTransformWithOffset(src, from, to, originPosition, buf, from, scaleRadius);
    }
  }

  useFrame(() => {
    // Detect reference frame change → force full rewrite
    const frameChanged =
      referenceFrame.orientation !== prevFrameRef.current.orientation ||
      !frameCenterEquals(referenceFrame.center, prevFrameRef.current.center);
    if (frameChanged) {
      prevFrameRef.current = referenceFrame;
      writtenCountRef.current = 0;
    }

    if (trailBuffer) {
      // --- TrailBuffer mode (realtime) ---
      const currentGen = trailBuffer.generation;
      const allPoints = trailBuffer.getAll();
      const totalPoints = allPoints.length;

      const needsFullRewrite =
        currentGen !== generationRef.current ||
        writtenCountRef.current === 0 ||
        // For satellite-centered, origin moves every frame → always full rewrite
        originPosition != null;

      if (needsFullRewrite) {
        generationRef.current = currentGen;
        ensureCapacity(totalPoints);

        writePoints(bufferRef.current, allPoints, 0, totalPoints);
        writtenCountRef.current = totalPoints;

        const attr = geometry.getAttribute("position") as THREE.BufferAttribute;
        attr.needsUpdate = true;
      } else if (totalPoints > writtenCountRef.current) {
        // Incremental append
        appendPoints(allPoints, writtenCountRef.current, totalPoints);
      }

      const vc = visibleCount != null ? Math.min(visibleCount, totalPoints) : totalPoints;
      const start = Math.min(drawStart, vc);
      geometry.setDrawRange(start, vc - start);
    } else if (points) {
      // --- Legacy points mode (replay) ---
      const totalPoints = points.length;

      if (originPosition != null) {
        // Satellite-centered: rewrite every frame (origin moves)
        ensureCapacity(totalPoints);
        writePoints(bufferRef.current, points, 0, totalPoints);
        writtenCountRef.current = totalPoints;
        const attr = geometry.getAttribute("position") as THREE.BufferAttribute;
        attr.needsUpdate = true;
      } else if (totalPoints > writtenCountRef.current) {
        appendPoints(points, writtenCountRef.current, totalPoints);
      }

      const clampedVc = Math.max(0, Math.min(visibleCount ?? totalPoints, totalPoints));
      const start = Math.min(drawStart, clampedVc);
      geometry.setDrawRange(start, clampedVc - start);
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
    writePoints(bufferRef.current, src, from, to);
    writtenCountRef.current = to;

    const attr = geometry.getAttribute("position") as THREE.BufferAttribute;
    attr.needsUpdate = true;
  }

  return <primitive object={lineObject} />;
}
