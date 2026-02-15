import { useEffect, useMemo, useRef } from "react";
import { Canvas, useFrame, useThree } from "@react-three/fiber";
import { OrbitControls } from "@react-three/drei";
import * as THREE from "three";
import { CelestialBody } from "./CelestialBody.js";
import { OrbitTrail } from "./OrbitTrail.js";
import { Satellite } from "./Satellite.js";
import { OrbitPoint } from "../orbit.js";
import { TrailBuffer } from "../utils/TrailBuffer.js";
import type { SatelliteInfo } from "../hooks/useWebSocket.js";
import { DEFAULT_CAMERA_POSITION, SCENE_UP, computeCameraUp, computeLvlhAxes, type LvlhAxes } from "../sceneFrame.js";
import { rotateZ } from "../frameTransform.js";
import { type ReferenceFrame, isLegacyEcef, isDefaultEci, DEFAULT_FRAME } from "../referenceFrame.js";
import { earth_rotation_angle, sun_direction_eci } from "../wasm/kanameInit.js";
import { transformToLvlh } from "../coordTransform.js";
import { getDisplayScaleProfile, computeSceneAmplification, type DisplayScaleProfile } from "../displayScale.js";
import { getSatelliteModelConfig } from "../satelliteModels.js";

// Set scene up vector before any Three.js objects are created
// so that Camera, OrbitControls, and all scene objects use the correct convention.
THREE.Object3D.DEFAULT_UP.set(...SCENE_UP);

// Default sun direction when no epoch is provided: ECI +X (vernal equinox).
const DEFAULT_SUN_DIRECTION = new THREE.Vector3(1, 0, 0);

/** Color palette for multiple satellites. */
const SATELLITE_COLORS = [0x00ff88, 0xff4488, 0x44aaff, 0xffaa44, 0xaa44ff];

/**
 * Smoothing speed for exponential-decay tracking (both orientation and position).
 * Higher = faster response (less smooth), lower = smoother (more lag).
 * Uses frame-rate-independent exponential decay: alpha = 1 - e^(-speed * dt).
 * At 60fps (dt≈0.017s) with speed=6: alpha≈0.10 per frame.
 */
const SMOOTHING_SPEED = 6;

/**
 * Tracks the LVLH frame and co-rotates the camera so that user-set
 * orientation (e.g. "Earth below, velocity right") is maintained as
 * the satellite orbits.
 *
 * The raw LVLH quaternion is smoothed via slerp to avoid jitter from
 * discrete velocity updates and perturbation oscillations.
 *
 * Runs at useFrame priority -1 (before OrbitControls at priority 0).
 * Each frame:
 *   1. Compute target LVLH quaternion from position + velocity
 *   2. Slerp smoothed quaternion toward target (exponential decay)
 *   3. Compute delta from previous smoothed quaternion
 *   4. Apply delta to camera.position (rotate around origin)
 *   5. Set camera.up to smoothed radial direction
 *
 * OrbitControls re-derives its spherical state from camera.position
 * each frame, so user drags are always relative to the current LVLH frame.
 *
 * Falls back to radial-only tracking when velocity is unavailable.
 */
