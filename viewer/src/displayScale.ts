/**
 * Display scale profiles for the 3D viewer.
 *
 * Controls how objects are sized and how the camera is configured
 * depending on which object is at the scene origin. For example,
 * body-centered view uses exaggerated satellite sizes for visibility,
 * while satellite-centered view uses true 1:1 physical proportions
 * with the camera close to the satellite.
 */

import type { FrameCenter } from "./referenceFrame.js";

/** Parameters controlling object sizes and camera for a given view mode. */
export interface DisplayScaleProfile {
  /** Descriptive name (for debugging / profile identification). */
  name: string;

  /**
   * Whether satellite models should render at true physical scale.
   * When false, uses the exaggerated `scale` from `SatelliteModelConfig`.
   */
  trueScale: boolean;

  /** Radius of the sphere fallback marker in scene units. */
  sphereFallbackRadius: number;

  /** Camera near plane in scene units. */
  cameraNear: number;
  /** Camera far plane in scene units. */
  cameraFar: number;

  /** OrbitControls minimum distance in scene units. */
  minDistance: number;
  /** OrbitControls maximum distance in scene units. */
  maxDistance: number;

  /** Default camera distance from origin when entering this profile. */
  defaultCameraDistance: number;
}

/** Default physical size for unknown satellites (10 m). */
const DEFAULT_SATELLITE_SIZE_KM = 0.010;

/** Body-centered profile: exaggerated satellite sizes for visibility. */
const BODY_CENTERED_PROFILE: DisplayScaleProfile = {
  name: "body-centered",
  trueScale: false,
  sphereFallbackRadius: 0.005,
  cameraNear: 0.01,
  cameraFar: 1000,
  minDistance: 1.5,
  maxDistance: 100,
  defaultCameraDistance: 5.4,
};

/**
 * Build a satellite-centered profile with true 1:1 physical scale.
 *
 * The sphere fallback radius is set to the default satellite size
 * converted to scene units (divided by central body radius).
 */
function buildSatelliteCenteredProfile(centralBodyRadius: number): DisplayScaleProfile {
  return {
    name: "satellite-centered",
    trueScale: true,
    sphereFallbackRadius: DEFAULT_SATELLITE_SIZE_KM / centralBodyRadius,
    cameraNear: 1e-6,
    cameraFar: 100,
    minDistance: 1e-5,
    maxDistance: 10,
    defaultCameraDistance: 0.01,
  };
}

/**
 * Get the display scale profile for a given frame center.
 *
 * @param center - Which object is at the scene origin
 * @param centralBodyRadius - Central body radius in km (for true-scale computation)
 */
export function getDisplayScaleProfile(
  center: FrameCenter,
  centralBodyRadius: number,
): DisplayScaleProfile {
  switch (center.type) {
    case "satellite":
      return buildSatelliteCenteredProfile(centralBodyRadius);
    case "central_body":
    case "moon":
    case "sun":
      return BODY_CENTERED_PROFILE;
  }
}
