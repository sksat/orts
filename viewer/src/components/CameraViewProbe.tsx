import { useThree } from "@react-three/fiber";
import { useEffect } from "react";
import { registerCameraView } from "../debug/cameraView.js";

/**
 * Publishes the scene's camera for the dev/E2E camera hook. Renders nothing, and
 * registers nothing outside dev builds.
 */
export function CameraViewProbe() {
  const camera = useThree((s) => s.camera);
  useEffect(() => registerCameraView(() => camera), [camera]);
  return null;
}
