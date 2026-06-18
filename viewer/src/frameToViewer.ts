import type { ViewerReferenceFrame } from "./lib/index.js";
import type { ReferenceFrame } from "./referenceFrame.js";

/**
 * Map the renderer's internal {@link ReferenceFrame} — which the app's frame UI
 * (FrameSelector) uses as its state — onto the public {@link ViewerReferenceFrame}
 * that `<OrbitScene>` accepts.
 *
 * The app only ever centres on the central body or a satellite. Body centres
 * other than the central body (`moon`/`sun`) aren't expressible in the narrower
 * public frame and fall back to central-body inertial — unreachable in practice
 * since the app's FrameSelector never offers them.
 */
export function toViewerReferenceFrame(frame: ReferenceFrame): ViewerReferenceFrame {
  if (frame.center.type === "satellite") {
    return {
      center: { satelliteId: frame.center.id },
      orientation: frame.orientation === "local_orbital" ? "localOrbital" : "inertial",
    };
  }
  return {
    center: "centralBody",
    orientation: frame.orientation === "body_fixed" ? "bodyFixed" : "inertial",
  };
}
