/**
 * Public, embeddable API of the orts viewer.
 *
 * The headline export is {@link OrbitViewer}: give it a central body and a list
 * of satellites and you get an interactive 3D scene you can orbit the camera
 * around. The lower-level Three.js / react-three-fiber building blocks and the
 * pure frame/trail adapters are also exported for assembling custom scenes.
 *
 * Surface note: the primitives below wrap internal Three.js/r3f components, so
 * they widen the semver surface (their internals changing is a breaking change).
 * That's an accepted trade-off for the "minimal component + primitives" goal —
 * consumers who only need the component should import just OrbitViewer + types.
 * See the package README (../../README.md).
 */

// Central-body definitions: built-in bodies + the type for adding custom ones.
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
// Marker shape for a satellite (SatelliteState.markerShape / defaultMarkerShape).
export type { MarkerShape } from "../satelliteShapes.js";
// `SCENE_UP`: camera up convention — initialise a bring-your-own `<Canvas>`
// camera with it (see OrbitScene); OrbitViewer applies it for you.
export { computeLvlhAxes, type LvlhAxes, SCENE_UP } from "../sceneFrame.js";
// Trail buffer: the built-in streaming buffer (SatelliteState.trailBuffer) plus
// the read interface the scene accepts for a bring-your-own buffer.
export { TrailBuffer, type TrailBufferLike } from "../utils/TrailBuffer.js";
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
// Headline component (own Canvas) + the mid-layer scene (bring your own Canvas).
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
