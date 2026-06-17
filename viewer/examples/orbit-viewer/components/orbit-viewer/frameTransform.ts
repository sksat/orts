export function rotateZ(x: number, y: number, z: number, angle: number): [number, number, number] {
  const c = Math.cos(angle);
  const s = Math.sin(angle);
  return [x * c - y * s, x * s + y * c, z];
}
