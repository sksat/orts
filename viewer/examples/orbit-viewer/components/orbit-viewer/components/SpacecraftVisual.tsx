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
  /**
   * The spacecraft's apparent size in scene units. Passing it draws the body axes
   * as named arrows rather than one-pixel lines — see {@link BodyAxes}.
   */
  visualSpan?: number;
  /**
   * Whether a registered 3D model may stand in for the marker (default true).
   *
   * A model drawn without a quaternion sits at its own default orientation, which
   * in the orbit view is honest — the model is a position marker there, and a
   * satellite may arrive with no attitude at all. A view whose subject *is* the
   * orientation passes false when the attitude is unusable, so the reader gets
   * the marker that looks the same from every side instead of a model that
   * appears to point somewhere.
   */
  model?: boolean;
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
  visualSpan,
  model = true,
}: SpacecraftVisualProps) {
  const modelConfig = model && satId ? getSatelliteModelConfig(satId, satName) : null;
  // The default axis length follows the scale the model is *drawn* at, not the
  // registry's: overriding one without the other would silently change the ratio
  // between a spacecraft and its axes.
  const effectiveModelScale = modelScale ?? modelConfig?.scale;
  const resolvedAxisLength =
    axisLength ??
    (effectiveModelScale != null ? effectiveModelScale * 5 : DEFAULT_SPHERE_RADIUS * 6);

  const bodyAxes = quaternion ? (
    <BodyAxes
      position={position}
      quaternion={quaternion}
      axisLength={resolvedAxisLength}
      debugId={satId}
      visualSpan={visualSpan}
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
