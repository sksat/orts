import { useThree } from "@react-three/fiber";
import { useEffect, useRef } from "react";
import { PerspectiveCamera } from "three";
import {
  drawnExtentForSpan,
  initialCameraDistance,
  NOMINAL_SPACECRAFT_SPAN,
} from "../spacecraftScale.js";

/**
 * Choose the camera's distance once the canvas has a size, pulling it back if the
 * viewport is narrower than the default framing assumed.
 *
 * The `camera` prop is read at mount, before the canvas has a size, so a portrait
 * embedding would otherwise clip the axes sideways. Applied on the first sizing
 * only: reframing on every resize would undo a zoom the viewer had chosen, and
 * this is a starting view, not a constraint.
 *
 * `reframe` says whether that distance is ours to choose at all. The depth range
 * is not this component's: a caller's position of `[200, 0, 0]` with the default
 * `far` of 100, and a viewer dollying out past it, are the same problem — the
 * scene at the origin falls behind the far plane and the canvas goes blank — and
 * `FarPlaneBeyondScene`, in the scene itself, holds that invariant for as long as
 * the scene is mounted.
 */
export function InitialCameraFit({ fov, reframe }: { fov: number; reframe: boolean }) {
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
    // The distance the fit asks for goes through the same check a caller's
    // position does, in the same precision: the view matrix carries it to WebGL
    // as float32, so past 3.4e38 the camera is somewhere the renderer cannot
    // place it and the canvas is blank. A narrow effective field of view asks for
    // exactly that — a `zoom` of 1e38 leaves 5.3e-37° and fits from 6.5e38 spans
    // away. The camera keeps the framing it was built with instead, which is
    // drawable at any zoom.
    const placeable = Number.isFinite(Math.fround(needed));
    if (reframe && placeable && current > 0 && needed > current) {
      camera.position.multiplyScalar(needed / current);
    }
    // The far plane is not settled here. A narrow field of view fits from far
    // away — 1° needs some 345 spans — and so does a viewer dollying out, so the
    // scene keeps the plane beyond itself for as long as it is mounted rather
    // than once at this moment. See `FarPlaneBeyondScene`.
  }, [camera, size, fov, reframe]);
  return null;
}
