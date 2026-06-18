/**
 * Public API of the orts viewer.
 *
 * Two entry components: {@link OrbitViewer} (batteries-included — its own sized
 * `<div>` + `<Canvas>`) and {@link OrbitScene} (the scene graph to mount inside
 * your own @react-three/fiber `<Canvas>`). Drive either with a central body and a
 * list of {@link SatelliteState}.
 *
 * The Three.js / react-three-fiber building blocks (CelestialBody, Satellite,
 * OrbitTrail, …) and the internal frame wiring are intentionally NOT exported:
 * they ride internal types and are an implementation detail. The streaming
 * {@link TrailBuffer} is the one renderer primitive that is public, since a
 * high-rate feed needs to hand the scene a buffer it mutates directly
 * (`SatelliteState.trailBuffer`); the {@link TrailPoint} adapters to fill one are
 * exported alongside it. See the package README (../../README.md).
 */

// Central-body definitions: built-in bodies + the type for adding custom ones.
export {
  type BodyDefinition,
  type BodyDefinitions,
  type BodyTexture,
  DEFAULT_BODIES,
} from "../bodies.js";
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
export { toTrailBuffer, trailPointToOrbitPoint } from "./adapt.js";
// Entry components: headline (own Canvas) + mid-layer (bring your own Canvas).
export { OrbitScene } from "./OrbitScene.js";
export { OrbitViewer } from "./OrbitViewer.js";
export {
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
