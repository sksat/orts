import { Canvas } from "@react-three/fiber";
import { usableDirection, usablePosition, usableProjection } from "../cameraProps.js";
import { CameraViewProbe } from "../components/CameraViewProbe.js";
import { FarPlaneBeyondScene } from "../components/FarPlaneBeyondScene.js";
import { InitialCameraFit } from "../components/InitialCameraFit.js";
import { SCENE_UP } from "../sceneFrame.js";
import {
  cameraDistanceForSpan,
  DEFAULT_CAMERA_FOV_DEGREES,
  drawnExtentForSpan,
  NOMINAL_SPACECRAFT_SPAN,
  usableFovDegrees,
} from "../spacecraftScale.js";
import { AttitudeScene } from "./AttitudeScene.js";
import type { AttitudeViewerProps } from "./types.js";

/** The module that derives the framing owns the value. */
const DEFAULT_FOV = DEFAULT_CAMERA_FOV_DEGREES;

/** Viewing direction: off to one side and above, so all three axes are distinct. */
const CAMERA_DIRECTION: [number, number, number] = [0.894, 0, 0.447];

/**
 * Default camera framing, derived from what the scene draws rather than picked by
 * eye: the reference axes and the arrow tips both reach twice the spacecraft's
 * apparent size, and a closer camera clips them. Assumes a square viewport; a
 * narrower one is handled by {@link InitialCameraFit} once the size is known.
 */
const DEFAULT_CAMERA_POSITION: [number, number, number] = (() => {
  const d = cameraDistanceForSpan(NOMINAL_SPACECRAFT_SPAN, DEFAULT_FOV);
  return [CAMERA_DIRECTION[0] * d, CAMERA_DIRECTION[1] * d, CAMERA_DIRECTION[2] * d];
})();

/**
 * Embeddable attitude viewer.
 *
 * Shows one spacecraft's orientation: the spacecraft at the origin with its body
 * axes, the reference frame's axes around it, and the reference directions (Sun,
 * nadir) as arrows. Choose the display frame via `orientation`. Supply `epochJd`
 * (and advance `time`) for the Sun direction and the body-fixed frame, both of
 * which come from the bundled arika WASM.
 *
 * This is the batteries-included wrapper: a sized `<div>` and a configured
 * `<Canvas>` around {@link AttitudeScene}. Drop {@link AttitudeScene} into your
 * own Canvas instead when you need your own camera, lights or extra meshes.
 *
 * For a spacecraft in its orbit, with the central body and its trail, use
 * {@link OrbitViewer}. To compare two spacecraft's attitudes, place two of these
 * side by side: this view puts its spacecraft at the origin, and two spacecraft
 * cannot both be there.
 *
 * @example
 * ```tsx
 * <AttitudeViewer
 *   centralBody={{ id: "earth" }}
 *   body={{ id: "sat-1", attitude: [1, 0, 0, 0] }}
 * />
 * ```
 */
export function AttitudeViewer({ className, style, canvas, ...sceneProps }: AttitudeViewerProps) {
  // The default near plane is the scale the fit assumes; a caller's is checked
  // against what this view draws around the origin.
  const projection = usableProjection(
    canvas?.camera,
    usableFovDegrees(canvas?.camera?.fov),
    drawnExtentForSpan(NOMINAL_SPACECRAFT_SPAN),
  );
  const position = usablePosition(
    canvas?.camera?.position as readonly number[] | undefined,
    DEFAULT_CAMERA_POSITION,
  );
  // Whether the camera's *distance* is still ours to choose. `usablePosition`
  // hands back the fallback itself when the caller's position is no place to put
  // a camera, so a
  // position that was supplied but unusable belongs to us as much as an absent
  // one — and checking the raw prop instead would leave the square default fit
  // standing on a portrait canvas. The far plane is settled either way, so the
  // helper always mounts.
  const framingIsOurs = position === DEFAULT_CAMERA_POSITION;
  return (
    <div className={className} style={{ width: "100%", height: "100%", ...style }}>
      <Canvas
        camera={{
          ...canvas?.camera,
          position,
          up: usableDirection(canvas?.camera?.up as readonly number[] | undefined, SCENE_UP),
          ...projection,
        }}
        gl={{ ...canvas?.gl }}
      >
        <CameraViewProbe />
        <InitialCameraFit fov={projection.fov} reframe={framingIsOurs} />
        {/* Only for a plane this component chose. A caller's `far` is theirs. */}
        {projection.farIsDefault && <FarPlaneBeyondScene span={NOMINAL_SPACECRAFT_SPAN} />}
        <AttitudeScene {...sceneProps} />
      </Canvas>
    </div>
  );
}
