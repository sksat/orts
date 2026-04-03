import * as THREE from "three";

/**
 * Earth radius in km -- used as the scene scale factor.
 * Positions in CSV are expected in km; divide by this to get scene units.
 */
const EARTH_RADIUS_KM = 6378.137;

/** A single orbit state point from CSV or WebSocket. */
export interface OrbitPoint {
  /** Entity path identifier (from WebSocket protocol). */
  entityPath?: string;
  t: number;
  x: number;
  y: number;
  z: number;
  vx: number;
  vy: number;
  vz: number;
  /** Semi-major axis [km] */
  a: number;
  /** Eccentricity [-] */
  e: number;
  /** Inclination [rad] */
  inc: number;
  /** Right ascension of ascending node [rad] */
  raan: number;
  /** Argument of periapsis [rad] */
  omega: number;
  /** True anomaly [rad] */
  nu: number;
  /** Pre-computed derived values from server (for chart display). */
  altitude?: number;
  specific_energy?: number;
  angular_momentum?: number;
  velocity_mag?: number;
  /** Acceleration magnitudes [km/s²] — 0 when perturbation is inactive. */
  accel_gravity?: number;
  accel_drag?: number;
  accel_srp?: number;
  accel_third_body_sun?: number;
  accel_third_body_moon?: number;
  /** Body-to-inertial quaternion components (Hamilton scalar-first: w,x,y,z). */
  qw?: number;
  qx?: number;
  qy?: number;
  qz?: number;
  /** Angular velocity in body frame [rad/s]. */
  wx?: number;
  wy?: number;
  wz?: number;
}

/** Metadata parsed from CSV comment headers. */
export interface CSVMetadata {
  epochJd: number | null;
  mu: number | null;
  centralBody: string | null;
  centralBodyRadius: number | null;
  satelliteName: string | null;
  /** Multi-satellite CSV: list of satellite IDs from `# satellites = ...` */
  satellites: string[] | null;
}

/** Result of parsing a CSV file: points + optional metadata. */
export interface ParsedCSV {
  points: OrbitPoint[];
  metadata: CSVMetadata;
}

/**
 * Parse CSV orbit data with metadata extraction from comment headers.
 *
 * Format:
 *   - Lines starting with '#' are comments; `# key = value` lines are parsed as metadata.
 *   - Blank lines are skipped.
 *   - Data lines: t,x,y,z,vx,vy,vz  (all numbers, positions in km, velocities in km/s)
 */
export function parseOrbitCSVWithMetadata(text: string): ParsedCSV {
  const points: OrbitPoint[] = [];
  const metadata: CSVMetadata = {
    epochJd: null,
    mu: null,
    centralBody: null,
    centralBodyRadius: null,
    satelliteName: null,
    satellites: null,
  };

  // First pass: extract metadata from comment lines
  const lines = text.split("\n");
  for (const rawLine of lines) {
    const line = rawLine.trim();
    if (line === "") continue;
    if (!line.startsWith("#")) break;

    const match = line.match(/^#\s*(\w+)\s*=\s*(.+)/);
    if (match) {
      const [, key, value] = match;
      switch (key) {
        case "epoch_jd":
          metadata.epochJd = Number(value.trim());
          break;
        case "mu":
          metadata.mu = Number(value.trim().split(/\s/)[0]);
          break;
        case "central_body":
          metadata.centralBody = value.trim();
          break;
        case "central_body_radius":
          metadata.centralBodyRadius = Number(value.trim().split(/\s/)[0]);
          break;
        case "satellite": {
          const trimmed = value.trim();
          if (trimmed) metadata.satelliteName = trimmed;
          break;
        }
        case "satellites":
          metadata.satellites = value
            .split(",")
            .map((s) => s.trim())
            .filter((s) => s.length > 0);
          break;
      }
    }
  }

  // Detect multi-satellite mode
  const multiSat = metadata.satellites != null && metadata.satellites.length > 0;

  // Second pass: parse data lines
  for (const rawLine of lines) {
    const line = rawLine.trim();
    if (line === "" || line.startsWith("#")) continue;

    let entityPath: string | undefined;
    let numericParts: string[];
    const parts = line.split(",").map((s) => s.trim());

    if (multiSat) {
      if (parts.length < 8) continue;
      entityPath = parts[0];
      numericParts = parts.slice(1);
    } else {
      if (parts.length < 7) continue;
      numericParts = parts;
    }

    const nums = numericParts.map(Number);
    if (nums.some(Number.isNaN)) continue;

    points.push({
      t: nums[0],
      x: nums[1],
      y: nums[2],
      z: nums[3],
      vx: nums[4],
      vy: nums[5],
      vz: nums[6],
      a: nums[7] ?? 0,
      e: nums[8] ?? 0,
      inc: nums[9] ?? 0,
      raan: nums[10] ?? 0,
      omega: nums[11] ?? 0,
      nu: nums[12] ?? 0,
      entityPath,
      accel_gravity: 0,
      accel_drag: 0,
      accel_srp: 0,
      accel_third_body_sun: 0,
      accel_third_body_moon: 0,
    });
  }

  return { points, metadata };
}

/**
 * Parse CSV orbit data (legacy wrapper, ignores metadata).
 */
export function parseOrbitCSV(text: string): OrbitPoint[] {
  return parseOrbitCSVWithMetadata(text).points;
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
export function createOrbitVisualization(points: OrbitPoint[]): OrbitVisualization {
  // Convert positions from km to scene units (Earth radii)
  const vertices: number[] = [];
  for (const p of points) {
    vertices.push(p.x / EARTH_RADIUS_KM, p.y / EARTH_RADIUS_KM, p.z / EARTH_RADIUS_KM);
  }

  // Orbit trajectory line
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(vertices, 3));

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
    lastPoint.z / EARTH_RADIUS_KM,
  );

  return { orbitLine, satelliteMarker };
}

