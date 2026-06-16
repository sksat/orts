import { useEffect, useRef } from "react";
import type * as THREE from "three";

interface PrimitiveMarkerProps {
  /** Position in scene units (already divided by scaleRadius). */
  position: [number, number, number];
  /** Marker color. */
  color: number;
  /**
   * Body-to-display quaternion [w, x, y, z] (Hamilton scalar-first). When present,
   * the shape is oriented by it so the attitude is visible. Same convention as
   * SatelliteModel / BodyAxes.
   */
  quaternion?: [number, number, number, number];
  /** Half-length of the body along its longest (X) axis, in scene units. */
  size?: number;
}

/** Default body half-length, matching the legacy sphere marker's footprint. */
const DEFAULT_SIZE = 0.008;

/**
 * Orientation-revealing fallback marker for satellites without a 3D model.
 *
 * Unlike a sphere (rotationally symmetric → attitude invisible), this is an
 * asymmetric primitive: a box body whose three edge lengths differ, plus a cone
 * "nose" marking the body +X axis. Rendered with unlit materials so it is visible
 * under the scene's faint ambient light. Pair with {@link BodyAxes} for explicit
 * RGB axes.
 *
 * --- Customize the shape here ---
 * The geometry below is a deliberately simple default. Swap it for a bus + solar
 * paddles, a CubeSat, an arrow, etc. — anything asymmetric across all three body
 * axes. Conventions assumed by the default: +X is the "front"/boresight (cone),
 * the box is widest along X then Y then Z. Only the JSX between the markers needs
 * to change; the quaternion wiring and registration stay as-is.
 */
export function PrimitiveMarker({
  position,
  color,
  quaternion,
  size = DEFAULT_SIZE,
}: PrimitiveMarkerProps) {
  const groupRef = useRef<THREE.Group>(null);

  // Apply the body-to-display quaternion imperatively (Hamilton [w,x,y,z] →
  // Three.js (x,y,z,w)), matching SatelliteModel.tsx / BodyAxes.tsx.
  useEffect(() => {
    if (groupRef.current && quaternion) {
      const [w, x, y, z] = quaternion;
      groupRef.current.quaternion.set(x, y, z, w);
    }
  }, [quaternion]);

  return (
    <group position={position} ref={groupRef}>
      {/* --- shape begin --- */}
      <mesh>
        <boxGeometry args={[size * 2, size * 1.2, size * 0.8]} />
        <meshBasicMaterial color={color} />
      </mesh>
      {/* +X nose cone marks the body's front/boresight. coneGeometry points +Y by
          default, so rotate -90° about Z to aim it along +X. */}
      <mesh position={[size * 1.5, 0, 0]} rotation={[0, 0, -Math.PI / 2]}>
        <coneGeometry args={[size * 0.6, size, 16]} />
        <meshBasicMaterial color={color} />
      </mesh>
      {/* --- shape end --- */}
    </group>
  );
}
