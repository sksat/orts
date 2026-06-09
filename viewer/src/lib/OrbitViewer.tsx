import { OrbitControls } from "@react-three/drei";
import { Canvas } from "@react-three/fiber";
import { useEffect, useMemo, useState } from "react";
import * as THREE from "three";
import { CelestialBody } from "../components/CelestialBody.js";
import { OrbitTrail } from "../components/OrbitTrail.js";
import { Satellite } from "../components/Satellite.js";
import { transformToLvlh } from "../coordTransform.js";
import { rotateZ } from "../frameTransform.js";
import { DEFAULT_CAMERA_POSITION, SCENE_UP } from "../sceneFrame.js";
import {
  earth_rotation_angle,
  initArika,
  isArikaReady,
  sun_direction_from_body,
  sun_distance_from_body,
} from "../wasm/arikaInit.js";
import { toOrbitPoint } from "./adapt.js";
import { resolveFrameContext } from "./frameContext.js";
import { DEFAULT_VIEWER_FRAME, type OrbitViewerProps } from "./types.js";
import { useTrailBuffers } from "./useTrailBuffers.js";

/** Default colour palette cycled across satellites without an explicit colour. */
const SATELLITE_COLORS = [0x00ff88, 0xff4488, 0x44aaff, 0xffaa44, 0xaa44ff];

const AU_KM = 149_597_870.7;
const DEFAULT_SUN_DIRECTION: [number, number, number] = [1, 0, 0];

function dot(axis: [number, number, number], v: THREE.Vector3): number {
  return axis[0] * v.x + axis[1] * v.y + axis[2] * v.z;
}

/**
 * Drive the bundled arika WASM to readiness. Returns `true` once loaded so
 * components can switch from default lighting to ephemeris-accurate values.
 */
function useArikaReady(): boolean {
  const [ready, setReady] = useState(() => isArikaReady());
  useEffect(() => {
    if (ready) return;
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
  }, [ready]);
  return ready;
}

