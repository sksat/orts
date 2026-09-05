import { useEffect, useRef } from "react";
import type * as THREE from "three";
import { DEFAULT_CUBE_HALF_EXTENT } from "../spacecraftScale.js";

interface PrimitiveMarkerProps {
  /** Position in scene units (already divided by scaleRadius). */
  position: [number, number, number];
  /**
   * Body-to-display quaternion [w, x, y, z] (Hamilton scalar-first). When present,
   * the cube is oriented by it so the attitude is visible. Same convention as
   * SatelliteModel / BodyAxes.
   */
  quaternion?: [number, number, number, number];
  /** Half-extent of the cube in scene units. */
  size?: number;
}

/**
 * Default half-extent. Larger than the sphere marker's radius: a cube read at an
 * angle presents less silhouette than a sphere of the same radius, so matching
 * radii would make the orientation cube look like the smaller marker.
 */
const DEFAULT_SIZE = DEFAULT_CUBE_HALF_EXTENT;

/**
 * Per-face colors of the XYZ orientation cube, in Three.js BoxGeometry face order
 * [+X, -X, +Y, -Y, +Z, -Z]. Positive faces use the bright RGB axis colors (X=red,
 * Y=green, Z=blue, matching {@link BodyAxes}); negative faces are dimmed so each of
 * the six faces — hence the full orientation — is unambiguous at a glance.
 */
const AXES_CUBE_FACE_COLORS = [0xff4444, 0x802222, 0x44ff44, 0x228022, 0x4488ff, 0x224488] as const;

/**
 * Orientation-revealing fallback marker for satellites without a 3D model: an XYZ
 * cube whose six faces are colored per body axis. A sphere is rotationally
 * symmetric → attitude invisible; this cube reads orientation at a glance. Unlit
 * materials so it is visible under the scene's faint ambient light. Pair with
 * {@link BodyAxes} for explicit RGB axes.
 */
export function PrimitiveMarker({
  position,
  quaternion,
  size = DEFAULT_SIZE,
}: PrimitiveMarkerProps) {
  const groupRef = useRef<THREE.Group>(null);

  // Apply the body-to-display quaternion imperatively (Hamilton [w,x,y,z] →
  // Three.js (x,y,z,w)), matching SatelliteModel.tsx / BodyAxes.tsx.
  useEffect(() => {
    if (!groupRef.current) return;
    if (quaternion) {
      const [w, x, y, z] = quaternion;
      groupRef.current.quaternion.set(x, y, z, w);
    } else {
      // A stream can stop carrying a usable attitude — a sample whose quaternion
      // names no rotation, or none at all after one that did. Leaving the group
      // where the last good sample put it would show that orientation as the
      // current one, so it goes back to unrotated, which is what "no attitude"
      // draws from the start.
      groupRef.current.quaternion.identity();
    }
  }, [quaternion]);

  return (
    <group position={position} ref={groupRef}>
      <mesh>
        <boxGeometry args={[size * 2, size * 2, size * 2]} />
        {AXES_CUBE_FACE_COLORS.map((c, i) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: fixed-length face list, index is the face id.
          <meshBasicMaterial key={i} attach={`material-${i}`} color={c} />
        ))}
      </mesh>
    </group>
  );
}
