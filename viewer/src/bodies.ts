/**
 * Central / secondary body definitions for the 3D scene.
 *
 * Bodies are *data*, not hardcoded: {@link DEFAULT_BODIES} ships Earth / Moon /
 * Sun / Mars, and a consumer can add or override any body by passing its own
 * definitions (merged over the defaults via {@link resolveBodyDefinitions}).
 * The lookups below are pure functions of a definitions map, so a scene only
 * ever sees the bodies it was given — no module-global registry.
 *
 * Note: physically-correct Sun direction and body rotation come from the arika
 * WASM model, which only knows a fixed set of bodies. A *custom* body renders
 * (radius, texture, colour) but does not get physical lighting/rotation.
 */

import { BASE_URL } from "./env.js";

/** Texture sources for a body. Provide direct URLs/paths via `day`/`night`. */
export interface BodyTexture {
  /** Day-side texture URL/path, or null/omitted for a flat fallback colour. */
  day?: string | null;
  /**
   * Night-side (city lights) texture URL/path. Currently only the built-in
   * Earth renders a day/night terminator; for other bodies this is ignored
   * (they render with `day` as a flat-lit sphere).
   */
  night?: string | null;
  /**
   * Base name for an orts server's multi-resolution upgrade (e.g. "earth" →
   * earth_2k/4k/8k). Only used when a `textureBaseUrl` server is connected;
   * standalone consumers can ignore this and just set `day`/`night`.
   */
  baseName?: string;
  nightBaseName?: string;
}

/**
 * How a body should be rendered. The body's id is the key in a
 * {@link BodyDefinitions} map, so it isn't repeated here.
 */
export interface BodyDefinition {
  name?: string;
  /** Physical radius in km (sets the scene scale and secondary-body sizing). */
  radiusKm: number;
  texture?: BodyTexture;
  /** Solid colour shown when no texture is available/loaded (hex). */
  fallbackColor?: number;
  /** Emissive colour for lit materials (hex). */
  emissiveColor?: number;
  /** Whether the body emits its own light (e.g. the Sun). */
  selfLuminous?: boolean;
}

/** A map of body id → definition. */
export type BodyDefinitions = Record<string, BodyDefinition>;

const base = BASE_URL;

/** Built-in bodies. Consumers merge their own over these. */
export const DEFAULT_BODIES: BodyDefinitions = {
  earth: {
    name: "Earth",
    radiusKm: 6378.137,
    texture: {
      day: `${base}textures/earth_2k.jpg`,
      night: `${base}textures/earth_night_2k.jpg`,
      baseName: "earth",
      nightBaseName: "earth_night",
    },
    fallbackColor: 0x2255aa,
    emissiveColor: 0x112244,
    selfLuminous: false,
  },
  moon: {
    name: "Moon",
    radiusKm: 1737.4,
    texture: { day: `${base}textures/moon.jpg`, baseName: "moon" },
    fallbackColor: 0x888888,
    emissiveColor: 0x222222,
    selfLuminous: false,
  },
  sun: {
    name: "Sun",
    radiusKm: 695700,
    texture: { day: `${base}textures/sun.jpg`, baseName: "sun" },
    fallbackColor: 0xffcc00,
    emissiveColor: 0xffaa00,
    selfLuminous: true,
  },
  mars: {
    name: "Mars",
    radiusKm: 3389.5,
    texture: { day: `${base}textures/mars.jpg`, baseName: "mars" },
    fallbackColor: 0xcc6633,
    emissiveColor: 0x331100,
    selfLuminous: false,
  },
};

/** Render fallback for an id with no definition (flat grey sphere). */
const UNKNOWN_BODY: BodyDefinition = {
  name: "Unknown Body",
  radiusKm: 1,
  fallbackColor: 0x666666,
  emissiveColor: 0x222222,
  selfLuminous: false,
};

/** Merge consumer-supplied bodies over the defaults (shallow, per id). */
export function resolveBodyDefinitions(custom?: BodyDefinitions): BodyDefinitions {
  return custom ? { ...DEFAULT_BODIES, ...custom } : DEFAULT_BODIES;
}

/** Look up a body's render definition, falling back to a flat grey sphere. */
export function getBodyRenderInfo(bodyId: string, bodies: BodyDefinitions): BodyDefinition {
  return bodies[bodyId] ?? UNKNOWN_BODY;
}

/** Get a body's radius in km, or null if it's not one of the scene's bodies. */
export function getBodyRadius(bodyId: string, bodies: BodyDefinitions): number | null {
  return bodies[bodyId]?.radiusKm ?? null;
}

/**
 * Extract a body id from an orts server entity path, or null if it's a
 * satellite / not one of the scene's bodies.
 *
 * Convention (orts-server specific — kept internal, not part of the public API):
 * - `/world/sat/*` → satellite (null)
 * - `/world/<bodyId>` where `<bodyId>` is in `bodies` → that body
 */
export function entityPathToBodyId(entityPath: string, bodies: BodyDefinitions): string | null {
  if (entityPath.startsWith("/world/sat/")) return null;
  const segments = entityPath.split("/").filter(Boolean);
  if (segments.length >= 2 && segments[0] === "world") {
    const candidate = segments[segments.length - 1];
    if (candidate in bodies) return candidate;
  }
  return null;
}
