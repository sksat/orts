/**
 * Lazy WASM loader for tobari Earth environment models.
 *
 * Follows the same pattern as viewer/src/wasm/arikaInit.ts.
 */

// biome-ignore lint: dynamic import types
let wasmModule: any = undefined;
let initialized = false;
let initPromise: Promise<void> | undefined;

/** Initialize the tobari WASM module. Safe to call multiple times. */
export function initTobari(): Promise<void> {
  if (initialized) return Promise.resolve();
  if (initPromise) return initPromise;

  const p: Promise<void> = import("./tobari/tobari.js").then(async (mod) => {
    await mod.default();
    wasmModule = mod;
    initialized = true;
  });
  initPromise = p;
  return p;
}

export function isTobariReady(): boolean {
  return initialized;
}

// ---------------------------------------------------------------------------
// Atmosphere
// ---------------------------------------------------------------------------

export function exponentialDensity(altitudeKm: number): number {
  return wasmModule!.exponential_density(altitudeKm);
}

export function harrisPriesterDensity(
  latDeg: number,
  lonDeg: number,
  altitudeKm: number,
  epochJd: number,
): number {
  return wasmModule!.harris_priester_density(latDeg, lonDeg, altitudeKm, epochJd);
}

export function nrlmsise00Density(
  latDeg: number,
  lonDeg: number,
  altitudeKm: number,
  epochJd: number,
  f107: number,
  ap: number,
): number {
  return wasmModule!.nrlmsise00_density(latDeg, lonDeg, altitudeKm, epochJd, f107, ap);
}

export function atmosphereAltitudeProfile(
  altitudes: Float64Array,
  latDeg: number,
  lonDeg: number,
  epochJd: number,
  f107: number,
  ap: number,
): Float64Array {
  return wasmModule!.atmosphere_altitude_profile(altitudes, latDeg, lonDeg, epochJd, f107, ap);
}

export function atmosphereLatlonMap(
  model: string,
  altitudeKm: number,
  epochJd: number,
  nLat: number,
  nLon: number,
  f107: number,
  ap: number,
): Float64Array {
  return wasmModule!.atmosphere_latlon_map(model, altitudeKm, epochJd, nLat, nLon, f107, ap);
}

// ---------------------------------------------------------------------------
// Magnetic field
// ---------------------------------------------------------------------------

export interface FieldInfo {
  north: number; // nT
  east: number; // nT
  down: number; // nT
  total: number; // nT
  inclination: number; // deg
  declination: number; // deg
}

export function igrfFieldAt(
  latDeg: number,
  lonDeg: number,
  altitudeKm: number,
  epochJd: number,
): FieldInfo {
  const arr: Float64Array = wasmModule!.igrf_field_at(latDeg, lonDeg, altitudeKm, epochJd);
  return {
    north: arr[0],
    east: arr[1],
    down: arr[2],
    total: arr[3],
    inclination: arr[4],
    declination: arr[5],
  };
}

export function dipoleFieldAt(
  latDeg: number,
  lonDeg: number,
  altitudeKm: number,
  epochJd: number,
): FieldInfo {
  const arr: Float64Array = wasmModule!.dipole_field_at(latDeg, lonDeg, altitudeKm, epochJd);
  return {
    north: arr[0],
    east: arr[1],
    down: arr[2],
    total: arr[3],
    inclination: arr[4],
    declination: arr[5],
  };
}

export function magneticFieldLatlonMap(
  model: string,
  component: string,
  altitudeKm: number,
  epochJd: number,
  nLat: number,
  nLon: number,
): Float64Array {
  return wasmModule!.magnetic_field_latlon_map(model, component, altitudeKm, epochJd, nLat, nLon);
}

// ---------------------------------------------------------------------------
// Volume data (3D)
// ---------------------------------------------------------------------------

export interface VolumeResult {
  /** Scalar values, alt-major: index = iAlt * nLat * nLon + iLat * nLon + iLon */
  data: Float32Array;
  min: number;
  max: number;
}

export function atmosphereVolume(
  model: string,
  altMinKm: number,
  altMaxKm: number,
  nAlt: number,
  epochJd: number,
  nLat: number,
  nLon: number,
  f107: number,
  ap: number,
): VolumeResult {
  const raw: Float32Array = wasmModule!.atmosphere_volume(
    model,
    altMinKm,
    altMaxKm,
    nAlt,
    epochJd,
    nLat,
    nLon,
    f107,
    ap,
  );
  const total = nAlt * nLat * nLon;
  return {
    data: raw.slice(0, total),
    min: raw[total],
    max: raw[total + 1],
  };
}

// ---------------------------------------------------------------------------
// Magnetic field lines
// ---------------------------------------------------------------------------

export interface FieldLine {
  /** Vertices in Earth radii, flat [x0,y0,z0, x1,y1,z1, ...] */
  vertices: Float32Array;
  nPoints: number;
}

export function magneticFieldLines(
  seedLats: Float64Array,
  seedLons: Float64Array,
  seedAltKm: number,
  epochJd: number,
  model: string,
  maxSteps: number,
  stepKm: number,
): FieldLine[] {
  const raw: Float32Array = wasmModule!.magnetic_field_lines(
    seedLats,
    seedLons,
    seedAltKm,
    epochJd,
    model,
    maxSteps,
    stepKm,
  );

  const nLines = raw[0];
  const lines: FieldLine[] = [];
  let offset = 1;

  for (let i = 0; i < nLines; i++) {
    const nPts = raw[offset];
    offset++;
    const verts = raw.slice(offset, offset + nPts * 3);
    offset += nPts * 3;
    lines.push({ vertices: verts, nPoints: nPts });
  }

  return lines;
}
