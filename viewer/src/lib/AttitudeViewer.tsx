import { Canvas, useThree } from "@react-three/fiber";
import { useEffect, useRef } from "react";
import { PerspectiveCamera } from "three";
import { usableProjection, usableVector } from "../cameraProps.js";
import { CameraViewProbe } from "../components/CameraViewProbe.js";
import { SCENE_UP } from "../sceneFrame.js";
import {
  cameraDistanceForSpan,
  DEFAULT_CAMERA_FOV_DEGREES,
  drawnExtentForSpan,
  initialCameraDistance,
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
    // A `near` from the camera prop can also sit past the scene, which frames it
    // correctly and draws none of it, so the distance clears that plane too.
    const needed = initialCameraDistance(
      NOMINAL_SPACECRAFT_SPAN,
      effectiveFov,
      size.width / size.height,
      camera instanceof PerspectiveCamera ? camera.near : 0,
    );
    const current = camera.position.length();
    // Applying the fit has to leave a position Three.js can still measure. It
    // sums the squared components, so a distance past about 1.3e154 overflows the
    // length — and the far plane derived from it just below would become
    // infinite. A `zoom` of 1e200 asks for exactly that: an effective field of
    // view of 5.3e-199° fits from 6.5e200 spans away. The camera keeps the
    // framing it was built with instead, which is drawable at any zoom.
    const placeable = Number.isFinite(needed * needed);
    if (placeable && current > 0 && needed > current) {
      camera.position.multiplyScalar(needed / current);
    }

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
export function AttitudeViewer({ className, style, canvas, ...sceneProps }: AttitudeViewerProps) {
  // The default near plane is the scale the fit assumes; a caller's is checked
  // against what this view draws around the origin.
  const projection = usableProjection(
    canvas?.camera,
    usableFovDegrees(canvas?.camera?.fov),
    drawnExtentForSpan(NOMINAL_SPACECRAFT_SPAN),
  );
  const position = usableVector(
    canvas?.camera?.position as readonly number[] | undefined,
    DEFAULT_CAMERA_POSITION,
  );
  // Whether the framing is still ours to adjust. `usableVector` hands back the
  // fallback itself when the caller's position is no direction, so a position
  // that was supplied but unusable belongs to us as much as an absent one — and
  // checking the raw prop instead would leave the square default fit standing on
  // a portrait canvas.
  const framingIsOurs = position === DEFAULT_CAMERA_POSITION;
  return (
    <div className={className} style={{ width: "100%", height: "100%", ...style }}>
      <Canvas
        camera={{
          ...canvas?.camera,
          position,
          up: usableVector(canvas?.camera?.up as readonly number[] | undefined, SCENE_UP),
          ...projection,
        }}
        gl={{ ...canvas?.gl }}
      >
        <CameraViewProbe />
        {framingIsOurs && <InitialCameraFit fov={projection.fov} />}
        <AttitudeScene {...sceneProps} />
      </Canvas>
    </div>
  );
}
