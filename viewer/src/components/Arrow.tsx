import { useMemo } from "react";
import * as THREE from "three";
import type { Vec3 } from "../displayFrame.js";
import type { ArrowGeometry } from "../spacecraftScale.js";

/** Cylinder and cone geometries are built along +Y; arrows are rotated from it. */
const GEOMETRY_AXIS = new THREE.Vector3(0, 1, 0);

const SHAFT_SEGMENTS = 8;
const HEAD_SEGMENTS = 12;

export interface ArrowProps extends ArrowGeometry {
  /** Unit vector in the parent's axes. */
  direction: Vec3;
  color: number;
  /** Below 1 the arrow is drawn transparent — for a subordinate reference. */
  opacity?: number;
  /** Receives the head mesh, whose world position gives the drawn direction. */
  headRef?: (mesh: THREE.Object3D | null) => void;
}

/**
 * One arrow: a shaft and a head, unlit so it reads at any Sun angle.
 *
 * Meshes rather than `THREE.ArrowHelper` or `AxesHelper` because the shaft needs
 * a thickness — WebGL ignores `linewidth`, so a helper's line is one pixel wide
 * whatever the scene's scale, and at a distance it disappears.
 */
export function Arrow({
  direction,
  color,
  length,
  shaftRadius,
  headLength,
  headRadius,
  startOffset,
  opacity = 1,
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
  const transparent = opacity < 1;

  return (
    <group quaternion={quaternion}>
      {shaftLength > 0 && (
        <mesh position={[0, startOffset + shaftLength / 2, 0]}>
          <cylinderGeometry args={[shaftRadius, shaftRadius, shaftLength, SHAFT_SEGMENTS]} />
          <meshBasicMaterial color={color} transparent={transparent} opacity={opacity} />
        </mesh>
      )}
      <mesh ref={headRef} position={[0, startOffset + shaftLength + head / 2, 0]}>
        <coneGeometry args={[headRadius, head, HEAD_SEGMENTS]} />
        <meshBasicMaterial color={color} transparent={transparent} opacity={opacity} />
      </mesh>
    </group>
  );
}
