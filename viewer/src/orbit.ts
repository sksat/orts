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
 * A point's quaternion at unit norm, or null when its components do not describe
 * a length that can be divided by (zero, or non-finite).
 *
 * Callers must have checked {@link hasQuaternion} first.
 */
function unitQuaternion(p: OrbitPoint): THREE.Quaternion | null {
  const [w, x, y, z] = [p.qw as number, p.qx as number, p.qy as number, p.qz as number];
  const n = Math.hypot(w, x, y, z);
  if (!(Number.isFinite(n) && n > 0)) return null;
  return new THREE.Quaternion(x / n, y / n, z / n, w / n);
}

/** Whether a point carries a complete quaternion (all of qw/qx/qy/qz). */
function hasQuaternion(p: OrbitPoint): boolean {
  return p.qw != null && p.qx != null && p.qy != null && p.qz != null;
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

  // Quaternion slerp for attitude interpolation. Require a *complete* quaternion
  // on both points — guarding on qw alone would let a partial one (missing
  // qx/qy/qz, which Three defaults to 0) build an un-normalized rotation.
  if (hasQuaternion(a) && hasQuaternion(b)) {
    // Slerp assumes unit quaternions, and a simulator's attitude drifts off unit
    // norm as it integrates. Equal norms cancel — scaling both endpoints by 2
    // reproduces the unit result to 1e-16 — but unequal ones bend the path: norms
    // of 1 and 2 put the halfway rotation 1.2e-1 off in each component, and a
    // drift of a thousandth 1.9e-4 off. Normalising afterwards cannot recover it,
    // because the error is in which rotation was chosen, not in its length.
    //
    // What cannot be normalised is passed through as it came, so the display
    // frame still sees an attitude to refuse rather than one this function
    // invented: `THREE.Quaternion.normalize` turns a zero quaternion into the
    // identity, which would be exactly that invention.
    const qa = unitQuaternion(a);
    const qb = unitQuaternion(b);
    if (qa != null && qb != null) {
      // Ensure shortest-path interpolation
      if (qa.dot(qb) < 0) {
        qb.set(-qb.x, -qb.y, -qb.z, -qb.w);
      }
      qa.slerp(qb, frac);
      result.qw = qa.w;
      result.qx = qa.x;
      result.qy = qa.y;
      result.qz = qa.z;
    } else {
      // An endpoint the display frame refuses cannot be interpolated through.
      // Slerp from a zero quaternion returns a multiple of the *other* endpoint —
      // measured at a quarter of the way along, the result normalises to that
      // endpoint's rotation exactly — so the refused sample would be presented as
      // a measurement taken next door. The refusal is carried instead, and the
      // exact endpoints keep their own values, which is what a reader at a
      // sample's own timestamp should see.
      const source = frac <= 0 ? a : frac >= 1 ? b : qa == null ? a : b;
      result.qw = source.qw;
      result.qx = source.qx;
      result.qy = source.qy;
      result.qz = source.qz;
    }
    // Angular velocity: linear interpolation
    result.wx = (a.wx ?? 0) * inv + (b.wx ?? 0) * frac;
    result.wy = (a.wy ?? 0) * inv + (b.wy ?? 0) * frac;
    result.wz = (a.wz ?? 0) * inv + (b.wz ?? 0) * frac;
  }

  return result;
}
