/**
 * The physical constants of the bodies a source may name.
 *
 * A recording says which body it is around, and may or may not carry that
 * body's `mu` and radius. A default belongs to a body we know: this catalog is
 * what "know" means, so a name in it can have a missing field filled in, and a
 * name outside it cannot.
 *
 * Apart from {@link BodyDefinitions}, which says how a body is *drawn*
 * (textures, colours) and covers only the bodies the scene renders. These are
 * the numbers the orbit is measured against, and they cover every body `arika`
 * propagates around. A consumer can extend either.
 *
 * The values are `arika`'s, from `KnownBody::properties` in `arika/src/body.rs`.
 * TODO: generate this from there, the way the WS protocol types are generated
 * from their Rust definitions, so the two cannot drift.
 */

/** A body's physical constants. Both are optional: a catalog entry may know
 * only one of them, and a field it does not know cannot be filled in. */
export interface BodyConstants {
  /** Standard gravitational parameter μ = GM \[km³/s²\]. */
  mu?: number;
  /** Mean equatorial radius \[km\]. */
  radiusKm?: number;
}

/** A map of body id → its physical constants. */
export type BodyCatalog = Record<string, BodyConstants>;

/**
 * The bodies `arika` knows, keyed as `orts` writes them: lowercase.
 *
 * Earth is also what a source that names no body at all is read as, so its
 * entry carries the values the viewer has always assumed.
 */
export const DEFAULT_BODY_CATALOG: BodyCatalog = {
  sun: { mu: 132712440018.0, radiusKm: 695700.0 },
  mercury: { mu: 22031.868551, radiusKm: 2439.7 },
  venus: { mu: 324858.592, radiusKm: 6051.8 },
  earth: { mu: 398600.4418, radiusKm: 6378.137 },
  moon: { mu: 4902.800066, radiusKm: 1737.4 },
  mars: { mu: 42828.375214, radiusKm: 3396.2 },
  jupiter: { mu: 126686534.9218, radiusKm: 71492.0 },
  saturn: { mu: 37931206.159, radiusKm: 60268.0 },
  uranus: { mu: 5793951.256, radiusKm: 25559.0 },
  neptune: { mu: 6835099.9754, radiusKm: 24764.0 },
};

/** The body a source that names none is read as. */
export const IMPLICIT_BODY_ID = "earth";

/** Merge consumer-supplied constants over the defaults, per body id. */
export function resolveBodyCatalog(custom?: BodyCatalog): BodyCatalog {
  return custom ? { ...DEFAULT_BODY_CATALOG, ...custom } : DEFAULT_BODY_CATALOG;
}
