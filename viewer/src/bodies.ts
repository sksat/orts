/** Rendering properties for a known celestial body. */
export interface BodyRenderInfo {
  id: string;
  name: string;
  /** Path to texture image (relative to public/), or null for fallback color. */
  texturePath: string | null;
  /** Path to night-side texture (city lights), or null if not applicable. */
  nightTexturePath: string | null;
  /** Base name for multi-resolution textures (e.g., "earth" → earth_2k.jpg, earth_4k.jpg). */
  textureBaseName?: string;
  /** Base name for multi-resolution night textures. */
  nightTextureBaseName?: string;
  /** Fallback solid color (hex) when no texture is available/loaded. */
  fallbackColor: number;
  /** Emissive color for lit materials. */
  emissiveColor: number;
  /** Whether this body emits its own light (e.g., Sun). */
  isSelfLuminous: boolean;
}

const base = import.meta.env.BASE_URL;

const BODY_REGISTRY: Record<string, BodyRenderInfo> = {
  earth: {
    id: "earth",
    name: "Earth",
    texturePath: `${base}textures/earth_2k.jpg`,
    nightTexturePath: `${base}textures/earth_night_2k.jpg`,
    textureBaseName: "earth",
    nightTextureBaseName: "earth_night",
    fallbackColor: 0x2255aa,
    emissiveColor: 0x112244,
    isSelfLuminous: false,
  },
  moon: {
    id: "moon",
    name: "Moon",
    texturePath: `${base}textures/moon.jpg`,
    nightTexturePath: null,
    textureBaseName: "moon",
    fallbackColor: 0x888888,
    emissiveColor: 0x222222,
    isSelfLuminous: false,
  },
  sun: {
    id: "sun",
    name: "Sun",
    texturePath: `${base}textures/sun.jpg`,
    nightTexturePath: null,
    textureBaseName: "sun",
    fallbackColor: 0xffcc00,
    emissiveColor: 0xffaa00,
    isSelfLuminous: true,
  },
  mars: {
    id: "mars",
    name: "Mars",
    texturePath: `${base}textures/mars.jpg`,
    nightTexturePath: null,
    textureBaseName: "mars",
    fallbackColor: 0xcc6633,
    emissiveColor: 0x331100,
    isSelfLuminous: false,
  },
};

const UNKNOWN_BODY: BodyRenderInfo = {
  id: "unknown",
  name: "Unknown Body",
  texturePath: null,
  nightTexturePath: null,
  fallbackColor: 0x666666,
  emissiveColor: 0x222222,
  isSelfLuminous: false,
};

/** Look up rendering info for a body by its identifier. */
export function getBodyRenderInfo(bodyId: string): BodyRenderInfo {
  return BODY_REGISTRY[bodyId] ?? UNKNOWN_BODY;
}
