/** Rendering configuration for a satellite with a known 3D model. */
export interface SatelliteModelConfig {
  /** URL to the GLB model file. */
  modelUrl: string;
  /** Scale factor applied to the loaded model for body-centered (exaggerated) display. */
  scale: number;
  /** Euler rotation [rx, ry, rz] to orient the model in ECI frame. */
  rotation: [number, number, number];
  /** Physical span of the real satellite in km (e.g. ISS = 0.109 km). */
  physicalSpanKm?: number;
  /**
   * Native span of the 3D model in model-local units (before any scale).
   * Used to compute true-scale: trueScale = (physicalSpanKm / bodyRadius) / nativeSpanUnits.
   */
  nativeSpanUnits?: number;
}

const MODEL_REGISTRY: Record<string, SatelliteModelConfig> = {
  iss: {
    modelUrl:
      "https://assets.science.nasa.gov/content/dam/science/psd/solar/2023/09/i/ISS_stationary.glb",
    scale: 0.0003,
    rotation: [0, 0, 0],
    physicalSpanKm: 0.109, // ~109 m
    nativeSpanUnits: 111.99,
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

/** Default physical span for satellites without a known size (10 m). */
const DEFAULT_PHYSICAL_SPAN_KM = 0.01;

/**
 * Compute the scene-unit scale for a satellite model at true 1:1 physical proportions.
 *
 * @param config - Model config (with optional physicalSpanKm / nativeSpanUnits)
 * @param centralBodyRadius - Central body radius in km
 * @returns Scale to apply to the model for true-size rendering, or null if
 *          nativeSpanUnits is unknown (fall back to exaggerated scale).
 */
export function computeTrueModelScale(
  config: SatelliteModelConfig,
  centralBodyRadius: number,
): number | null {
  const nativeSpan = config.nativeSpanUnits;
  if (nativeSpan == null || nativeSpan <= 0) return null;

  const physicalKm = config.physicalSpanKm ?? DEFAULT_PHYSICAL_SPAN_KM;
  // physicalKm / centralBodyRadius = desired span in scene units
  // Divide by nativeSpan to get per-unit scale
  return physicalKm / centralBodyRadius / nativeSpan;
}
