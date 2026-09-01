import { OrbitControls, type OrbitControlsProps } from "@react-three/drei";
import { useCallback } from "react";
import type * as THREE from "three";
import type { DirectionVector } from "../directionVectors.js";
import type { Quat, Vec3 } from "../displayFrame.js";
import { getSatelliteModelConfig } from "../satelliteModels.js";
import { type MarkerShape, resolveMarkerShape } from "../satelliteShapes.js";
import {
  axisLengthForSpan,
  frameAxisLengthForSpan,
  NOMINAL_SPACECRAFT_SPAN,
  spanNormalizedModelScale,
} from "../spacecraftScale.js";
import { DirectionArrows } from "./DirectionArrows.js";
import { SpacecraftVisual } from "./SpacecraftVisual.js";

/** The spacecraft sits at the origin; that is what makes this the attitude view. */
const ORIGIN: Vec3 = [0, 0, 0];

/** Ambient term. Higher than the orbit scene's: there is no lit body to bounce off. */
const AMBIENT_INTENSITY = 0.35;

/** Directional intensity. Fixed — apparent brightness is not the subject here. */
const DIRECTIONAL_INTENSITY = 2.0;

/** Light distance from the origin, in spacecraft spans. */
const LIGHT_DISTANCE = 10;

/** Opacity of the reference-frame triad, which sits behind the body axes. */
const FRAME_AXES_OPACITY = 0.4;

/**
 * The reference frame's axes: the scene axes themselves, drawn at the origin.
 *
 * Dimmed and longer than the body axes so the two RGB triads read apart when the
 * spacecraft happens to be near-aligned with the frame.
 */
function ReferenceAxes({ length }: { length: number }) {
  const dim = useCallback((helper: THREE.AxesHelper | null) => {
    if (helper == null) return;
    const material = helper.material as THREE.Material;
    material.transparent = true;
    material.opacity = FRAME_AXES_OPACITY;
  }, []);
  return <axesHelper ref={dim} args={[length]} />;
}

export interface AttitudeSceneContentsProps {
  /** Body-to-display quaternion [w, x, y, z], already in the display frame. */
  quaternion?: Quat;
  satId?: string;
  satName?: string | null;
  color?: number;
  /** Per-spacecraft marker shape override. */
  markerShape?: MarkerShape | null;
  /** View-wide marker shape default. */
  defaultMarkerShape?: MarkerShape | null;
  /** Reference directions in display-frame axes. */
  vectors: readonly DirectionVector[];
  /**
   * Sun direction in display-frame axes, from the same hook that produces the
   * Sun arrow — so a 3D model is lit from where the arrow points. With no epoch
   * this is the hook's documented fixed direction, which lights a model without
   * claiming to be a measurement; the arrow is the thing that gets dropped.
   */
  sunDirection: Vec3;
  controls?: boolean | Partial<OrbitControlsProps>;
  /** Draw the reference-frame triad (default true). */
  axes?: boolean;
}

/**
 * The attitude scene graph: one spacecraft at the origin, its body axes, the
 * reference-frame axes, and the reference-direction arrows.
 *
 * Every length here is a ratio of the spacecraft's apparent size, which is
 * normalised to {@link NOMINAL_SPACECRAFT_SPAN}. The scene therefore has no
 * central-body radius and no physical scale — only the orientation is being
 * shown, and the camera can be placed once and left alone.
 */
export function AttitudeSceneContents({
  quaternion,
  satId,
  satName,
  color,
  markerShape,
  defaultMarkerShape,
  vectors,
  sunDirection,
  controls = true,
  axes = true,
}: AttitudeSceneContentsProps) {
  const span = NOMINAL_SPACECRAFT_SPAN;
  const modelConfig = satId ? getSatelliteModelConfig(satId, satName) : null;
  // TODO: a model registered without `nativeSpanUnits` cannot be normalised, so
  // it falls back to the registry's own scale — which is relative to a central
  // body radius this scene does not have, and will draw far too small. Every
  // registered model is measured today; requiring `nativeSpanUnits` at
  // registration would make that structural.
  const modelScale = modelConfig
    ? (spanNormalizedModelScale(modelConfig, span) ?? undefined)
    : undefined;

  const shape = resolveMarkerShape({
    override: markerShape,
    globalDefault: defaultMarkerShape,
    hasAttitude: quaternion != null,
  });

  return (
    <>
      <ambientLight intensity={AMBIENT_INTENSITY} />
      <directionalLight
        intensity={DIRECTIONAL_INTENSITY}
        position={[
          sunDirection[0] * span * LIGHT_DISTANCE,
          sunDirection[1] * span * LIGHT_DISTANCE,
          sunDirection[2] * span * LIGHT_DISTANCE,
        ]}
      />

      {axes && <ReferenceAxes length={frameAxisLengthForSpan(span)} />}

      <SpacecraftVisual
        position={ORIGIN}
        quaternion={quaternion}
        satId={satId}
        satName={satName}
        color={color}
        markerShape={shape}
        markerSize={span / 2}
        axisLength={axisLengthForSpan(span)}
        modelScale={modelScale}
      />

      <DirectionArrows position={ORIGIN} vectors={vectors} visualSpan={span} debugId={satId} />

      {controls !== false && (
        <OrbitControls
          target={ORIGIN}
          enablePan={false}
          {...(typeof controls === "object" ? controls : {})}
        />
      )}
    </>
  );
}
