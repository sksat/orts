export {
  type BodyDefinition,
  type BodyDefinitions,
  type BodyTexture,
  DEFAULT_BODIES,
} from "../bodies.js";
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
// arika WASM control (Sun direction / body rotation). OrbitViewer auto-inits when
// given an epoch; these let an embedder pre-load or point at an external .wasm.
export { type InitArikaOptions, initArika, isArikaReady } from "../wasm/arikaInit.js";
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
