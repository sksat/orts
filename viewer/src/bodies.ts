/** Rendering properties for a known celestial body. */
export interface BodyRenderInfo {
  id: string;
  name: string;
  /** Path to texture image (relative to public/), or null for fallback color. */
  texturePath: string | null;
  /** Fallback solid color (hex) when no texture is available/loaded. */
  fallbackColor: number;
  /** Emissive color for lit materials. */
  emissiveColor: number;
  /** Whether this body emits its own light (e.g., Sun). */
  isSelfLuminous: boolean;
}

const BODY_REGISTRY: Record<string, BodyRenderInfo> = {
  earth: {
    id: "earth",
    name: "Earth",
    texturePath: "/textures/earth.jpg",
    fallbackColor: 0x2255aa,
    emissiveColor: 0x112244,
    isSelfLuminous: false,
  },
  moon: {
    id: "moon",
    name: "Moon",
    texturePath: "/textures/moon.jpg",
    fallbackColor: 0x888888,
    emissiveColor: 0x222222,
    isSelfLuminous: false,
  },
  sun: {
    id: "sun",
    name: "Sun",
    texturePath: "/textures/sun.jpg",
    fallbackColor: 0xffcc00,
    emissiveColor: 0xffaa00,
    isSelfLuminous: true,
  },
  mars: {
    id: "mars",
    name: "Mars",
    texturePath: "/textures/mars.jpg",
    fallbackColor: 0xcc6633,
    emissiveColor: 0x331100,
    isSelfLuminous: false,
  },
};

const UNKNOWN_BODY: BodyRenderInfo = {
  id: "unknown",
  name: "Unknown Body",
  texturePath: null,
  fallbackColor: 0x666666,
  emissiveColor: 0x222222,
  isSelfLuminous: false,
};

/** Look up rendering info for a body by its identifier. */
export function getBodyRenderInfo(bodyId: string): BodyRenderInfo {
  return BODY_REGISTRY[bodyId] ?? UNKNOWN_BODY;
}
