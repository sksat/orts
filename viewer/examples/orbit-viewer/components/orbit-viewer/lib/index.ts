export {
  type BodyDefinition,
  type BodyDefinitions,
  type BodyTexture,
  DEFAULT_BODIES,
} from "../bodies.js";
// Which reference-direction arrows a scene draws (AttitudeScene.directionVectors).
export type { DirectionVectorOptions } from "../directionVectors.js";
// Streaming trail buffer (SatelliteState.trailBuffer): the built-in buffer, its
// read contract, the point type it holds, and adapters to fill one from TrailPoints.
export type { OrbitPoint } from "../orbit.js";
// Marker shape for a satellite (SatelliteState.markerShape / defaultMarkerShape).
export type { MarkerShape } from "../satelliteShapes.js";
// Camera up convention: initialise a bring-your-own `<Canvas>` camera with it
// (see OrbitScene); OrbitViewer applies it for you.
export { SCENE_UP } from "../sceneFrame.js";
export { TrailBuffer, type TrailBufferLike } from "../utils/TrailBuffer.js";
// arika WASM control (Sun direction / body rotation). OrbitViewer/OrbitScene
// auto-init when given an epoch; these let an embedder pre-load or point at an
// external .wasm.
export { type InitArikaOptions, initArika, isArikaReady } from "../wasm/arikaInit.js";
// Attitude view: one spacecraft's orientation, without a central body or trails.
export { AttitudeScene } from "./AttitudeScene.js";
export { AttitudeViewer } from "./AttitudeViewer.js";
export { toTrailBuffer, trailPointToOrbitPoint } from "./adapt.js";
// Entry components: headline (own Canvas) + mid-layer (bring your own Canvas).
export { OrbitScene } from "./OrbitScene.js";
export { OrbitViewer } from "./OrbitViewer.js";
export {
  type AttitudeBodyState,
  type AttitudeFrame,
  type AttitudeSceneDataProps,
  type AttitudeSceneProps,
  type AttitudeViewerProps,
  type CentralBody,
  type ControlsProp,
  DEFAULT_VIEWER_FRAME,
  type OrbitSceneDataProps,
  type OrbitSceneProps,
  type OrbitViewerProps,
  type Quat,
  type SatelliteState,
  type TrailPoint,
  type Vec3,
  type ViewerReferenceFrame,
} from "./types.js";