function CameraLvlhTracker({ originPosition, originVelocity, lvlhActive }: {
  originPosition: [number, number, number] | null;
  originVelocity: [number, number, number] | null;
  /** When true, LVLH rotation is handled by the coordinate data, not the camera. */
  lvlhActive: boolean;
}) {
  const { camera } = useThree();
  const prevQuatRef = useRef<THREE.Quaternion | null>(null);

  useFrame((_state, delta) => {
    // Non-satellite-centered or LVLH body-frame mode: Z=radial is natural up
    if (originPosition == null || lvlhActive) {
      camera.up.set(...SCENE_UP);
      prevQuatRef.current = null;
      return;
    }

    const axes = computeLvlhAxes(originPosition, originVelocity);

    if (!axes) {
      // Fallback: radial-only tracking (no velocity available)
      const up = computeCameraUp(originPosition);
      camera.up.set(up[0], up[1], up[2]);
      prevQuatRef.current = null;
      return;
    }

    // LVLH basis: columns = [inTrack, crossTrack, radial] maps LVLH→ECI
    const basisMat = new THREE.Matrix4().makeBasis(
      new THREE.Vector3(...axes.inTrack),
      new THREE.Vector3(...axes.crossTrack),
      new THREE.Vector3(...axes.radial),
    );
    const targetQuat = new THREE.Quaternion().setFromRotationMatrix(basisMat);

    // Frame-rate-independent smoothing: slerp toward target
    const alpha = 1 - Math.exp(-SMOOTHING_SPEED * delta);
    const prevQuat = prevQuatRef.current;
    let smoothedQuat: THREE.Quaternion;

    if (prevQuat) {
      smoothedQuat = prevQuat.clone().slerp(targetQuat, alpha);
      // Delta: rotation from previous smoothed to current smoothed
      const deltaQuat = smoothedQuat.clone().multiply(prevQuat.clone().invert());
      camera.position.applyQuaternion(deltaQuat);
    } else {
      smoothedQuat = targetQuat;
    }

    // Extract smoothed radial direction (3rd column of smoothed rotation matrix)
    const m = new THREE.Matrix4().makeRotationFromQuaternion(smoothedQuat);
    const e = m.elements;
    camera.up.set(e[8], e[9], e[10]);

    prevQuatRef.current = smoothedQuat;
  }, -1); // Priority -1: run before OrbitControls (priority 0)

  return null;
}

/**
 * Wraps scene content in a group whose position smoothly tracks a target.
 *
 * Used for satellite-centered view: instead of subtracting the satellite's
 * position from every trail point / satellite / central body each frame,
 * the children render in central-body-relative coordinates and this group
 * smoothly translates them by -originPosition/scaleRadius.
 *
 * Snaps instantly when the target jumps by more than 1 scene unit (e.g.,
 * switching from central-body to satellite-centered mode).
 */
function SmoothOriginGroup({ children, targetPosition }: {
  children: React.ReactNode;
  targetPosition: [number, number, number];
}) {
  const groupRef = useRef<THREE.Group>(null);

  useFrame((_state, delta) => {
    const group = groupRef.current;
    if (!group) return;

    const [tx, ty, tz] = targetPosition;
    const dx = tx - group.position.x;
    const dy = ty - group.position.y;
    const dz = tz - group.position.z;
    const dist2 = dx * dx + dy * dy + dz * dz;

    // Snap for large jumps (mode switch); smooth for small updates (server data)
    if (dist2 > 1.0) {
      group.position.set(tx, ty, tz);
      return;
    }

    const alpha = 1 - Math.exp(-SMOOTHING_SPEED * delta);
    group.position.x += dx * alpha;
    group.position.y += dy * alpha;
    group.position.z += dz * alpha;
  });

  return <group ref={groupRef}>{children}</group>;
}

/**
 * Dynamically updates camera near/far planes based on the active display scale profile.
 * Must be rendered inside the Canvas tree.
 */
function CameraConfigurator({ profile }: { profile: DisplayScaleProfile }) {
  const { camera } = useThree();

  useEffect(() => {
    if (camera instanceof THREE.PerspectiveCamera) {
      camera.near = profile.cameraNear;
      camera.far = profile.cameraFar;
      camera.updateProjectionMatrix();
    }
  }, [camera, profile.cameraNear, profile.cameraFar]);

  return null;
}

/**
 * Snaps camera distance when transitioning between display scale profiles.
 * Runs at useFrame priority -2 (before CameraLvlhTracker at -1).
 */
function CameraDistanceTransition({ profile }: { profile: DisplayScaleProfile }) {
  const { camera } = useThree();
  const prevProfileRef = useRef(profile.name);

  useFrame(() => {
    if (profile.name !== prevProfileRef.current) {
      prevProfileRef.current = profile.name;
      // Snap camera to profile's default distance, keeping current direction
      const dir = camera.position.clone().normalize();
      if (dir.length() > 0) {
        camera.position.copy(dir.multiplyScalar(profile.defaultCameraDistance));
      }
    }
  }, -2);

  return null;
}

