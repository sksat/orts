/**
 * Earth component: a blue sphere with a wireframe overlay.
 * Radius = 1 unit (represents 6378 km in scene scale).
 */
export function Earth() {
  return (
    <group>
      {/* Solid Earth sphere */}
      <mesh>
        <sphereGeometry args={[1, 64, 64]} />
        <meshPhongMaterial
          color={0x2255aa}
          emissive={0x112244}
          emissiveIntensity={0.1}
          shininess={25}
        />
      </mesh>

      {/* Wireframe overlay for reference grid */}
      <mesh>
        <sphereGeometry args={[1.002, 24, 24]} />
        <meshBasicMaterial
          color={0x4488cc}
          wireframe
          transparent
          opacity={0.15}
        />
      </mesh>
    </group>
  );
}
