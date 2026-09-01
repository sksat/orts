import { useEffect, useMemo, useRef } from "react";
import * as THREE from "three";
import { type DrawnArrow, registerDirectionVectors } from "../debug/directionVectors.js";
import type { DirectionVector } from "../directionVectors.js";
import type { Vec3 } from "../displayFrame.js";
import { type ArrowGeometry, arrowGeometryForSpan } from "../spacecraftScale.js";

/** Cylinder and cone geometries are built along +Y; arrows are rotated from it. */
const GEOMETRY_AXIS = new THREE.Vector3(0, 1, 0);

/** Radial segments. Arrows are thin annotations, not subjects. */
const SHAFT_SEGMENTS = 8;
const HEAD_SEGMENTS = 12;

interface ArrowProps extends ArrowGeometry {
  /** Unit vector in scene axes. */
  direction: Vec3;
  color: number;
  /** Receives the head mesh, whose world position gives the drawn direction. */
  headRef?: (mesh: THREE.Object3D | null) => void;
}

/**
 * One arrow: a shaft and a head, unlit so it reads at any Sun angle.
 *
 * Meshes rather than `THREE.ArrowHelper` because the shaft needs a thickness —
 * WebGL ignores `linewidth`, so a helper's shaft is always one pixel wide.
 */
function Arrow({
  direction,
  color,
  length,
  shaftRadius,
  headLength,
  headRadius,
  startOffset,
  headRef,
}: ArrowProps) {
  const quaternion = useMemo(
    () =>
      new THREE.Quaternion().setFromUnitVectors(
        GEOMETRY_AXIS,
        new THREE.Vector3(...direction).normalize(),
      ),
    [direction],
  );

  // A head longer than the arrow would invert the shaft; clamp instead of
  // drawing a mesh with negative height.
  const head = Math.min(headLength, length);
  const shaftLength = length - head;

  return (
    <group quaternion={quaternion}>
      {shaftLength > 0 && (
        <mesh position={[0, startOffset + shaftLength / 2, 0]}>
          <cylinderGeometry args={[shaftRadius, shaftRadius, shaftLength, SHAFT_SEGMENTS]} />
          <meshBasicMaterial color={color} />
        </mesh>
      )}
      <mesh ref={headRef} position={[0, startOffset + shaftLength + head / 2, 0]}>
        <coneGeometry args={[headRadius, head, HEAD_SEGMENTS]} />
        <meshBasicMaterial color={color} />
      </mesh>
    </group>
  );
}

interface DirectionArrowsProps {
  /** Spacecraft position in scene units — where the arrows start. */
  position: Vec3;
  /** Directions in scene axes, as resolved by `resolveDirectionVectors`. */
  vectors: readonly DirectionVector[];
  /** The spacecraft's apparent size in scene units, which sets the proportions. */
  visualSpan: number;
  /**
   * Optional spacecraft id. When set, the *rendered* directions are published
   * (dev/E2E only) via `window.__debug_get_direction_vectors(id)`.
   */
  debugId?: string;
}

/**
 * Reference-direction arrows drawn at a spacecraft.
 *
 * Shared by both views: the attitude view draws them at the origin, the orbit
 * view at the centred satellite's scene position. Proportions come from the
 * spacecraft's apparent size, so the same component works in a scene measured in
 * central-body radii and in one measured in spacecraft spans.
 */
export function DirectionArrows({ position, vectors, visualSpan, debugId }: DirectionArrowsProps) {
  const geometry = useMemo(() => arrowGeometryForSpan(visualSpan), [visualSpan]);
  const groupRef = useRef<THREE.Group>(null);
  const headsRef = useRef(new Map<string, THREE.Object3D>());

  useEffect(() => {
    if (debugId == null) return;
    const heads = headsRef.current;
    return registerDirectionVectors(debugId, () =>
      vectors.map<DrawnArrow>((v) => ({
        kind: v.kind,
        origin: groupRef.current,
        head: heads.get(v.kind) ?? null,
      })),
    );
  }, [debugId, vectors]);

  if (vectors.length === 0) return null;

  return (
    <group ref={groupRef} position={position}>
      {vectors.map((v) => (
        <Arrow
          key={v.kind}
          direction={v.direction}
          color={v.color}
          headRef={(mesh) => {
            if (mesh == null) headsRef.current.delete(v.kind);
            else headsRef.current.set(v.kind, mesh);
          }}
          {...geometry}
        />
      ))}
    </group>
  );
}
