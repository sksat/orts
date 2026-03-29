import { useFrame } from "@react-three/fiber";
import { useEffect, useMemo, useRef } from "react";
import * as THREE from "three";
import {
  batchEncodeEcefHighLow,
  batchEncodeEciHighLow,
  encodeFloat64ToHighLow,
} from "../coordTransform.js";
import type { OrbitPoint } from "../orbit.js";
import { frameCenterEquals, isLegacyEcef, type ReferenceFrame } from "../referenceFrame.js";
import type { LvlhAxes } from "../sceneFrame.js";
import { orbitTrailFrag, orbitTrailVert } from "../shaders/orbitTrail.js";
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

const IDENTITY_MAT3 = new THREE.Matrix3();

/**
 * Orbit trajectory line component.
 *
 * Uses a custom ShaderMaterial with high/low split vertex attributes
 * for f64-level precision on the GPU. Origin subtraction and frame
 * rotation are applied in the vertex shader via uniforms, so mode
 * switches (ECI, ECEF, LVLH) are O(1) per frame.
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
  const positionHighRef = useRef(new Float32Array(INITIAL_CAPACITY * 3));
  const positionLowRef = useRef(new Float32Array(INITIAL_CAPACITY * 3));
  const generationRef = useRef(-1);
  const prevFrameRef = useRef<ReferenceFrame>(referenceFrame);
  const prevPointsRef = useRef<OrbitPoint[] | undefined>(points);

  // --- Geometry with dual high/low attributes ---
  const geometry = useMemo(() => {
    positionHighRef.current = new Float32Array(INITIAL_CAPACITY * 3);
    positionLowRef.current = new Float32Array(INITIAL_CAPACITY * 3);
    capacityRef.current = INITIAL_CAPACITY;
    writtenCountRef.current = 0;
    generationRef.current = trailBuffer ? trailBuffer.generation : -1;

    const geom = new THREE.BufferGeometry();
    const highAttr = new THREE.BufferAttribute(positionHighRef.current, 3);
    highAttr.setUsage(THREE.DynamicDrawUsage);
    const lowAttr = new THREE.BufferAttribute(positionLowRef.current, 3);
    lowAttr.setUsage(THREE.DynamicDrawUsage);
    geom.setAttribute("positionHigh", highAttr);
    geom.setAttribute("positionLow", lowAttr);
    geom.setDrawRange(0, 0);
    return geom;
  }, [trailBuffer]);

  // --- ShaderMaterial (stable, uniforms updated separately) ---
  const material = useMemo(
    () =>
      new THREE.ShaderMaterial({
        uniforms: {
          uOriginHigh: { value: new THREE.Vector3(0, 0, 0) },
          uOriginLow: { value: new THREE.Vector3(0, 0, 0) },
          uFrameRotation: { value: new THREE.Matrix3() },
          uInvScaleRadius: { value: 1 / scaleRadius },
          uColor: { value: new THREE.Color(color) },
          uOpacity: { value: 1.0 },
        },
        vertexShader: orbitTrailVert,
        fragmentShader: orbitTrailFrag,
      }),
    [],
  );

  const lineObject = useMemo(() => {
    const line = new THREE.Line(geometry, material);
    line.frustumCulled = false;
    return line;
  }, [geometry, material]);

  useEffect(() => {
    return () => {
      geometry.dispose();
    };
  }, [geometry]);

  useEffect(() => {
    return () => {
      material.dispose();
    };
  }, [material]);

  // --- Uniform updates ---
  // scaleRadius and color change rarely → useEffect is fine.
  useEffect(() => {
    material.uniforms.uInvScaleRadius.value = 1 / scaleRadius;
  }, [material, scaleRadius]);

  useEffect(() => {
    material.uniforms.uColor.value.set(color);
  }, [material, color]);

  // originPosition and lvlhAxes change every frame in satellite-centered/LVLH mode.
  // They MUST be updated in useFrame (before render), not useEffect (after render),
  // to avoid a one-frame lag where the trail is drawn with stale transform.
  const originPositionRef = useRef(originPosition);
  originPositionRef.current = originPosition;
  const lvlhAxesRef = useRef(lvlhAxes);
  lvlhAxesRef.current = lvlhAxes;

  // --- Unified writePoints: encode source coordinates into high/low buffers ---
  function writePoints(src: OrbitPoint[], from: number, to: number): void {
    if (isLegacyEcef(referenceFrame) && epochJd != null) {
      batchEncodeEcefHighLow(
        src,
        from,
        to,
        epochJd,
        positionHighRef.current,
        positionLowRef.current,
        from,
      );
    } else {
      batchEncodeEciHighLow(src, from, to, positionHighRef.current, positionLowRef.current, from);
    }
  }

  useFrame(() => {
    // Update per-frame uniforms (origin + rotation) before rendering.
    // In ECEF mode, vertices are in body-fixed frame — origin/rotation uniforms
    // must be identity to avoid mixing coordinate frames.
    const isEcef = isLegacyEcef(referenceFrame);
    const curOrigin = !isEcef ? originPositionRef.current : null;
    if (curOrigin != null) {
      const [xh, xl] = encodeFloat64ToHighLow(curOrigin[0]);
      const [yh, yl] = encodeFloat64ToHighLow(curOrigin[1]);
      const [zh, zl] = encodeFloat64ToHighLow(curOrigin[2]);
      material.uniforms.uOriginHigh.value.set(xh, yh, zh);
      material.uniforms.uOriginLow.value.set(xl, yl, zl);
    } else {
      material.uniforms.uOriginHigh.value.set(0, 0, 0);
      material.uniforms.uOriginLow.value.set(0, 0, 0);
    }

    const curAxes = !isEcef ? lvlhAxesRef.current : null;
    if (curAxes != null) {
      const m = material.uniforms.uFrameRotation.value as THREE.Matrix3;
      m.set(
        curAxes.inTrack[0],
        curAxes.inTrack[1],
        curAxes.inTrack[2],
        curAxes.crossTrack[0],
        curAxes.crossTrack[1],
        curAxes.crossTrack[2],
        curAxes.radial[0],
        curAxes.radial[1],
        curAxes.radial[2],
      );
    } else {
      (material.uniforms.uFrameRotation.value as THREE.Matrix3).copy(IDENTITY_MAT3);
    }

    // Detect reference frame change (ECI ↔ ECEF) → force full rewrite
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

      // Origin/rotation changes are uniform-only — no full rewrite needed.
      const needsFullRewrite =
        currentGen !== generationRef.current || writtenCountRef.current === 0;

      if (needsFullRewrite) {
        generationRef.current = currentGen;
        ensureCapacity(totalPoints);
        writePoints(allPoints, 0, totalPoints);
        writtenCountRef.current = totalPoints;
        markAttrsNeedUpdate();
      } else if (totalPoints > writtenCountRef.current) {
        appendPoints(allPoints, writtenCountRef.current, totalPoints);
      }

      const vc = visibleCount != null ? Math.min(visibleCount, totalPoints) : totalPoints;
      const start = Math.min(drawStart, vc);
      geometry.setDrawRange(start, vc - start);
    } else if (points) {
      // --- Legacy points mode (replay) ---
      const totalPoints = points.length;

      // Detect when the points array itself changes (different orbit data).
      const pointsChanged = points !== prevPointsRef.current;
      if (pointsChanged) {
        prevPointsRef.current = points;
        writtenCountRef.current = 0;
      }

      if (totalPoints > writtenCountRef.current) {
        appendPoints(points, writtenCountRef.current, totalPoints);
      }

      const clampedVc = Math.max(0, Math.min(visibleCount ?? totalPoints, totalPoints));
      const start = Math.min(drawStart, clampedVc);
      geometry.setDrawRange(start, clampedVc - start);
    }
  });

  /** Ensure GPU buffers can hold `needed` points; grows if necessary. */
  function ensureCapacity(needed: number): void {
    if (needed <= capacityRef.current) return;
    const newCap = Math.max(needed * 2, capacityRef.current * 2);

    const newHigh = new Float32Array(newCap * 3);
    newHigh.set(positionHighRef.current);
    positionHighRef.current = newHigh;

    const newLow = new Float32Array(newCap * 3);
    newLow.set(positionLowRef.current);
    positionLowRef.current = newLow;

    capacityRef.current = newCap;

    const highAttr = new THREE.BufferAttribute(newHigh, 3);
    highAttr.setUsage(THREE.DynamicDrawUsage);
    geometry.setAttribute("positionHigh", highAttr);

    const lowAttr = new THREE.BufferAttribute(newLow, 3);
    lowAttr.setUsage(THREE.DynamicDrawUsage);
    geometry.setAttribute("positionLow", lowAttr);
  }

  /** Append points[from..to) to the GPU buffers. */
  function appendPoints(src: OrbitPoint[], from: number, to: number): void {
    ensureCapacity(to);
    writePoints(src, from, to);
    writtenCountRef.current = to;
    markAttrsNeedUpdate();
  }

  function markAttrsNeedUpdate(): void {
    (geometry.getAttribute("positionHigh") as THREE.BufferAttribute).needsUpdate = true;
    (geometry.getAttribute("positionLow") as THREE.BufferAttribute).needsUpdate = true;
  }

  return <primitive object={lineObject} />;
}
