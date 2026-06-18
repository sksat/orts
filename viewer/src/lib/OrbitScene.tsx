import { useEffect, useMemo, useState } from "react";
import { getBodyRadius, resolveBodyDefinitions } from "../bodies.js";
import { OrbitSceneContents } from "../components/OrbitSceneContents.js";
import { IS_DEV } from "../env.js";
import type { OrbitPoint } from "../orbit.js";
import type { MarkerShape } from "../satelliteShapes.js";
import { initArika, isArikaReady } from "../wasm/arikaInit.js";
import { toOrbitPoint } from "./adapt.js";
import { resolveFrameContext } from "./frameContext.js";
import { DEFAULT_VIEWER_FRAME, type OrbitSceneProps } from "./types.js";
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
 * The orbit scene graph, for mounting inside your own @react-three/fiber `<Canvas>`.
 *
 * This is the framework boundary: it adapts the public {@link OrbitSceneProps}
 * (a central body + a list of satellites) onto the internal renderer, handling
 * frame resolution, trail buffers and arika WASM init, and renders the central
 * body, satellites and trails plus (by default) the orbit camera rig + controls.
 *
 * Unlike {@link OrbitViewer}, it does NOT create a Canvas or a wrapping element,
 * so you can compose it with your own lights, meshes, post-processing or camera.
 * Initialise your Canvas camera with `up = SCENE_UP`; with the default camera
 * package (`controls` not `false`) the rig keeps `camera.up` correct each frame.
 *
 * @example
 * ```tsx
 * <Canvas camera={{ up: SCENE_UP }}>
 *   <OrbitScene
 *     centralBody={{ id: "earth", radiusKm: 6378.137 }}
 *     satellites={[{ id: "sat-1", position: [7000, 0, 1500] }]}
 *   />
 * </Canvas>
 * ```
 */
export function OrbitScene({
  centralBody,
  bodies,
  satellites,
  referenceFrame = DEFAULT_VIEWER_FRAME,
  epochJd,
  time = 0,
  textureBaseUrl,
  textureVersion,
  defaultMarkerShape,
  atmosphereScale,
  controls = true,
  axes = true,
}: OrbitSceneProps) {
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
      `OrbitScene: central body "${centralBody.id}" has no radius. Pass ` +
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

  // Per-satellite marker shape overrides (the renderer falls back to
  // defaultMarkerShape, then to an automatic shape). null/omitted = no override.
  const satelliteShapes = useMemo(() => {
    const map = new Map<string, MarkerShape>();
    for (const sat of satellites) {
      if (sat.markerShape != null) map.set(sat.id, sat.markerShape);
    }
    return map;
  }, [satellites]);

  // Per-satellite trail clipping (playback scrub / time window). Absent entries
  // draw the whole trail.
  const trailVisibleCounts = useMemo(() => {
    const map = new Map<string, number>();
    for (const sat of satellites) {
      const vc = sat.trailDisplay?.visibleCount;
      if (vc != null) map.set(sat.id, vc);
    }
    return map;
  }, [satellites]);

  const trailDrawStarts = useMemo(() => {
    const map = new Map<string, number>();
    for (const sat of satellites) {
      const ds = sat.trailDisplay?.drawStart;
      if (ds != null) map.set(sat.id, ds);
    }
    return map;
  }, [satellites]);

  // "auto" (and undefined) defers to the renderer's per-view default.
  const physicalScale =
    atmosphereScale === "physical" ? true : atmosphereScale === "visual" ? false : undefined;

  // Persistent per-satellite trail buffers (stable identity → incremental upload).
  // Streaming-mode satellites pass their own buffer through unchanged.
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
    if (!IS_DEV) return;
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
    <OrbitSceneContents
      trailBuffers={trailBuffers}
      satellitePositions={satellitePositions}
      satelliteNames={satelliteNames}
      satelliteColors={satelliteColors}
      satelliteShapes={satelliteShapes}
      defaultMarkerShape={defaultMarkerShape}
      trailVisibleCounts={trailVisibleCounts}
      trailDrawStarts={trailDrawStarts}
      centralBody={centralBody.id}
      centralBodyRadius={centralBodyRadius}
      bodyDefinitions={bodyDefinitions}
      epochJd={effectiveEpochJd}
      referenceFrame={internalFrame}
      physicalScale={physicalScale}
      textureBaseUrl={textureBaseUrl}
      textureRevision={textureVersion}
      controls={controls}
      axes={axes}
    />
  );
}
