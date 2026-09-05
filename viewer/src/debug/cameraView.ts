/**
 * Dev/E2E-only view of where the camera is and which way it looks.
 *
 * The camera is configured through props and then oriented by whatever mounts —
 * R3F's own `lookAt` at creation, `OrbitControls` when it is enabled, the initial
 * fit when it moves the camera. A test that wants to know the result has to read
 * the camera, not the props: `window.__debug_get_camera_view()` returns the
 * rendered position and forward direction in world coordinates.
 *
 * No-op outside dev builds, like the other debug registries.
 */
import type * as THREE from "three";
import { Vector3 } from "three";
import { IS_DEV } from "../env.js";

export interface CameraView {
  /** World position [scene units]. */
  position: [number, number, number];
  /** Unit vector the camera looks along. */
  forward: [number, number, number];
  near: number;
  far: number;
}

type CameraGetter = () => THREE.Camera | null;

interface DebugWindow extends Record<string, unknown> {
  __debug_camera_getter?: CameraGetter;
  __debug_get_camera_view?: () => CameraView | null;
}

/**
 * Publish a camera so its rendered placement is queryable. Returns a cleanup
 * function; a no-op (returning a no-op) outside dev builds.
 */
export function registerCameraView(getCamera: CameraGetter): () => void {
  if (!IS_DEV) return () => {};
  const w = window as unknown as DebugWindow;
  w.__debug_camera_getter = getCamera;
  w.__debug_get_camera_view = () => {
    const camera = w.__debug_camera_getter?.();
    if (!camera) return null;
    const position = camera.getWorldPosition(new Vector3());
    const forward = camera.getWorldDirection(new Vector3());
    const perspective = camera as THREE.PerspectiveCamera;
    return {
      position: [position.x, position.y, position.z],
      forward: [forward.x, forward.y, forward.z],
      near: perspective.near,
      far: perspective.far,
    };
  };
  return () => {
    if (w.__debug_camera_getter === getCamera) {
      w.__debug_camera_getter = undefined;
    }
  };
}
