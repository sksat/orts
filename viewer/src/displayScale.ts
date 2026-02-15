/**
 * Display scale profiles for the 3D viewer.
 *
 * Controls how objects are sized and how the camera is configured
 * depending on which object is at the scene origin.
 *
 * Body-centered view uses exaggerated satellite sizes for visibility.
 * Satellite-centered view keeps the exaggerated satellite model at origin
 * and amplifies the surrounding scene (Earth, trails) so that angular
 * sizes and positions are physically accurate from the satellite's viewpoint.
 */

import type { FrameCenter } from "./referenceFrame.js";
import { computeTrueModelScale, type SatelliteModelConfig } from "./satelliteModels.js";

/** Parameters controlling object sizes and camera for a given view mode. */
export interface DisplayScaleProfile {
  /** Descriptive name (for debugging / profile identification). */
  name: string;

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
  sphereFallbackRadius: 0.005,
  cameraNear: 0.01,
  cameraFar: 1000,
  minDistance: 1.5,
  maxDistance: 100,
  defaultCameraDistance: 5.4,
};

/**
 * Satellite-centered profile: camera close to satellite, scene amplified
 * for physically accurate angular proportions.
 */
const SATELLITE_CENTERED_PROFILE: DisplayScaleProfile = {
  name: "satellite-centered",
  sphereFallbackRadius: 0.005,
  cameraNear: 1e-4,
  cameraFar: 10000,
  minDistance: 0.005,
  maxDistance: 2000,
  defaultCameraDistance: 0.15,
};

/**
 * Get the display scale profile for a given frame center.
 *
 * @param center - Which object is at the scene origin
 */
export function getDisplayScaleProfile(
  center: FrameCenter,
): DisplayScaleProfile {
  switch (center.type) {
    case "satellite":
      return SATELLITE_CENTERED_PROFILE;
    case "central_body":
    case "moon":
    case "sun":
      return BODY_CENTERED_PROFILE;
  }
}

/**
 * Compute the scene amplification factor for satellite-centered mode.
 *
 * When a satellite is centered, its 3D model renders at exaggerated scale
 * for visibility. To show the surrounding environment (Earth, orbit trails)
 * at physically correct angular proportions relative to the satellite, all
 * environment geometry is amplified by this factor.
 *
 * The factor is the ratio of the satellite's exaggerated display size to
 * its true physical size in scene units (normalised by central body radius).
 *
 * @param satModelConfig - Model config of the centered satellite, or null for sphere fallback
 * @param centralBodyRadius - Central body radius in km
 */
export function computeSceneAmplification(
  satModelConfig: SatelliteModelConfig | null,
  centralBodyRadius: number,
): number {
  if (satModelConfig) {
    const trueScale = computeTrueModelScale(satModelConfig, centralBodyRadius);
    if (trueScale != null) {
      return satModelConfig.scale / trueScale;
    }
  }
  // Sphere fallback: ratio of exaggerated sphere to true physical radius
  const trueRadius = DEFAULT_SATELLITE_SIZE_KM / centralBodyRadius;
  return BODY_CENTERED_PROFILE.sphereFallbackRadius / trueRadius;
}
