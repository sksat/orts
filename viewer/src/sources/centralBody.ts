/**
 * The central body constants a source is read against.
 *
 * A recording written by `orts` carries `mu` and the body radius. Anything
 * missing falls back to Earth, which is what the rest of the viewer assumes
 * when a source declares nothing.
 *
 * One module because two readers have to agree: the WASM batch that derives the
 * orbital values, and the `SimInfo` / DuckDB schema that the charts are built
 * from. A `mu` that differs between them shows the same recording two ways.
 */

/** Earth's gravitational parameter [km³/s²]. */
export const DEFAULT_MU = 398600.4418;
/** Earth's equatorial radius [km]. */
export const DEFAULT_BODY_RADIUS = 6378.137;

/** Resolved central body constants. */
export interface CentralBody {
  mu: number;
  bodyRadius: number;
}

/**
 * Resolve `mu` and the body radius from what a source declared.
 *
 * A `mu` that is absent, zero, negative or non-finite cannot scale an orbit —
 * every element derived from it would come out non-finite — so it falls back to
 * Earth rather than propagating through the charts.
 */
export function resolveCentralBody(
  mu: number | null | undefined,
  bodyRadius: number | null | undefined,
): CentralBody {
  return {
    mu: mu != null && Number.isFinite(mu) && mu > 0 ? mu : DEFAULT_MU,
    bodyRadius:
      bodyRadius != null && Number.isFinite(bodyRadius) ? bodyRadius : DEFAULT_BODY_RADIUS,
  };
}
