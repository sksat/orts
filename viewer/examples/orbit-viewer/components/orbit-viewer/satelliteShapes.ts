export const MARKER_SHAPES = ["sphere", "axes-cube"] as const;
export type MarkerShape = (typeof MARKER_SHAPES)[number];

/** Shape used automatically for satellites that carry attitude. */
export const DEFAULT_ATTITUDE_SHAPE: MarkerShape = "axes-cube";

/** Human-readable labels for UI selectors. */
export const MARKER_SHAPE_LABELS: Record<MarkerShape, string> = {
  sphere: "Sphere",
  "axes-cube": "XYZ Cube",
};

export function isMarkerShape(value: string): value is MarkerShape {
  return (MARKER_SHAPES as readonly string[]).includes(value);
}

/**
 * Resolve the marker shape for a satellite. Precedence (most specific first):
 *   0. `attitudeRefused` — the sphere, whatever was asked for
 *   1. `override`     — per-satellite viewer choice
 *   2. `simShape`     — shape declared by the simulation (via SatelliteInfo)
 *   3. `globalDefault`— viewer-wide default
 *   4. automatic      — orientation cube when attitude is present, else sphere
 * A null/undefined value at any level falls through to the next.
 *
 * `attitudeRefused` says the sample carried an attitude the viewer could not use.
 * It outranks the requests above because the cube's faces show which way the body
 * points: drawn at the identity it would answer a question the data could not,
 * and the sphere is the one marker that looks the same from every side. A
 * satellite with no attitude at all is not this case — there the requested shape
 * stands, as it always has.
 */
export function resolveMarkerShape(opts: {
  override?: MarkerShape | null;
  simShape?: MarkerShape | null;
  globalDefault?: MarkerShape | null;
  hasAttitude: boolean;
  attitudeRefused?: boolean;
}): MarkerShape {
  if (opts.attitudeRefused) return "sphere";
  if (opts.override) return opts.override;
  if (opts.simShape) return opts.simShape;
  if (opts.globalDefault) return opts.globalDefault;
  return opts.hasAttitude ? DEFAULT_ATTITUDE_SHAPE : "sphere";
}

/** Read the global default marker shape from the `satShape` URL param (null = auto). */
export function readSatShapeParam(): MarkerShape | null {
  const raw = new URLSearchParams(window.location.search).get("satShape");
  return raw && isMarkerShape(raw) ? raw : null;
}

/** Persist the global default marker shape into the `satShape` URL param. */
export function writeSatShapeParam(shape: MarkerShape | null): void {
  const params = new URLSearchParams(window.location.search);
  if (shape == null) params.delete("satShape");
  else params.set("satShape", shape);
  const qs = params.toString();
  history.replaceState(
    null,
    "",
    qs ? `${window.location.pathname}?${qs}` : window.location.pathname,
  );
}
