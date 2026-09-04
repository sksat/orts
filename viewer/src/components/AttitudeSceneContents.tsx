import { OrbitControls, type OrbitControlsProps } from "@react-three/drei";
import type { DirectionVector } from "../directionVectors.js";
import type { Quat, Vec3 } from "../displayFrame.js";
import { getSatelliteModelConfig } from "../satelliteModels.js";
import { type MarkerShape, resolveMarkerShape } from "../satelliteShapes.js";
import {
  axisLengthForSpan,
  frameAxisArrows,
  frameAxisLengthForSpan,
  markerBoundingRadius,
  modelBoundingRadius,
  NOMINAL_SPACECRAFT_SPAN,
  spanNormalizedModelScale,
} from "../spacecraftScale.js";
import { AxisTriad } from "./AxisTriad.js";
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
 * Longer, fainter and slenderer than the body axes, so the two RGB triads read
 * apart by form and not only by colour when the spacecraft happens to be
 * near-aligned with the frame. Both carry X / Y / Z, since neither triad's
 * identity can be recovered from its colours alone.
 */
function ReferenceAxes({ length, span }: { length: number; span: number }) {
  return <AxisTriad geometry={frameAxisArrows(length, span)} opacity={FRAME_AXES_OPACITY} labels />;
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

  // Whether the model is the thing on screen. Without a usable attitude a model
  // would be drawn at its own default orientation, which reads as a measured one
  // in a view about orientation, so a marker stands in — and one value decides
  // both what is drawn and what the arrows have to start outside of.
  const modelIsDrawn = modelConfig != null && quaternion != null;

  // Without a usable attitude, nothing drawn may imply one. An explicit shape —
  // or the app's default, which the `satShape` URL param persists — outranks
  // `hasAttitude` in `resolveMarkerShape`, so the orientation-revealing cube
  // would be drawn at identity and read as a measured attitude. The sphere is the
  // one marker that looks the same from every side.
  const shape: MarkerShape =
    quaternion == null
      ? "sphere"
      : resolveMarkerShape({
          override: markerShape,
          globalDefault: defaultMarkerShape,
          hasAttitude: true,
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

      {axes && <ReferenceAxes length={frameAxisLengthForSpan(span)} span={span} />}

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
        visualSpan={span}
        model={modelIsDrawn}
      />

      <DirectionArrows
        position={ORIGIN}
        vectors={vectors}
        visualSpan={span}
        // Outside whatever is actually drawn: a cube marker's corners stand
        // further out than its faces, and a model is bounded only by the envelope
        // of the cube its largest extent fits in. Keyed on the model being drawn
        // rather than on the registry knowing one, or a registered spacecraft
        // with an unusable attitude would draw a sphere and start its arrows at
        // the model's envelope, well outside it.
        startRadius={modelIsDrawn ? modelBoundingRadius(span) : markerBoundingRadius(shape, span)}
        debugId={satId}
      />

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