interface SceneProps {
  /** Points array for replay mode. */
  points?: OrbitPoint[] | null;
  /** Single satellite position (replay mode). */
  satellitePosition?: OrbitPoint | null;
  /** Visible count for replay mode progressive trail. */
  trailVisibleCount?: number;
  /** TrailBuffer for single-satellite realtime mode (backward compat). */
  trailBuffer?: TrailBuffer;
  /** Per-satellite TrailBuffers for multi-satellite realtime mode. */
  trailBuffers?: Map<string, TrailBuffer>;
  /** Per-satellite positions for multi-satellite mode. */
  satellitePositions?: Map<string, OrbitPoint | null>;
  /** Per-satellite visible counts for multi-satellite mode. */
  trailVisibleCounts?: Map<string, number>;
  /** Per-satellite draw start indices for time-range clipping. */
  trailDrawStarts?: Map<string, number>;
  /** Draw start index for single-satellite replay mode. */
  trailDrawStart?: number;
  centralBody: string;
  centralBodyRadius: number;
  /** Julian Date of the simulation epoch, or null if not set. */
  epochJd?: number | null;
  /** Reference frame for display (default: central-body inertial). */
  referenceFrame?: ReferenceFrame;
  /** Per-satellite metadata for model lookup. */
  satelliteNames?: Map<string, string | null>;
}

/**
 * Main Three.js scene component using @react-three/fiber Canvas.
 * Contains camera, controls, lights, central body, orbit trail(s), and satellite(s).
 */