/**
 * Update the satellite marker position to reflect a given orbit point.
 *
 * @param marker - The satellite mesh to reposition
 * @param point  - The orbit state to move to (position in km)
 */
export function updateSatellitePosition(marker: THREE.Mesh, point: OrbitPoint): void {
  marker.position.set(
    point.x / EARTH_RADIUS_KM,
    point.y / EARTH_RADIUS_KM,
    point.z / EARTH_RADIUS_KM,
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
export function updateOrbitTrail(line: THREE.Line, visibleCount: number, totalCount: number): void {
  const clamped = Math.max(0, Math.min(visibleCount, totalCount));
  line.geometry.setDrawRange(0, clamped);
}

/**
 * Linearly interpolate between two OrbitPoints at the given fraction (0..1)
 * between them. Quaternion attitude is interpolated via slerp.
 */
export function lerpPoint(a: OrbitPoint, b: OrbitPoint, frac: number): OrbitPoint {
  const inv = 1 - frac;
  const result: OrbitPoint = {
    t: a.t * inv + b.t * frac,
    x: a.x * inv + b.x * frac,
    y: a.y * inv + b.y * frac,
    z: a.z * inv + b.z * frac,
    vx: a.vx * inv + b.vx * frac,
    vy: a.vy * inv + b.vy * frac,
    vz: a.vz * inv + b.vz * frac,
    a: a.a * inv + b.a * frac,
    e: a.e * inv + b.e * frac,
    inc: a.inc * inv + b.inc * frac,
    raan: a.raan * inv + b.raan * frac,
    omega: a.omega * inv + b.omega * frac,
    nu: a.nu * inv + b.nu * frac,
  };

  // Quaternion slerp for attitude interpolation
  if (a.qw != null && b.qw != null) {
    const qa = new THREE.Quaternion(a.qx, a.qy, a.qz, a.qw);
    const qb = new THREE.Quaternion(b.qx, b.qy, b.qz, b.qw);
    // Ensure shortest-path interpolation
    if (qa.dot(qb) < 0) {
      qb.set(-qb.x, -qb.y, -qb.z, -qb.w);
    }
    qa.slerp(qb, frac);
    result.qw = qa.w;
    result.qx = qa.x;
    result.qy = qa.y;
    result.qz = qa.z;
    // Angular velocity: linear interpolation
    result.wx = (a.wx ?? 0) * inv + (b.wx ?? 0) * frac;
    result.wy = (a.wy ?? 0) * inv + (b.wy ?? 0) * frac;
    result.wz = (a.wz ?? 0) * inv + (b.wz ?? 0) * frac;
  }

  return result;
}
