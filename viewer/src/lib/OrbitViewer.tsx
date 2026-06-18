import { Canvas } from "@react-three/fiber";
import { DEFAULT_CAMERA_POSITION, SCENE_UP } from "../sceneFrame.js";
import { OrbitScene } from "./OrbitScene.js";
import type { OrbitViewerProps } from "./types.js";

/**
 * Embeddable orbit viewer.
 *
 * Renders a central body at the origin and a set of satellites around it, with
 * an orbit-controls camera. Choose the display frame via `referenceFrame`
 * (central-body inertial/body-fixed, or satellite-centred). Supply `epochJd`
 * (and advance `time`) for physically-correct Sun lighting and body rotation
 * via the bundled arika WASM; otherwise a fixed Sun is used and the body is static.
 *
 * This is the batteries-included wrapper: a sized `<div>` and a configured
 * `<Canvas>` around the shared {@link OrbitScene} graph. Drop {@link OrbitScene}
 * into your own Canvas instead when you need to compose it with your own lights,
 * meshes, post-processing or camera.
 *
 * @example
 * ```tsx
 * <OrbitViewer
 *   centralBody={{ id: "earth", radiusKm: 6378.137 }}
 *   satellites={[{ id: "sat-1", position: [7000, 0, 1500] }]}
 * />
 * ```
 */
export function OrbitViewer({ className, style, canvas, ...sceneProps }: OrbitViewerProps) {
  return (
    <div className={className} style={{ width: "100%", height: "100%", ...style }}>
      {/* Set the camera up via the prop rather than mutating the global
          THREE.Object3D.DEFAULT_UP: a library shouldn't change a global that
          affects the embedder's own Three.js objects. The camera rig keeps
          camera.up correct each frame. The `canvas` prop merges over these. */}
      <Canvas
        camera={{
          position: DEFAULT_CAMERA_POSITION,
          up: SCENE_UP,
          fov: 60,
          near: 0.01,
          far: 1000,
          ...canvas?.camera,
        }}
        gl={{ logarithmicDepthBuffer: true, ...canvas?.gl }}
      >
        <OrbitScene {...sceneProps} />
      </Canvas>
    </div>
  );
}
