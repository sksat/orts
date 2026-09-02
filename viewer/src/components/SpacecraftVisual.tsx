import { Suspense } from "react";
import type { Quat } from "../displayFrame.js";
import { getSatelliteModelConfig } from "../satelliteModels.js";
import { type MarkerShape, resolveMarkerShape } from "../satelliteShapes.js";
import { BodyAxes } from "./BodyAxes.js";
import { PrimitiveMarker } from "./PrimitiveMarker.js";
import { SatelliteModel } from "./SatelliteModel.js";

/** Default radius of the sphere fallback marker in scene units. */
export const DEFAULT_SPHERE_RADIUS = 0.005;

interface SphereMarkerProps {
  position: [number, number, number];
  color: number;
  radius?: number;
}

export function SphereMarker({
  position,
  color,
  radius = DEFAULT_SPHERE_RADIUS,
}: SphereMarkerProps) {
  return (
    <mesh position={position}>
      <sphereGeometry args={[radius, 16, 16]} />
      <meshBasicMaterial color={color} />
    </mesh>
  );
}

interface SpacecraftVisualProps {
  /** Position in scene units, already expressed in the display frame. */
  position: [number, number, number];
  /**
   * Body-to-display quaternion [w, x, y, z], already expressed in the display
   * frame. Absent means no attitude is known: the body axes are not drawn and the
   * automatic marker shape falls back to a sphere.
   */
  quaternion?: Quat;
  /** Satellite identifier for the 3D model lookup and the E2E attitude hook. */
  satId?: string;
  /** Satellite display name, used as the model-lookup fallback. */
  satName?: string | null;
  /** Marker colour (default: 0xff4444). */
  color?: number;
  /**
   * Resolved marker shape for spacecraft without a 3D model. When omitted, falls
   * back to automatic (orientation-revealing cube when an attitude is present,
   * else a sphere). A GLTF model, when available, always takes precedence.
   */
  markerShape?: MarkerShape;
  /** Sphere radius / cube half-extent in scene units. */
  markerSize?: number;
  /** Body-axis length in scene units. */
  axisLength?: number;
  /** Model scale, overriding the registry's own (scene units per model unit). */
  modelScale?: number;
}

/**
 * One spacecraft as it is drawn: a 3D model when the registry knows this
 * satellite, else an orientation-revealing marker, plus its body axes.
 *
 * Takes only display-frame values — a scene position and a body-to-display
 * quaternion — so it carries no frame semantics of its own and both the orbit
 * and the attitude view can draw a spacecraft the same way. Deciding *where* a
 * spacecraft is and *which* frame it is drawn in belongs to the caller.
 *
 * The size defaults are the orbit view's: sizes are relative to the model's
 * configured scale, which is itself relative to the central body's radius. A view
 * with a different spatial scale (the attitude view normalises the spacecraft's
 * apparent size) passes its own `markerSize` / `axisLength` / `modelScale`.
 */
export function SpacecraftVisual({
  position,
  quaternion,
  satId,
  satName,
  color = 0xff4444,
  markerShape,
  markerSize,
  axisLength,
  modelScale,
}: SpacecraftVisualProps) {
  const modelConfig = satId ? getSatelliteModelConfig(satId, satName) : null;
  const resolvedAxisLength =
    axisLength ?? (modelConfig ? modelConfig.scale * 5 : DEFAULT_SPHERE_RADIUS * 6);

  const bodyAxes = quaternion ? (
    <BodyAxes
      position={position}
      quaternion={quaternion}
      axisLength={resolvedAxisLength}
      debugId={satId}
    />
  ) : null;

  if (modelConfig) {
    return (
      <>
        <Suspense fallback={<SphereMarker position={position} color={color} radius={markerSize} />}>
          <SatelliteModel
            position={position}
            config={modelConfig}
            scale={modelScale}
            quaternion={quaternion}
          />
        </Suspense>
        {bodyAxes}
      </>
    );
  }

  // Pick the marker shape: caller-resolved override/default, else automatic
  // (orientation-revealing cube when attitude is present — a sphere looks identical
  // at every orientation — sphere otherwise).
  const shape = resolveMarkerShape({ override: markerShape, hasAttitude: quaternion != null });
  const fallbackMarker =
    shape === "sphere" ? (
      <SphereMarker position={position} color={color} radius={markerSize} />
    ) : (
      <PrimitiveMarker position={position} quaternion={quaternion} size={markerSize} />
    );
  return (
    <>
      {fallbackMarker}
      {bodyAxes}
    </>
  );
}