export function Scene({
  points,
  satellitePosition,
  trailVisibleCount,
  trailBuffer,
  trailBuffers,
  satellitePositions,
  trailVisibleCounts,
  trailDrawStarts,
  trailDrawStart,
  centralBody,
  centralBodyRadius,
  epochJd,
  referenceFrame = DEFAULT_FRAME,
  satelliteNames,
}: SceneProps) {
  const isEcef = isLegacyEcef(referenceFrame);
  const isSatCentered = referenceFrame.center.type === "satellite";
  const centeredSatId = referenceFrame.center.type === "satellite" ? referenceFrame.center.id : null;

  // Display scale profile for the current view center
  const displayProfile = useMemo(
    () => getDisplayScaleProfile(referenceFrame.center),
    [referenceFrame.center],
  );

  // Scene amplification: scale up environment to show correct proportions
  // relative to the satellite's exaggerated model at origin.
  const sceneAmplification = useMemo(() => {
    if (!isSatCentered || centeredSatId == null) return 1;
    const modelConfig = getSatelliteModelConfig(
      centeredSatId,
      satelliteNames?.get(centeredSatId),
    );
    return computeSceneAmplification(modelConfig, centralBodyRadius);
  }, [isSatCentered, centeredSatId, satelliteNames, centralBodyRadius]);

  // Effective scale radius: smaller when amplified, so positions appear larger
  const effectiveScaleRadius = centralBodyRadius / sceneAmplification;

  // Compute origin position for satellite-centered view
  const originPosition: [number, number, number] | null = useMemo(() => {
    if (!isSatCentered || centeredSatId == null) return null;

    // Try multi-satellite mode first
    const satPos = satellitePositions?.get(centeredSatId);
    if (satPos) return [satPos.x, satPos.y, satPos.z];

    // Fall back to single satellite (replay mode)
    if (satellitePosition) return [satellitePosition.x, satellitePosition.y, satellitePosition.z];

    return null;
  }, [isSatCentered, centeredSatId, satellitePositions, satellitePosition]);

  // Compute origin velocity for LVLH axes
  const originVelocity: [number, number, number] | null = useMemo(() => {
    if (!isSatCentered || centeredSatId == null) return null;

    const satPos = satellitePositions?.get(centeredSatId);
    if (satPos) return [satPos.vx, satPos.vy, satPos.vz];

    if (satellitePosition) return [satellitePosition.vx, satellitePosition.vy, satellitePosition.vz];

    return null;
  }, [isSatCentered, centeredSatId, satellitePositions, satellitePosition]);

  // Compute LVLH axes for body-frame transformation
  const lvlhAxes: LvlhAxes | null = useMemo(
    () => computeLvlhAxes(originPosition, originVelocity),
    [originPosition, originVelocity],
  );

  // LVLH body-frame mode: active when satellite-centered with valid axes
  const lvlhActive = isSatCentered && lvlhAxes != null && originPosition != null;

  // Determine sim time for sun direction from first available satellite position
  const firstPosition = satellitePosition
    ?? (satellitePositions ? Array.from(satellitePositions.values()).find((p) => p != null) ?? null : null);
  const simTime = firstPosition?.t ?? 0;
  const quantizedSimTime = Math.floor(simTime / 60) * 60;

  // Sun direction in ECI (via WASM)
  const sunDirectionEci = useMemo(() => {
    if (epochJd == null) return DEFAULT_SUN_DIRECTION;
    const dir = sun_direction_eci(epochJd, quantizedSimTime);
    return new THREE.Vector3(dir[0], dir[1], dir[2]);
  }, [epochJd, quantizedSimTime]);

  // Earth rotation angle (ERA) via WASM — updates every frame via simTime (not quantized)
  const era = useMemo(() => {
    if (epochJd == null) return undefined;
    return earth_rotation_angle(epochJd, simTime);
  }, [epochJd, simTime]);

  // Sun direction in the display frame
  const sunDirection = useMemo(() => {
    if (lvlhActive) {
      // LVLH: rotate sun direction into satellite body frame
      const s = sunDirectionEci;
      const ax = lvlhAxes!;
      return new THREE.Vector3(
        ax.inTrack[0] * s.x + ax.inTrack[1] * s.y + ax.inTrack[2] * s.z,
        ax.crossTrack[0] * s.x + ax.crossTrack[1] * s.y + ax.crossTrack[2] * s.z,
        ax.radial[0] * s.x + ax.radial[1] * s.y + ax.radial[2] * s.z,
      );
    }
    if (!isEcef || era == null) return sunDirectionEci;
    // ECEF: rotate sun direction by -ERA to match Earth-fixed frame
    const [sx, sy, sz] = rotateZ(sunDirectionEci.x, sunDirectionEci.y, sunDirectionEci.z, -era);
    return new THREE.Vector3(sx, sy, sz);
  }, [sunDirectionEci, isEcef, era, lvlhActive, lvlhAxes]);

  const lightDistance = sceneAmplification * 10;
  const lightPosition = useMemo<[number, number, number]>(() => {
    return [sunDirection.x * lightDistance, sunDirection.y * lightDistance, sunDirection.z * lightDistance];
  }, [sunDirection, lightDistance]);

  // Earth rotation angle for the mesh: ERA in ECI, 0 in ECEF (Earth is static)
  const earthRotation = isEcef ? 0 : era;

  // Central body position and orientation in LVLH frame
  const bodyLvlhPosition = useMemo<[number, number, number] | null>(() => {
    if (!lvlhActive || originPosition == null || lvlhAxes == null) return null;
    return transformToLvlh(0, 0, 0, originPosition, lvlhAxes, effectiveScaleRadius);
  }, [lvlhActive, originPosition, lvlhAxes, effectiveScaleRadius]);

  const bodyLvlhQuaternion = useMemo<[number, number, number, number] | null>(() => {
    if (!lvlhActive || lvlhAxes == null) return null;
    // R_lvlh: basis matrix [inTrack, crossTrack, radial] maps LVLH→ECI
    const lvlhMat = new THREE.Matrix4().makeBasis(
      new THREE.Vector3(...lvlhAxes.inTrack),
      new THREE.Vector3(...lvlhAxes.crossTrack),
      new THREE.Vector3(...lvlhAxes.radial),
    );
    const lvlhQuat = new THREE.Quaternion().setFromRotationMatrix(lvlhMat);
    // R_z(ERA) rotation
    const eraQuat = new THREE.Quaternion().setFromEuler(new THREE.Euler(0, 0, era ?? 0));
    // R_x(π/2) pole alignment
    const poleQuat = new THREE.Quaternion().setFromEuler(new THREE.Euler(Math.PI / 2, 0, 0));
    // Body orientation in LVLH: R_lvlh^T * R_z(ERA) * R_x(π/2)
    const bodyQuat = lvlhQuat.clone().conjugate().multiply(eraQuat).multiply(poleQuat);
    return [bodyQuat.x, bodyQuat.y, bodyQuat.z, bodyQuat.w];
  }, [lvlhActive, lvlhAxes, era]);

  // Target offset for SmoothOriginGroup (non-LVLH satellite-centered fallback)
  const originOffset = useMemo<[number, number, number]>(() => {
    if (originPosition == null || lvlhActive) return [0, 0, 0];
    return [
      -originPosition[0] / centralBodyRadius,
      -originPosition[1] / centralBodyRadius,
      -originPosition[2] / centralBodyRadius,
    ];
  }, [originPosition, centralBodyRadius, lvlhActive]);

  // No useMemo: the trailBuffers Map reference (from useRef) never changes,
  // but Scene re-renders each frame via satellitePositions, so reading entries
  // inline picks up newly-added satellites.
  const multiSatEntries = trailBuffers
    ? Array.from(trailBuffers.entries()).filter(([, buf]) => buf.length > 0)
    : null;

  // Single-satellite backward compat
  const hasTrailData = trailBuffer
    ? trailBuffer.length > 0
    : points != null && points.length > 0;

  return (
    <Canvas
      camera={{ position: DEFAULT_CAMERA_POSITION, fov: 60, near: 0.01, far: 1000 }}
      gl={{ logarithmicDepthBuffer: true }}
      style={{ position: "absolute", top: 0, left: 0, width: "100%", height: "100%" }}
    >
      <CameraConfigurator profile={displayProfile} />
      <CameraDistanceTransition profile={displayProfile} />
      <OrbitControls
        enableDamping
        dampingFactor={0.1}
        minDistance={displayProfile.minDistance}
        maxDistance={displayProfile.maxDistance}
      />
      <CameraLvlhTracker originPosition={originPosition} originVelocity={originVelocity} lvlhActive={lvlhActive} />

      <ambientLight intensity={1.0} />
      <directionalLight intensity={2.0} position={lightPosition} />
      <hemisphereLight args={[0xffffff, 0x444466, 0.4]} />

      {/* Centered satellite: always exactly at world origin (0,0,0). */}
      {centeredSatId != null && multiSatEntries && (() => {
        const idx = multiSatEntries.findIndex(([id]) => id === centeredSatId);
        if (idx < 0) return null;
        const pos = satellitePositions?.get(centeredSatId);
        if (!pos) return null;
        return (
          <Satellite
            position={pos}
            scaleRadius={centralBodyRadius}
            color={SATELLITE_COLORS[idx % SATELLITE_COLORS.length]}
            referenceFrame={referenceFrame}
            epochJd={epochJd ?? undefined}
            satId={centeredSatId}
            satName={satelliteNames?.get(centeredSatId)}
            originPosition={originPosition}
            lvlhAxes={lvlhAxes}
          />
        );
      })()}
      {!multiSatEntries && isSatCentered && satellitePosition && (
        <Satellite
          position={satellitePosition}
          scaleRadius={centralBodyRadius}
          referenceFrame={referenceFrame}
          epochJd={epochJd ?? undefined}
          originPosition={originPosition}
          lvlhAxes={lvlhAxes}
        />
      )}

      {/* LVLH body-frame mode: all children render in satellite-centered LVLH
          coordinates. No SmoothOriginGroup needed — data is already transformed. */}
      {lvlhActive ? (
        <>
          <CelestialBody
            bodyId={centralBody}
            radius={sceneAmplification}
            sunDirection={sunDirection}
            rotationAngle={earthRotation}
            lvlhPosition={bodyLvlhPosition}
            lvlhQuaternion={bodyLvlhQuaternion}
          />

          {/* Multi-satellite mode */}
          {multiSatEntries && multiSatEntries.map(([satId, buf], index) => {
            const color = SATELLITE_COLORS[index % SATELLITE_COLORS.length];
            const vc = trailVisibleCounts?.get(satId);
            const pos = satellitePositions?.get(satId);
            const isCenteredSat = satId === centeredSatId;
            return (
              <group key={satId}>
                <OrbitTrail
                  trailBuffer={buf}
                  visibleCount={vc}
                  drawStart={trailDrawStarts?.get(satId)}
                  scaleRadius={effectiveScaleRadius}
                  color={color}
                  referenceFrame={referenceFrame}
                  epochJd={epochJd}
                  originPosition={originPosition}
                  lvlhAxes={lvlhAxes}
                />
                {pos && !isCenteredSat && (
                  <Satellite
                    position={pos}
                    scaleRadius={effectiveScaleRadius}
                    color={color}
                    referenceFrame={referenceFrame}
                    epochJd={epochJd ?? undefined}
                    satId={satId}
                    satName={satelliteNames?.get(satId)}
                    originPosition={originPosition}
                    lvlhAxes={lvlhAxes}
                  />
                )}
              </group>
            );
          })}

          {/* Single-satellite fallback (replay mode or legacy) */}
          {!multiSatEntries && hasTrailData && (
            trailBuffer ? (
              <OrbitTrail
                trailBuffer={trailBuffer}
                visibleCount={trailVisibleCount}
                drawStart={trailDrawStart}
                scaleRadius={effectiveScaleRadius}
                referenceFrame={referenceFrame}
                epochJd={epochJd}
                originPosition={originPosition}
                lvlhAxes={lvlhAxes}
              />
            ) : (
              <OrbitTrail
                points={points!}
                visibleCount={trailVisibleCount ?? points!.length}
                drawStart={trailDrawStart}
                scaleRadius={effectiveScaleRadius}
                referenceFrame={referenceFrame}
                epochJd={epochJd}
                originPosition={originPosition}
                lvlhAxes={lvlhAxes}
              />
            )
          )}
        </>
      ) : (
        /* Non-LVLH mode: SmoothOriginGroup handles satellite-centered offset. */
        <SmoothOriginGroup targetPosition={originOffset}>
          <CelestialBody bodyId={centralBody} sunDirection={sunDirection} rotationAngle={earthRotation} />

          {/* Multi-satellite mode */}
          {multiSatEntries && multiSatEntries.map(([satId, buf], index) => {
            const color = SATELLITE_COLORS[index % SATELLITE_COLORS.length];
            const vc = trailVisibleCounts?.get(satId);
            const pos = satellitePositions?.get(satId);
            const isCenteredSat = satId === centeredSatId;
            return (
              <group key={satId}>
                <OrbitTrail
                  trailBuffer={buf}
                  visibleCount={vc}
                  drawStart={trailDrawStarts?.get(satId)}
                  scaleRadius={centralBodyRadius}
                  color={color}
                  referenceFrame={referenceFrame}
                  epochJd={epochJd}
                />
                {pos && !isCenteredSat && (
                  <Satellite
                    position={pos}
                    scaleRadius={centralBodyRadius}
                    color={color}
                    referenceFrame={referenceFrame}
                    epochJd={epochJd ?? undefined}
                    satId={satId}
                    satName={satelliteNames?.get(satId)}
                  />
                )}
              </group>
            );
          })}

          {/* Single-satellite fallback (replay mode or legacy) */}
          {!multiSatEntries && hasTrailData && (
            trailBuffer ? (
              <OrbitTrail
                trailBuffer={trailBuffer}
                visibleCount={trailVisibleCount}
                drawStart={trailDrawStart}
                scaleRadius={centralBodyRadius}
                referenceFrame={referenceFrame}
                epochJd={epochJd}
              />
            ) : (
              <OrbitTrail
                points={points!}
                visibleCount={trailVisibleCount ?? points!.length}
                drawStart={trailDrawStart}
                scaleRadius={centralBodyRadius}
                referenceFrame={referenceFrame}
                epochJd={epochJd}
              />
            )
          )}
          {!multiSatEntries && satellitePosition && !isSatCentered && (
            <Satellite
              position={satellitePosition}
              scaleRadius={centralBodyRadius}
              referenceFrame={referenceFrame}
              epochJd={epochJd ?? undefined}
            />
          )}
        </SmoothOriginGroup>
      )}

      {/* Axes at world origin (= satellite position when satellite-centered) */}
      <axesHelper args={[2]} />
    </Canvas>
  );
}
