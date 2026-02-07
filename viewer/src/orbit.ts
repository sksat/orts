import * as THREE from "three";

/**
 * Earth radius in km -- used as the scene scale factor.
 * Positions in CSV are expected in km; divide by this to get scene units.
 */
const EARTH_RADIUS_KM = 6378.137;

/** A single orbit state point from CSV. */
export interface OrbitPoint {
  t: number;
  x: number;
  y: number;
  z: number;
  vx: number;
  vy: number;
  vz: number;
}

/**
 * Parse CSV orbit data.
 *
 * Format:
 *   - Lines starting with '#' are comments and are skipped.
 *   - Blank lines are skipped.
 *   - Data lines: t,x,y,z,vx,vy,vz  (all numbers, positions in km, velocities in km/s)
 */
export function parseOrbitCSV(text: string): OrbitPoint[] {
  const points: OrbitPoint[] = [];

  for (const rawLine of text.split("\n")) {
    const line = rawLine.trim();
    if (line === "" || line.startsWith("#")) continue;

    const parts = line.split(",").map((s) => s.trim());
    if (parts.length < 7) continue;

    const nums = parts.map(Number);
    if (nums.some(isNaN)) continue;

    points.push({
      t: nums[0],
      x: nums[1],
      y: nums[2],
      z: nums[3],
      vx: nums[4],
      vy: nums[5],
      vz: nums[6],
    });
  }

  return points;
}

/**
 * Holds the Three.js objects for a rendered orbit so they can be
 * removed from the scene when a new orbit is loaded.
 */
export interface OrbitVisualization {
  orbitLine: THREE.Line;
  satelliteMarker: THREE.Mesh;
}

/**
 * Create Three.js objects for the orbit trajectory and satellite marker.
 *
 * @param points - Parsed orbit points (positions in km)
 * @returns The line and marker meshes to be added to the scene
 */
export function createOrbitVisualization(
  points: OrbitPoint[]
): OrbitVisualization {
  // Convert positions from km to scene units (Earth radii)
  const vertices: number[] = [];
  for (const p of points) {
    vertices.push(
      p.x / EARTH_RADIUS_KM,
      p.y / EARTH_RADIUS_KM,
      p.z / EARTH_RADIUS_KM
    );
  }

  // Orbit trajectory line
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute(
    "position",
    new THREE.Float32BufferAttribute(vertices, 3)
  );

  const material = new THREE.LineBasicMaterial({
    color: 0x00ff88,
    linewidth: 1,
  });

  const orbitLine = new THREE.Line(geometry, material);

  // Satellite marker at the last position
  const lastPoint = points[points.length - 1];
  const markerGeometry = new THREE.SphereGeometry(0.03, 16, 16);
  const markerMaterial = new THREE.MeshBasicMaterial({ color: 0xff4444 });
  const satelliteMarker = new THREE.Mesh(markerGeometry, markerMaterial);
  satelliteMarker.position.set(
    lastPoint.x / EARTH_RADIUS_KM,
    lastPoint.y / EARTH_RADIUS_KM,
    lastPoint.z / EARTH_RADIUS_KM
  );

  return { orbitLine, satelliteMarker };
}

/**
 * Update the satellite marker position to reflect a given orbit point.
 *
 * @param marker - The satellite mesh to reposition
 * @param point  - The orbit state to move to (position in km)
 */
export function updateSatellitePosition(
  marker: THREE.Mesh,
  point: OrbitPoint
): void {
  marker.position.set(
    point.x / EARTH_RADIUS_KM,
    point.y / EARTH_RADIUS_KM,
    point.z / EARTH_RADIUS_KM
  );
}

/**
 * Update the orbit line's draw range so that only the trail up to (and
 * including) `visibleCount` vertices is rendered.
 *
 * Call with `visibleCount = points.length` to show the full orbit, or a
 * smaller value for a progressive trail effect during playback.
 *
 * @param line         - The THREE.Line whose geometry to update
 * @param visibleCount - Number of vertices to render (clamped to valid range)
 * @param totalCount   - Total number of vertices in the geometry
 */
export function updateOrbitTrail(
  line: THREE.Line,
  visibleCount: number,
  totalCount: number
): void {
  const clamped = Math.max(0, Math.min(visibleCount, totalCount));
  line.geometry.setDrawRange(0, clamped);
}
