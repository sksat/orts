import { Canvas } from "@react-three/fiber";
import * as THREE from "three";
import { DEFAULT_CAMERA_POSITION, SCENE_UP } from "../sceneFrame.js";
import { OrbitSceneContents, type OrbitSceneContentsProps } from "./OrbitSceneContents.js";

// Set the scene up vector before any Three.js objects are created so that the
// camera, OrbitControls, and all scene objects use the correct convention.
THREE.Object3D.DEFAULT_UP.set(...SCENE_UP);

/**
 * Main Three.js scene: the @react-three/fiber Canvas wrapper around the shared
 * {@link OrbitSceneContents} scene graph (camera, controls, lights, central
 * body, trails, satellites). The embeddable OrbitViewer renders the same
 * contents in its own Canvas, so the scene logic stays in one place.
 */
export function Scene(props: OrbitSceneContentsProps) {
  return (
    <Canvas
      camera={{ position: DEFAULT_CAMERA_POSITION, fov: 60, near: 0.01, far: 1000 }}
      gl={{ logarithmicDepthBuffer: true }}
      style={{ position: "absolute", top: 0, left: 0, width: "100%", height: "100%" }}
    >
      <OrbitSceneContents {...props} />
    </Canvas>
  );
}
