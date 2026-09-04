import { Canvas, useThree } from "@react-three/fiber";
import { useEffect, useRef } from "react";
import { PerspectiveCamera } from "three";
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
 * Pull the camera back once, if the viewport turns out to be narrower than the
 * default framing assumed.
 *
 * The `camera` prop is read at mount, before the canvas has a size, so a portrait
 * embedding would otherwise clip the axes sideways. Applied on the first sizing
 * only: reframing on every resize would undo a zoom the viewer had chosen, and
 * this is a starting view, not a constraint.
 */
function InitialCameraFit({ fov }: { fov: number }) {
  const camera = useThree((s) => s.camera);
  const size = useThree((s) => s.size);
  const applied = useRef(false);
  useEffect(() => {
    if (applied.current || size.width === 0 || size.height === 0) return;
    applied.current = true;
    // A `zoom` narrows the field of view, so fit the *effective* one — a camera
    // framed for 50° and zoomed 2× sees half as much and would clip.
    const effectiveFov = camera instanceof PerspectiveCamera ? camera.getEffectiveFOV() : fov;
    const needed = cameraDistanceForSpan(
      NOMINAL_SPACECRAFT_SPAN,
      effectiveFov,
      size.width / size.height,
    );
    const current = camera.position.length();
    if (current > 0 && needed > current) camera.position.multiplyScalar(needed / current);

    // A narrow field of view fits from far away — 1° needs some 345 spans — and
    // the default far plane would then cut the scene off in front of the
    // spacecraft: a blank canvas at a correct distance.
    const reach = camera.position.length() + drawnExtentForSpan(NOMINAL_SPACECRAFT_SPAN);
    if (camera instanceof PerspectiveCamera && camera.far < reach) {
      camera.far = reach;
      camera.updateProjectionMatrix();
    }
  }, [camera, size, fov]);
  return null;
}

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
/** The camera settings an embedder may pass through `canvas`. */
type CameraProps = NonNullable<AttitudeViewerProps["canvas"]>["camera"];

/** Near plane the default framing uses, in spacecraft spans. */
const DEFAULT_NEAR = 0.01;

/** Far plane; `InitialCameraFit` pushes it out when it moves the camera. */
const DEFAULT_FAR = 100;

/**
 * The caller's projection settings, or the defaults where they name no frustum.
 *
 * Every one of these reaches `PerspectiveCamera` directly and degenerates its
 * projection matrix on its own: a `zoom` of 0 or NaN, a `near` at or below zero,
 * a `far` inside the near plane. The result is a blank canvas that no distance
 * fitted for it can repair, so they are checked here rather than trusted.
 */
function usableProjection(camera: CameraProps) {
  const positive = (value: number | undefined, fallback: number) =>
    value != null && Number.isFinite(value) && value > 0 ? value : fallback;
  const near = positive(camera?.near, DEFAULT_NEAR);
  const far = positive(camera?.far, DEFAULT_FAR);
  return {
    fov: usableFovDegrees(camera?.fov),
    zoom: positive(camera?.zoom, 1),
    near,
    // A far plane inside the near one has no volume between them to draw.
    far: far > near ? far : Math.max(DEFAULT_FAR, near * 2),
  };
}

export function AttitudeViewer({ className, style, canvas, ...sceneProps }: AttitudeViewerProps) {
  const projection = usableProjection(canvas?.camera);
  return (
    <div className={className} style={{ width: "100%", height: "100%", ...style }}>
      <Canvas
        camera={{
          position: DEFAULT_CAMERA_POSITION,
          up: SCENE_UP,
          ...canvas?.camera,
          ...projection,
        }}
        gl={{ ...canvas?.gl }}
      >
        {canvas?.camera?.position == null && <InitialCameraFit fov={projection.fov} />}
        <AttitudeScene {...sceneProps} />
      </Canvas>
    </div>
  );
}