/**
 * Embeddable orbit viewer.
 *
 * Renders a central body at the origin and a set of satellites around it, with
 * an orbit-controls camera. Choose the display frame via `referenceFrame`
 * (central-body inertial/body-fixed, or satellite-centred inertial/local-orbital).
 * Supply `epochJd` (and optionally `time`) for physically-correct Sun lighting
 * and body rotation via the bundled arika WASM; otherwise a fixed Sun is used.
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
  satellites,
  referenceFrame = DEFAULT_VIEWER_FRAME,
  epochJd,
  time = 0,
  sunDirection: sunDirectionOverride,
  textureBaseUrl,
  className,
  style,
}: OrbitViewerProps) {
  const arikaReady = useArikaReady();
  const useEphemeris = epochJd != null && arikaReady;

  // Quantize Sun time to whole minutes: the Sun barely moves second-to-second,
  // so this avoids recomputing the ephemeris on every position update.
  const sunTime = Math.floor(time / 60) * 60;

  // Resolve the public frame into the wiring the primitives consume.
  const frame = useMemo(
    () =>
      resolveFrameContext(referenceFrame, (id) => {
        const sat = satellites.find((s) => s.id === id);
        return sat ? { position: sat.position, velocity: sat.velocity } : undefined;
      }),
    [referenceFrame, satellites],
  );

  // Persistent per-satellite trail buffers (stable identity → incremental GPU upload).
  const trailBuffers = useTrailBuffers(satellites);

  // Earth-rotation angle: drives both the body spin and the body-fixed Sun rotation.
  const era = useMemo(
    () => (useEphemeris ? earth_rotation_angle(epochJd, time) : undefined),
    [useEphemeris, epochJd, time],
  );

  // Sun direction in the inertial frame.
  const sunDirectionEci = useMemo(() => {
    if (sunDirectionOverride) return new THREE.Vector3(...sunDirectionOverride).normalize();
    if (useEphemeris) {
      const d = sun_direction_from_body(centralBody.id, epochJd, sunTime);
      return new THREE.Vector3(d[0], d[1], d[2]);
    }
    return new THREE.Vector3(...DEFAULT_SUN_DIRECTION);
  }, [sunDirectionOverride, useEphemeris, centralBody.id, epochJd, sunTime]);

  // Sun direction rotated into the active display frame so lighting stays correct.
  const sunDirection = useMemo(() => {
    const lvlh = frame.lvlhAxes;
    if (lvlh) {
      return new THREE.Vector3(
        dot(lvlh.inTrack, sunDirectionEci),
        dot(lvlh.crossTrack, sunDirectionEci),
        dot(lvlh.radial, sunDirectionEci),
      );
    }
    if (frame.bodyFixed && era != null) {
      const [x, y, z] = rotateZ(sunDirectionEci.x, sunDirectionEci.y, sunDirectionEci.z, -era);
      return new THREE.Vector3(x, y, z);
    }
    return sunDirectionEci;
  }, [sunDirectionEci, frame.lvlhAxes, frame.bodyFixed, era]);

  const sunIntensity = useMemo(() => {
    if (!useEphemeris) return 1;
    const distKm = sun_distance_from_body(centralBody.id, epochJd, sunTime);
    return (AU_KM / distKm) ** 2;
  }, [useEphemeris, centralBody.id, epochJd, sunTime]);

  const lightPosition = useMemo<[number, number, number]>(
    () => [sunDirection.x * 10, sunDirection.y * 10, sunDirection.z * 10],
    [sunDirection],
  );

  // Central body placement: at the origin when central-body-centred, otherwise
  // offset (LVLH-transformed, or plain inertial offset) relative to the centre.
  const bodyPosition = useMemo<[number, number, number] | null>(() => {
    const origin = frame.originPosition;
    if (origin == null) return null;
    if (frame.lvlhAxes) {
      return transformToLvlh(0, 0, 0, origin, frame.lvlhAxes, centralBody.radiusKm);
    }
    return [
      -origin[0] / centralBody.radiusKm,
      -origin[1] / centralBody.radiusKm,
      -origin[2] / centralBody.radiusKm,
    ];
  }, [frame.originPosition, frame.lvlhAxes, centralBody.radiusKm]);

  // In a body-fixed view the body is static; otherwise it spins by the ERA.
  const bodyRotation = frame.bodyFixed ? 0 : era;

  // Camera up: radial for an inertial satellite-centred view, scene-Z otherwise
  // (LVLH already maps radial onto +Z). Set on the camera, not the global default.
  const cameraUp = useMemo<[number, number, number]>(() => {
    const origin = frame.originPosition;
    if (origin == null || frame.lvlhAxes) return SCENE_UP;
    const len = Math.hypot(origin[0], origin[1], origin[2]);
    return len > 1e-10 ? [origin[0] / len, origin[1] / len, origin[2] / len] : SCENE_UP;
  }, [frame.originPosition, frame.lvlhAxes]);

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
      <Canvas
        camera={{ position: DEFAULT_CAMERA_POSITION, up: cameraUp, fov: 60, near: 0.01, far: 1000 }}
        gl={{ logarithmicDepthBuffer: true }}
      >
        <OrbitControls enableDamping dampingFactor={0.1} />

        <ambientLight intensity={0.15} />
        <directionalLight intensity={3.0 * sunIntensity} position={lightPosition} />

        <CelestialBody
          bodyId={centralBody.id}
          radius={1}
          sunDirection={sunDirection}
          rotationAngle={bodyRotation}
          lvlhPosition={bodyPosition}
          ambientIntensity={0.15}
          sunIntensity={sunIntensity}
          textureBaseUrl={textureBaseUrl}
        />

        {satellites.map((sat, i) => {
          const color = sat.color ?? SATELLITE_COLORS[i % SATELLITE_COLORS.length];
          const buffer = trailBuffers.get(sat.id);
          const isCenter =
            typeof frame.referenceFrame.center === "object" &&
            frame.referenceFrame.center.type === "satellite" &&
            frame.referenceFrame.center.id === sat.id;
          return (
            <group key={sat.id}>
              {buffer && buffer.length > 0 && (
                <OrbitTrail
                  trailBuffer={buffer}
                  scaleRadius={centralBody.radiusKm}
                  color={color}
                  referenceFrame={frame.referenceFrame}
                  epochJd={epochJd}
                  originPosition={frame.originPosition}
                  lvlhAxes={frame.lvlhAxes}
                />
              )}
              <Satellite
                position={toOrbitPoint(sat, time)}
                scaleRadius={centralBody.radiusKm}
                color={color}
                referenceFrame={frame.referenceFrame}
                epochJd={epochJd ?? undefined}
                satId={sat.id}
                satName={sat.name}
                originPosition={frame.originPosition}
                lvlhAxes={frame.lvlhAxes}
                hideSphereFallback={isCenter}
              />
            </group>
          );
        })}

        <axesHelper args={[2]} />
      </Canvas>
    </div>
  );
}
