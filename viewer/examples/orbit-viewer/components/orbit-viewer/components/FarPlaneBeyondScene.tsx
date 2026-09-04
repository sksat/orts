import { useFrame, useThree } from "@react-three/fiber";
import { PerspectiveCamera } from "three";
import { drawnExtentForSpan } from "../spacecraftScale.js";

/**
 * Keep the far plane beyond the scene, wherever the camera ends up.
 *
 * The controls dolly without a limit, and the spacecraft sits at the origin, so
 * a viewer who zooms out past the far plane loses the whole scene at once — the
 * canvas goes blank rather than showing something small. The rule is the one the
 * opening framing uses, `far >= distance + R` for the sphere of radius
 * {@link drawnExtentForSpan}, applied continuously instead of once: it is two
 * numbers compared per frame, and it writes only when the camera has moved
 * beyond the plane.
 *
 * Mounted by whoever owns the camera's depth range, which is why it is not in
 * the scene: an embedder who configures their own `<Canvas>` keeps their far
 * plane, including a deliberately tight one, and the scene has no way to tell
 * that from a default it should be maintaining. {@link AttitudeViewer} mounts
 * this for the plane it chose itself.
 */
export function FarPlaneBeyondScene({ span }: { span: number }) {
  const camera = useThree((s) => s.camera);
  useFrame(() => {
    if (!(camera instanceof PerspectiveCamera)) return;
    const reach = camera.position.length() + drawnExtentForSpan(span);
    if (Number.isFinite(reach) && camera.far < reach) {
      camera.far = reach;
      camera.updateProjectionMatrix();
    }
  });
  return null;
}
