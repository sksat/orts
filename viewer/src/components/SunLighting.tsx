import { useMemo } from "react";
import * as THREE from "three";
import { type DisplayFrame, type DisplayRotationFrame, displayDirection } from "../displayFrame.js";
import { inverseSquareIntensity } from "../sunLighting.js";
import {
  has_sun_ephemeris,
  sun_direction_from_body,
  sun_distance_from_body,
} from "../wasm/arikaInit.js";

// Default sun direction when no epoch is provided: ECI +X (vernal equinox).
const DEFAULT_SUN_DIRECTION_ECI: [number, number, number] = [1, 0, 0];

/** Directional light distance from the origin, as a multiple of sceneAmplification. */
const LIGHT_DISTANCE_FACTOR = 10;

/** Ambient light intensity. Shared with the lit bodies' ambient term. */
export const AMBIENT_INTENSITY = 0.15;

/** Directional intensity at 1 AU, before the inverse-square distance scale. */
const BASE_DIRECTIONAL_INTENSITY = 3.0;

export interface SunLightingParams {
  centralBody: string;
  epochJd?: number | null;
  /** Sim time, quantised (e.g. to 60s) to limit WASM recomputation. */
  quantizedSimTime: number;
  /**
   * The scene's resolved display frame. The same value the positions and
   * attitudes are drawn through, so the lit hemisphere cannot disagree with the
   * geometry it lights.
   */
  displayFrame: DisplayFrame;
  /** Environment scale-up factor for satellite-centred views. */
  sceneAmplification: number;
}

export interface SunLightingState {
  /** Sun direction in the active display frame. */
  sunDirection: THREE.Vector3;
  /**
   * Whether `sunDirection` came out of an ephemeris.
   *
   * False without an epoch, and false for a central body arika cannot place:
   * `sun_direction_from_body` answers +X — the vernal equinox — in that case, and
   * the value is indistinguishable from a computed one. A consumer that *draws*
   * the Sun has to check this; the light does not, since a fixed direction lights
   * a model without claiming anything.
   */
  sunDirectionIsComputed: boolean;
  /** Inverse-square intensity scale relative to 1 AU. */
  sunIntensity: number;
  /** Directional light position (sun direction scaled out by the light distance). */
  lightPosition: [number, number, number];
}

/**
 * Compute sun direction + intensity for the scene via the arika WASM model.
 *
 * `sunDirection`/`sunIntensity` are consumed both by {@link SunLighting} and by
 * the lit bodies (CelestialBody/SecondaryBody), so the scene holds them and
 * threads them down explicitly — there is no implicit lighting context (which
 * would also leak into the embeddable OrbitViewer's prop-driven scene graph).
 */
export function useSunLighting({
  centralBody,
  epochJd,
  quantizedSimTime,
  displayFrame,
  sceneAmplification,
}: SunLightingParams): SunLightingState {
  // Sun direction in the body-centred inertial frame (ECI), via WASM.
  const sunDirectionEci = useMemo<[number, number, number]>(() => {
    if (epochJd == null) return DEFAULT_SUN_DIRECTION_ECI;
    const dir = sun_direction_from_body(centralBody, epochJd, quantizedSimTime);
    return [dir[0], dir[1], dir[2]];
  }, [centralBody, epochJd, quantizedSimTime]);

  const sunDirectionIsComputed = useMemo(
    () => epochJd != null && has_sun_ephemeris(centralBody),
    [centralBody, epochJd],
  );

  // Sun intensity: inverse-square law based on the body-Sun distance.
  const sunIntensity = useMemo(() => {
    if (epochJd == null) return 1.0;
    return inverseSquareIntensity(sun_distance_from_body(centralBody, epochJd, quantizedSimTime));
  }, [centralBody, epochJd, quantizedSimTime]);

  // Sun direction rotated into the active display frame. Keyed on the frame's
  // rotation, not the frame itself: a satellite-centred view's origin moves with
  // every sample, and keying on that would rebuild the vector (and the light
  // position, and every lit body's uniform) each time without the direction
  // having changed.
  const era = displayFrame.kind === "bodyFixed" ? displayFrame.era : null;
  const lvlhAxes = displayFrame.kind === "localOrbital" ? displayFrame.axes : null;
  const sunDirection = useMemo(() => {
    const rotation: DisplayRotationFrame =
      era != null
        ? { kind: "bodyFixed", era }
        : lvlhAxes != null
          ? { kind: "localOrbital", axes: lvlhAxes }
          : { kind: "inertial" };
    return new THREE.Vector3(...displayDirection(rotation, sunDirectionEci));
  }, [sunDirectionEci, era, lvlhAxes]);

  const lightDistance = sceneAmplification * LIGHT_DISTANCE_FACTOR;
  const lightPosition = useMemo<[number, number, number]>(
    () => [
      sunDirection.x * lightDistance,
      sunDirection.y * lightDistance,
      sunDirection.z * lightDistance,
    ],
    [sunDirection, lightDistance],
  );

  return { sunDirection, sunDirectionIsComputed, sunIntensity, lightPosition };
}

/**
 * Scene lighting: a fixed ambient term plus a sun-tracking directional light.
 * Pair with {@link useSunLighting} for the position/intensity inputs.
 */
export function SunLighting({
  intensity,
  position,
}: {
  /** Inverse-square sun intensity scale (from {@link useSunLighting}). */
  intensity: number;
  position: [number, number, number];
}) {
  return (
    <>
      <ambientLight intensity={AMBIENT_INTENSITY} />
      <directionalLight intensity={BASE_DIRECTIONAL_INTENSITY * intensity} position={position} />
    </>
  );
}
