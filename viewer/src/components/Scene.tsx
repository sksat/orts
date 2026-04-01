import { OrbitControls } from "@react-three/drei";
import { Canvas, useFrame, useThree } from "@react-three/fiber";
import { useEffect, useMemo, useRef } from "react";
import * as THREE from "three";
import { transformToLvlh } from "../coordTransform.js";
import {
  computeSceneAmplification,
  type DisplayScaleProfile,
  getDisplayScaleProfile,
} from "../displayScale.js";
import { rotateZ } from "../frameTransform.js";
import type { OrbitPoint } from "../orbit.js";
import { DEFAULT_FRAME, isLegacyEcef, type ReferenceFrame } from "../referenceFrame.js";
import { getSatelliteModelConfig } from "../satelliteModels.js";
import {
  computeCameraUp,
  computeLvlhAxes,
  DEFAULT_CAMERA_POSITION,
  type LvlhAxes,
  SCENE_UP,
} from "../sceneFrame.js";
import type { TrailBuffer } from "../utils/TrailBuffer.js";
import {
  body_orientation,
  earth_rotation_angle,
  eci_to_ecef,
  sun_direction_from_body,
  sun_distance_from_body,
} from "../wasm/kanameInit.js";
import { entityPathToBodyId, getBodyRadius } from "../bodies.js";
import { CelestialBody } from "./CelestialBody.js";
import { OrbitTrail } from "./OrbitTrail.js";
import { Satellite } from "./Satellite.js";

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
function CameraLvlhTracker({
  originPosition,
  originVelocity,
  lvlhActive,
}: {
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
function SmoothOriginGroup({
  children,
  targetPosition,
}: {
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
 * Snaps camera position when transitioning between display scale profiles.
 * Uses the profile's default direction if specified, otherwise keeps current direction.
 * Runs at useFrame priority -2 (before CameraLvlhTracker at -1).
 */
function CameraDistanceTransition({
  profile,
  overrideDistance,
}: {
  profile: DisplayScaleProfile;
  overrideDistance?: number;
}) {
  const { camera } = useThree();
  const prevKeyRef = useRef(`${profile.name}:${overrideDistance ?? ""}`);

  useFrame(() => {
    const key = `${profile.name}:${overrideDistance ?? ""}`;
    if (key !== prevKeyRef.current) {
      prevKeyRef.current = key;
      const d = overrideDistance ?? profile.defaultCameraDistance;
      if (profile.defaultCameraDirection) {
        const [dx, dy, dz] = profile.defaultCameraDirection;
        camera.position.set(dx * d, dy * d, dz * d);
      } else {
        const dir = camera.position.clone().normalize();
        if (dir.length() > 0) {
          camera.position.copy(dir.multiplyScalar(d));
        }
      }
    }
  }, -2);

  return null;
}

/**
 * Renders a secondary celestial body (e.g., Moon) at the correct position
 * with a textured sphere scaled to its physical radius.
 */
function SecondaryBody({
  bodyId,
  position,
  scaleRadius,
  sunDirection,
  referenceFrame = DEFAULT_FRAME,
  epochJd,
  originPosition = null,
  lvlhAxes = null,
  textureRevision,
  textureBaseUrl,
}: {
  bodyId: string;
  position: OrbitPoint;
  scaleRadius: number;
  sunDirection?: THREE.Vector3;
  referenceFrame?: ReferenceFrame;
  epochJd?: number | null;
  originPosition?: [number, number, number] | null;
  lvlhAxes?: LvlhAxes | null;
  textureRevision?: number;
  textureBaseUrl?: string;
}) {
  const bodyRadiusKm = getBodyRadius(bodyId);
  const radius = bodyRadiusKm != null ? bodyRadiusKm / scaleRadius : 0.01;

  // Position transform: same pipeline as Satellite (ECI → ECEF → LVLH)
  let scenePos: [number, number, number];
  if (isLegacyEcef(referenceFrame) && epochJd != null) {
    const ecef = eci_to_ecef(position.x, position.y, position.z, epochJd, position.t);
    scenePos = [ecef[0] / scaleRadius, ecef[1] / scaleRadius, ecef[2] / scaleRadius];
  } else if (originPosition != null && lvlhAxes != null) {
    scenePos = transformToLvlh(
      position.x,
      position.y,
      position.z,
      originPosition,
      lvlhAxes,
      scaleRadius,
    );
  } else if (originPosition != null) {
    scenePos = [
      (position.x - originPosition[0]) / scaleRadius,
      (position.y - originPosition[1]) / scaleRadius,
      (position.z - originPosition[2]) / scaleRadius,
    ];
  } else {
    scenePos = [position.x / scaleRadius, position.y / scaleRadius, position.z / scaleRadius];
  }

  // Body orientation via IAU rotation model (kaname WASM).
  // IAU quaternion is body-fixed → ECI. For non-inertial display frames
  // (ECEF, LVLH), we must apply the same frame rotation as positions get.
  const orientation = useMemo(() => {
    if (epochJd == null) return undefined;
    const q = body_orientation(bodyId, epochJd, position.t);
    if (!q) return undefined;
    // IAU body-fixed → ECI: q = [w, x, y, z]
    const iauQuat = new THREE.Quaternion(q[1], q[2], q[3], q[0]); // THREE uses (x,y,z,w)
    // Pole alignment: rotate +Y (Three.js pole) → +Z (IAU pole)
    const poleAlign = new THREE.Quaternion().setFromEuler(new THREE.Euler(Math.PI / 2, 0, 0));
    // body-fixed → ECI → (optional frame rotation)
    let combined = iauQuat.multiply(poleAlign);
    // ECEF: apply inverse Earth rotation (same as position transform)
    if (isLegacyEcef(referenceFrame) && epochJd != null) {
      const era = earth_rotation_angle(epochJd, position.t);
      const ecefRot = new THREE.Quaternion().setFromEuler(new THREE.Euler(0, 0, -era));
      combined = ecefRot.multiply(combined);
    }
    return combined;
  }, [bodyId, epochJd, position.t, referenceFrame]);

  return (
    <group position={scenePos} quaternion={orientation ?? undefined}>
      <CelestialBody
        bodyId={bodyId}
        radius={radius}
        sunDirection={sunDirection}
        textureRevision={textureRevision}
        textureBaseUrl={textureBaseUrl}
      />
    </group>
  );
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
  /** When true, atmosphere uses physical scale. Default: auto (true for satellite-centered). */
  physicalScale?: boolean;
  /** Bumped when server notifies high-res textures are available. */
  textureRevision?: number;
  /** Base URL for fetching high-res textures (e.g., "http://localhost:9001/textures/"). */
  textureBaseUrl?: string;
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
  physicalScale,
  textureRevision,
  textureBaseUrl,
}: SceneProps) {
  const isEcef = isLegacyEcef(referenceFrame);
  const isSatCentered = referenceFrame.center.type === "satellite";
  const centeredSatId =
    referenceFrame.center.type === "satellite" ? referenceFrame.center.id : null;

  // Detect if centered entity is a celestial body
  const centeredBodyId = centeredSatId != null ? entityPathToBodyId(centeredSatId) : null;

  // Display scale profile for the current view center
  const displayProfile = useMemo(
    () => getDisplayScaleProfile(referenceFrame.center),
    [referenceFrame.center],
  );

  // Override camera distance when centering on a known body
  const cameraDistanceOverride = useMemo(() => {
    if (centeredBodyId == null) return undefined;
    const bodyRadiusKm = getBodyRadius(centeredBodyId);
    if (bodyRadiusKm == null) return undefined;
    // Camera at ~3x body radius in scene units
    return (bodyRadiusKm / centralBodyRadius) * 3;
  }, [centeredBodyId, centralBodyRadius]);

  // Scene amplification: scale up environment to show correct proportions
  // relative to the satellite's exaggerated model at origin.
  const sceneAmplification = useMemo(() => {
    if (!isSatCentered || centeredSatId == null) return 1;
    // Body entities (Moon, Sun, etc.) don't need satellite amplification
    if (centeredBodyId != null) return 1;
    const modelConfig = getSatelliteModelConfig(centeredSatId, satelliteNames?.get(centeredSatId));
    return computeSceneAmplification(modelConfig, centralBodyRadius);
  }, [isSatCentered, centeredSatId, centeredBodyId, satelliteNames, centralBodyRadius]);

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

    if (satellitePosition)
      return [satellitePosition.vx, satellitePosition.vy, satellitePosition.vz];

    return null;
  }, [isSatCentered, centeredSatId, satellitePositions, satellitePosition]);

  // Compute LVLH axes for body-frame transformation
  const lvlhAxes: LvlhAxes | null = useMemo(
    () => computeLvlhAxes(originPosition, originVelocity),
    [originPosition, originVelocity],
  );

  // LVLH body-frame mode: active when satellite-centered (not body entity) with valid axes.
  // Body entities (Moon, Mars) use IAU rotation in ECI, not LVLH.
  const lvlhActive =
    isSatCentered && centeredBodyId == null && lvlhAxes != null && originPosition != null;

  // Determine sim time for sun direction from first available satellite position
  const firstPosition =
    satellitePosition ??
    (satellitePositions
      ? (Array.from(satellitePositions.values()).find((p) => p != null) ?? null)
      : null);
  const simTime = firstPosition?.t ?? 0;
  const quantizedSimTime = Math.floor(simTime / 60) * 60;

  // Sun direction in body-centered inertial frame (via WASM)
  const sunDirectionEci = useMemo(() => {
    if (epochJd == null) return DEFAULT_SUN_DIRECTION;
    const dir = sun_direction_from_body(centralBody, epochJd, quantizedSimTime);
    return new THREE.Vector3(dir[0], dir[1], dir[2]);
  }, [centralBody, epochJd, quantizedSimTime]);

  // Sun intensity: inverse square law based on body-Sun distance
  const AU_KM = 149_597_870.7;
  const sunIntensity = useMemo(() => {
    if (epochJd == null) return 1.0;
    const distKm = sun_distance_from_body(centralBody, epochJd, quantizedSimTime);
    return (AU_KM / distKm) ** 2;
  }, [centralBody, epochJd, quantizedSimTime]);

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
    return [
      sunDirection.x * lightDistance,
      sunDirection.y * lightDistance,
      sunDirection.z * lightDistance,
    ];
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
  const hasTrailData = trailBuffer ? trailBuffer.length > 0 : points != null && points.length > 0;

  return (
    <Canvas
      camera={{ position: DEFAULT_CAMERA_POSITION, fov: 60, near: 0.01, far: 1000 }}
      gl={{ logarithmicDepthBuffer: true }}
      style={{ position: "absolute", top: 0, left: 0, width: "100%", height: "100%" }}
    >
      <CameraConfigurator profile={displayProfile} />
      <CameraDistanceTransition profile={displayProfile} overrideDistance={cameraDistanceOverride} />
      <OrbitControls
        enableDamping
        dampingFactor={0.1}
        minDistance={displayProfile.minDistance}
        maxDistance={displayProfile.maxDistance}
      />
      <CameraLvlhTracker
        originPosition={originPosition}
        originVelocity={originVelocity}
        lvlhActive={lvlhActive}
      />

      <ambientLight intensity={0.15} />
      <directionalLight intensity={3.0 * sunIntensity} position={lightPosition} />

      {/* Centered satellite/body: always exactly at world origin (0,0,0). */}
      {centeredSatId != null &&
        multiSatEntries &&
        (() => {
          const idx = multiSatEntries.findIndex(([id]) => id === centeredSatId);
          if (idx < 0) return null;
          const pos = satellitePositions?.get(centeredSatId);
          if (!pos) return null;
          const centeredBodyId = entityPathToBodyId(centeredSatId);
          if (centeredBodyId != null) {
            // Render as CelestialBody at origin with physical radius + IAU orientation
            const bodyRadiusKm = getBodyRadius(centeredBodyId);
            const bodyRadius = bodyRadiusKm != null ? bodyRadiusKm / centralBodyRadius : 0.01;
            const q = epochJd != null ? body_orientation(centeredBodyId, epochJd, pos.t) : undefined;
            const iauQuat = q
              ? new THREE.Quaternion(q[1], q[2], q[3], q[0])
                  .multiply(new THREE.Quaternion().setFromEuler(new THREE.Euler(Math.PI / 2, 0, 0)))
              : undefined;
            return (
              <group quaternion={iauQuat ?? undefined}>
                <CelestialBody
                  bodyId={centeredBodyId}
                  radius={bodyRadius}
                  sunDirection={sunDirection}
                  textureRevision={textureRevision}
                  textureBaseUrl={textureBaseUrl}
                />
              </group>
            );
          }
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

      {/* All scene objects in a single stable tree — no ternary remounting.
          SmoothOriginGroup handles non-LVLH satellite-centered offset;
          in LVLH or body-centered mode originOffset is [0,0,0] (no-op). */}
      <SmoothOriginGroup targetPosition={originOffset}>
        <CelestialBody
          bodyId={centralBody}
          radius={lvlhActive ? sceneAmplification : 1}
          sunDirection={sunDirection}
          rotationAngle={earthRotation}
          lvlhPosition={lvlhActive ? bodyLvlhPosition : null}
          lvlhQuaternion={lvlhActive ? bodyLvlhQuaternion : null}
          ambientIntensity={0.15}
          sunIntensity={sunIntensity}
          physicalScale={physicalScale}
          textureRevision={textureRevision}
          textureBaseUrl={textureBaseUrl}
        />

        {/* Multi-satellite mode */}
        {multiSatEntries?.map(([satId, buf], index) => {
          const color = SATELLITE_COLORS[index % SATELLITE_COLORS.length];
          const vc = trailVisibleCounts?.get(satId);
          const pos = satellitePositions?.get(satId);
          const isCenteredSat = satId === centeredSatId;
          const trailScale = lvlhActive ? effectiveScaleRadius : centralBodyRadius;
          const bodyId = entityPathToBodyId(satId);
          return (
            <group key={satId}>
              <OrbitTrail
                trailBuffer={buf}
                visibleCount={vc}
                drawStart={trailDrawStarts?.get(satId)}
                scaleRadius={trailScale}
                color={color}
                referenceFrame={referenceFrame}
                epochJd={epochJd}
                originPosition={lvlhActive ? originPosition : null}
                lvlhAxes={lvlhActive ? lvlhAxes : null}
              />
              {pos && !isCenteredSat && bodyId != null && (
                <SecondaryBody
                  bodyId={bodyId}
                  position={pos}
                  scaleRadius={trailScale}
                  sunDirection={sunDirection}
                  referenceFrame={referenceFrame}
                  epochJd={epochJd}
                  originPosition={lvlhActive ? originPosition : null}
                  lvlhAxes={lvlhActive ? lvlhAxes : null}
                  textureRevision={textureRevision}
                  textureBaseUrl={textureBaseUrl}
                />
              )}
              {pos && !isCenteredSat && bodyId == null && (
                <Satellite
                  position={pos}
                  scaleRadius={trailScale}
                  color={color}
                  referenceFrame={referenceFrame}
                  epochJd={epochJd ?? undefined}
                  satId={satId}
                  satName={satelliteNames?.get(satId)}
                  originPosition={lvlhActive ? originPosition : null}
                  lvlhAxes={lvlhActive ? lvlhAxes : null}
                />
              )}
            </group>
          );
        })}

        {/* Single-satellite fallback (replay mode or legacy) */}
        {!multiSatEntries &&
          hasTrailData &&
          (() => {
            const trailScale = lvlhActive ? effectiveScaleRadius : centralBodyRadius;
            return trailBuffer ? (
              <OrbitTrail
                trailBuffer={trailBuffer}
                visibleCount={trailVisibleCount}
                drawStart={trailDrawStart}
                scaleRadius={trailScale}
                referenceFrame={referenceFrame}
                epochJd={epochJd}
                originPosition={lvlhActive ? originPosition : null}
                lvlhAxes={lvlhActive ? lvlhAxes : null}
              />
            ) : (
              <OrbitTrail
                points={points!}
                visibleCount={trailVisibleCount ?? points?.length}
                drawStart={trailDrawStart}
                scaleRadius={trailScale}
                referenceFrame={referenceFrame}
                epochJd={epochJd}
                originPosition={lvlhActive ? originPosition : null}
                lvlhAxes={lvlhActive ? lvlhAxes : null}
              />
            );
          })()}
        {!multiSatEntries && satellitePosition && !isSatCentered && (
          <Satellite
            position={satellitePosition}
            scaleRadius={centralBodyRadius}
            referenceFrame={referenceFrame}
            epochJd={epochJd ?? undefined}
          />
        )}
      </SmoothOriginGroup>

      {/* Reference axes: full ECI axes for body-centered, small LVLH reference for satellite-centered */}
      <axesHelper args={[isSatCentered ? 0.015 : 2]} />
    </Canvas>
  );
}
