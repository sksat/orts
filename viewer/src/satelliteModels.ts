/** Rendering configuration for a satellite with a known 3D model. */
export interface SatelliteModelConfig {
  /** URL to the GLB model file. */
  modelUrl: string;
  /** Scale factor applied to the loaded model (in scene units). */
  scale: number;
  /** Euler rotation [rx, ry, rz] to orient the model in ECI frame. */
  rotation: [number, number, number];
}

const MODEL_REGISTRY: Record<string, SatelliteModelConfig> = {
  iss: {
    modelUrl:
      "https://assets.science.nasa.gov/content/dam/science/psd/solar/2023/09/i/ISS_stationary.glb",
    scale: 0.0003,
    rotation: [0, 0, 0],
  },
};

/** Name patterns that map to a registry key. */
const NAME_PATTERNS: ReadonlyArray<{ pattern: RegExp; key: string }> = [
  { pattern: /ISS/i, key: "iss" },
];

/**
 * Look up model config for a satellite.
 * Checks id first, then name patterns. Returns null for sphere fallback.
 */
export function getSatelliteModelConfig(
  satId: string,
  satName?: string | null,
): SatelliteModelConfig | null {
  if (MODEL_REGISTRY[satId]) return MODEL_REGISTRY[satId];
  if (satName) {
    for (const { pattern, key } of NAME_PATTERNS) {
      if (pattern.test(satName)) return MODEL_REGISTRY[key] ?? null;
    }
  }
  return null;
}
