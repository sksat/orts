/**
 * Public, embeddable API of the orts viewer.
 *
 * The headline export is {@link OrbitViewer}: give it a central body and a list
 * of satellites and you get an interactive 3D scene you can orbit the camera
 * around. The lower-level Three.js / react-three-fiber building blocks and the
 * pure frame/trail adapters are also exported for assembling custom scenes.
 */

// Lower-level building blocks for assembling a custom scene.
export { BodyAxes } from "../components/BodyAxes.js";
export { CelestialBody } from "../components/CelestialBody.js";
export { EarthBody } from "../components/EarthBody.js";
export { OrbitTrail } from "../components/OrbitTrail.js";
export { Satellite } from "../components/Satellite.js";
export { SatelliteModel } from "../components/SatelliteModel.js";
// Domain types and helpers used across the building blocks.
export type { OrbitPoint } from "../orbit.js";
export { DEFAULT_FRAME, type ReferenceFrame } from "../referenceFrame.js";
export { computeLvlhAxes, type LvlhAxes } from "../sceneFrame.js";
export { TrailBuffer } from "../utils/TrailBuffer.js";
// Pure adapters and frame/trail logic (useful for custom scenes / advanced use).
export { toOrbitPoint, toTrailBuffer, trailPointToOrbitPoint } from "./adapt.js";
export {
  type FrameContext,
  type FrameSatellite,
  type FrameSatelliteLookup,
  resolveFrameContext,
} from "./frameContext.js";
// Headline component + its public types.
export { OrbitViewer } from "./OrbitViewer.js";
export {
  type CentralBody,
  DEFAULT_VIEWER_FRAME,
  type OrbitViewerProps,
  type Quat,
  type SatelliteState,
  type TrailPoint,
  type Vec3,
  type ViewerReferenceFrame,
} from "./types.js";
