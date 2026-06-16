import { Canvas } from "@react-three/fiber";
import { useEffect, useMemo, useState } from "react";
import { getBodyRadius, resolveBodyDefinitions } from "../bodies.js";
import { OrbitSceneContents } from "../components/OrbitSceneContents.js";
import type { OrbitPoint } from "../orbit.js";
import { DEFAULT_CAMERA_POSITION, SCENE_UP } from "../sceneFrame.js";
import { initArika, isArikaReady } from "../wasm/arikaInit.js";
import { toOrbitPoint } from "./adapt.js";
import { resolveFrameContext } from "./frameContext.js";
import { DEFAULT_VIEWER_FRAME, type OrbitViewerProps } from "./types.js";
import { useTrailBuffers } from "./useTrailBuffers.js";

/**
 * Drive the bundled arika WASM to readiness when `enabled`. Returns `true` once
 * loaded so the scene can switch from default lighting to ephemeris-accurate
 * Sun/rotation. When `enabled` is false (no epoch), the WASM is never loaded —
 * epoch-less embedders pay no network/init cost and get the documented fixed Sun.
 */
function useArikaReady(enabled: boolean): boolean {
  const [ready, setReady] = useState(() => isArikaReady());
  useEffect(() => {
    if (!enabled || ready) return;
    let cancelled = false;
    initArika()
      .then(() => {
        if (!cancelled) setReady(true);
      })
      .catch(() => {
        // Leave `ready` false: the viewer keeps rendering with default lighting.
      });
    return () => {
      cancelled = true;
    };
  }, [enabled, ready]);
  return ready;
}

/**
 * Embeddable orbit viewer.
 *
 * Renders a central body at the origin and a set of satellites around it, with
 * an orbit-controls camera. Choose the display frame via `referenceFrame`
 * (central-body inertial/body-fixed, or satellite-centred). Supply `epochJd`
 * (and advance `time`) for physically-correct Sun lighting and body rotation
 * via the bundled arika WASM; otherwise a fixed Sun is used and the body is static.
 *
 * It composes the same {@link OrbitSceneContents} scene graph the app uses, so
 * frame handling, lighting and the LVLH camera tracking are shared, not duplicated.
 *
 * @example
 * ```tsx
 * <OrbitViewer
 *   centralBody={{ id: "earth", radiusKm: 6378.137 }}
 *   satellites={[{ id: "sat-1", position: [7000, 0, 1500] }]}
 * />
 * ```
 */
export function OrbitViewer({
  centralBody,
  bodies,
  satellites,
  referenceFrame = DEFAULT_VIEWER_FRAME,
  epochJd,
  time = 0,
  textureBaseUrl,
  className,
  style,
}: OrbitViewerProps) {
  // Only load the arika WASM when an epoch is supplied (Sun/rotation features).
  const arikaReady = useArikaReady(epochJd != null);

  // Body definitions: consumer-supplied bodies merged over the built-in defaults.
  const bodyDefinitions = useMemo(() => resolveBodyDefinitions(bodies), [bodies]);
  // Central-body radius: explicit prop wins; otherwise from the body definition.
  // Fail loudly rather than guess a scale — a wrong radius silently breaks the
  // entire scene's sizing.
  const centralBodyRadius = centralBody.radiusKm ?? getBodyRadius(centralBody.id, bodyDefinitions);
  if (centralBodyRadius == null) {
    throw new Error(
      `OrbitViewer: central body "${centralBody.id}" has no radius. Pass ` +
        "centralBody.radiusKm, or add the body to `bodies` with a radiusKm.",
    );
  }

  // Current position per satellite. `time` is stamped onto each point; the scene
  // reads it back as the simulation time that drives Sun direction and rotation.
  const satellitePositions = useMemo(() => {
    const map = new Map<string, OrbitPoint>();
    for (const sat of satellites) map.set(sat.id, toOrbitPoint(sat, time));
    return map;
  }, [satellites, time]);

  const satelliteNames = useMemo(() => {
    const map = new Map<string, string | null>();
    for (const sat of satellites) map.set(sat.id, sat.name ?? null);
    return map;
  }, [satellites]);

  // Per-satellite colour overrides from the public SatelliteState.color.
  const satelliteColors = useMemo(() => {
    const map = new Map<string, number | undefined>();
    for (const sat of satellites) map.set(sat.id, sat.color);
    return map;
  }, [satellites]);

  // Persistent per-satellite trail buffers (stable identity → incremental upload).
  const trailBuffers = useTrailBuffers(satellites);

  // Map the public frame onto the renderer's internal ReferenceFrame (incl.
  // local_orbital); the scene resolves the geometry itself via the shared
  // resolveSceneFrame kernel from the satellite positions it receives.
  const internalFrame = useMemo(
    () => resolveFrameContext(referenceFrame, () => undefined).referenceFrame,
    [referenceFrame],
  );

  // Only hand the scene an epoch once arika is loaded; otherwise its Sun/rotation
  // calls would run before the WASM is ready. Default lighting is used until then.
  const effectiveEpochJd = arikaReady ? (epochJd ?? null) : null;

  // Dev/E2E-only: expose per-satellite trail buffer state so E2E can prove that
  // advancing `time` (or appending points) does not rebuild the trail — a stable
  // `generation` means no full GPU re-upload. See tests/orbit-viewer-lib.spec.ts.
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    const w = window as unknown as Record<string, unknown>;
    w.__debug_orbit_viewer = {
      trail: (id: string) => {
        const b = trailBuffers.get(id);
        return b ? { length: b.length, generation: b.generation } : null;
      },
    };
    return () => {
      delete w.__debug_orbit_viewer;
    };
  }, [trailBuffers]);

  return (
    <div className={className} style={{ width: "100%", height: "100%", ...style }}>
      {/* Set the camera up via the prop rather than mutating the global
          THREE.Object3D.DEFAULT_UP (as the app's Scene does): a library shouldn't
          change a global that affects the embedder's own Three.js objects.
          CameraLvlhTracker keeps camera.up correct each frame. */}
      <Canvas
        camera={{ position: DEFAULT_CAMERA_POSITION, up: SCENE_UP, fov: 60, near: 0.01, far: 1000 }}
        gl={{ logarithmicDepthBuffer: true }}
      >
        <OrbitSceneContents
          trailBuffers={trailBuffers}
          satellitePositions={satellitePositions}
          satelliteNames={satelliteNames}
          satelliteColors={satelliteColors}
          centralBody={centralBody.id}
          centralBodyRadius={centralBodyRadius}
          bodyDefinitions={bodyDefinitions}
          epochJd={effectiveEpochJd}
          referenceFrame={internalFrame}
          textureBaseUrl={textureBaseUrl}
        />
      </Canvas>
    </div>
  );
}
