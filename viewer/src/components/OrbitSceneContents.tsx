import { OrbitControls, type OrbitControlsProps } from "@react-three/drei";
import { useFrame, useThree } from "@react-three/fiber";
import { useEffect, useMemo, useRef } from "react";
import * as THREE from "three";
import {
  type BodyDefinitions,
  DEFAULT_BODIES,
  entityPathToBodyId,
  getBodyRadius,
} from "../bodies.js";
import { displayPosition, displayRotation, resolveDisplayFrame } from "../displayFrame.js";
import {
  computeSceneAmplification,
  type DisplayScaleProfile,
  getDisplayScaleProfile,
} from "../displayScale.js";
import { IS_DEV } from "../env.js";
import { resolveSceneFrame } from "../frameResolve.js";
import type { OrbitPoint } from "../orbit.js";
import { DEFAULT_FRAME, isLegacyEcef, type ReferenceFrame } from "../referenceFrame.js";
import { getSatelliteModelConfig } from "../satelliteModels.js";
import { type MarkerShape, resolveMarkerShape } from "../satelliteShapes.js";
import { computeCameraUp, computeLvlhAxes, type LvlhAxes, SCENE_UP } from "../sceneFrame.js";
import type { TrailBufferLike } from "../utils/TrailBuffer.js";
import { body_orientation, earth_rotation_angle } from "../wasm/arikaInit.js";
import { CelestialBody } from "./CelestialBody.js";
import { OrbitTrail } from "./OrbitTrail.js";
import { buildRenderEntries } from "./renderEntries.js";
import { Satellite } from "./Satellite.js";
import { AMBIENT_INTENSITY, SunLighting, useSunLighting } from "./SunLighting.js";

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
  // Empty sentinel so the first frame also snaps to the active profile's default
  // placement — not only when switching profiles. This matters when the viewer
  // *starts* in a non-default frame (e.g. an embedder mounting satellite-centred):
  // otherwise the camera keeps the generic initial position instead of the
  // profile's framing. (Body-centred uses defaultCameraDirection=null, so the
  // initial snap only normalises distance and leaves the app's view unchanged.)
  const prevKeyRef = useRef("");

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
  bodyDefinitions,
}: {
  bodyId: string;
  position: OrbitPoint;
  scaleRadius: number;
  sunDirection?: THREE.Vector3;
  referenceFrame?: ReferenceFrame;
  epochJd?: number | null;
  originPosition?: [number, number, number] | null;
  lvlhAxes?: LvlhAxes | null;
  textureRevision?: number | string;
  textureBaseUrl?: string;
  bodyDefinitions: BodyDefinitions;
}) {
  const bodyRadiusKm = getBodyRadius(bodyId, bodyDefinitions);
  const radius = bodyRadiusKm != null ? bodyRadiusKm / scaleRadius : 0.01;

  // Position and orientation share one display frame (see displayFrame.ts), so
  // the mesh can never be placed in one basis and oriented in another.
  const era =
    isLegacyEcef(referenceFrame) && epochJd != null
      ? earth_rotation_angle(epochJd, position.t)
      : null;
  const frame = resolveDisplayFrame(referenceFrame, { era, originPosition, lvlhAxes });
  const scenePos = displayPosition(frame, position.x, position.y, position.z, scaleRadius);

  // Body-fixed → inertial orientation via the IAU rotation model (arika WASM),
  // including the Three.js +Y-pole → IAU +Z-pole alignment.
  const bodyToInertial = useMemo(() => {
    if (epochJd == null) return undefined;
    const q = body_orientation(bodyId, epochJd, position.t);
    if (!q) return undefined;
    // IAU body-fixed → ECI: q = [w, x, y, z]; THREE uses (x, y, z, w)
    const iauQuat = new THREE.Quaternion(q[1], q[2], q[3], q[0]);
    const poleAlign = new THREE.Quaternion().setFromEuler(new THREE.Euler(Math.PI / 2, 0, 0));
    return iauQuat.multiply(poleAlign);
  }, [bodyId, epochJd, position.t]);

  const orientation = bodyToInertial ? displayRotation(frame).multiply(bodyToInertial) : undefined;

  return (
    <group position={scenePos} quaternion={orientation ?? undefined}>
      <CelestialBody
        bodyId={bodyId}
        radius={radius}
        sunDirection={sunDirection}
        textureRevision={textureRevision}
        textureBaseUrl={textureBaseUrl}
        bodyDefinitions={bodyDefinitions}
      />
    </group>
  );
}

export interface OrbitSceneContentsProps {
  /** Per-satellite trail buffers (all source types). */
  trailBuffers?: Map<string, TrailBufferLike>;
  /** Per-satellite positions. */
  satellitePositions?: Map<string, OrbitPoint | null>;
  /** Per-satellite visible counts (when not live). */
  trailVisibleCounts?: Map<string, number>;
  /** Per-satellite draw start indices for time-range clipping. */
  trailDrawStarts?: Map<string, number>;
  centralBody: string;
  centralBodyRadius: number;
  /** Body definitions (render info + radii) for the central body and any secondary bodies. */
  bodyDefinitions?: BodyDefinitions;
  /** Julian Date of the simulation epoch, or null if not set. */
  epochJd?: number | null;
  /** Reference frame for display (default: central-body inertial). */
  referenceFrame?: ReferenceFrame;
  /** Per-satellite metadata for model lookup. */
  satelliteNames?: Map<string, string | null>;
  /** Per-satellite marker/trail colour override (0xRRGGBB); falls back to the palette. */
  satelliteColors?: Map<string, number | undefined>;
  /** Per-satellite marker shape override; falls back to {@link defaultMarkerShape} then auto. */
  satelliteShapes?: Map<string, MarkerShape>;
  /** Sim-declared per-satellite marker shapes (from SatelliteInfo); below override, above default. */
  satelliteSimShapes?: Map<string, MarkerShape>;
  /** Global default marker shape (null/undefined = automatic per attitude). */
  defaultMarkerShape?: MarkerShape | null;
  /** When true, atmosphere uses physical scale. Default: auto (true for satellite-centered). */
  physicalScale?: boolean;
  /** Bumped when server notifies high-res textures are available. */
  textureRevision?: number | string;
  /** Base URL for fetching high-res textures (e.g., "http://localhost:9001/textures/"). */
  textureBaseUrl?: string;
  /**
   * Default orbit camera package: OrbitControls plus the frame-aware camera rig
   * (near/far configurator, distance transitions, LVLH tracking). `true`
   * (default) renders it; `false` hands the camera entirely to the caller; an
   * object enables it and is spread onto OrbitControls. Disabling it means the
   * caller is responsible for camera placement, near/far, and `camera.up`.
   */
  controls?: boolean | Partial<OrbitControlsProps>;
  /** Render the reference axes helper. Default `true`. */
  axes?: boolean;
}

/**
 * The contents of the Three.js scene: camera rig, controls, lights, central
 * body, orbit trail(s), and satellite(s). Rendered inside a @react-three/fiber
 * Canvas — both the app's {@link Scene} and the embeddable OrbitViewer mount
 * this same graph, so frame/lighting logic lives in exactly one place.
 */
export function OrbitSceneContents({
  trailBuffers,
  satellitePositions,
  trailVisibleCounts,
  trailDrawStarts,
  centralBody,
  centralBodyRadius,
  bodyDefinitions = DEFAULT_BODIES,
  epochJd,
  referenceFrame = DEFAULT_FRAME,
  satelliteNames,
  satelliteColors,
  satelliteShapes,
  satelliteSimShapes,
  defaultMarkerShape,
  physicalScale,
  textureRevision,
  textureBaseUrl,
  controls = true,
  axes = true,
}: OrbitSceneContentsProps) {
  // Default camera package (controls + frame-aware rig). `false` opts out
  // entirely; an object enables it and is spread onto OrbitControls.
  const controlsEnabled = controls !== false;
  const controlsProps = typeof controls === "object" ? controls : undefined;

  const isEcef = isLegacyEcef(referenceFrame);
  const isSatCentered = referenceFrame.center.type === "satellite";
  const centeredSatId =
    referenceFrame.center.type === "satellite" ? referenceFrame.center.id : null;

  // Detect if centered entity is a celestial body
  const centeredBodyId =
    centeredSatId != null ? entityPathToBodyId(centeredSatId, bodyDefinitions) : null;

  // Display scale profile for the current view center
  const displayProfile = useMemo(
    () => getDisplayScaleProfile(referenceFrame.center),
    [referenceFrame.center],
  );

  // Override camera distance when centering on a known body
  const cameraDistanceOverride = useMemo(() => {
    if (centeredBodyId == null) return undefined;
    const bodyRadiusKm = getBodyRadius(centeredBodyId, bodyDefinitions);
    if (bodyRadiusKm == null) return undefined;
    // Camera at ~3x body radius in scene units
    return (bodyRadiusKm / centralBodyRadius) * 3;
  }, [centeredBodyId, centralBodyRadius, bodyDefinitions]);

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

  // Resolve the frame semantics (origin, LVLH axes, camera behaviour) through
  // the shared kernel — the one place that honours `orientation` (#90).
  const { originPosition, originVelocity, lvlhAxes, lvlhActive, cameraTracking } = useMemo(
    () =>
      resolveSceneFrame(
        referenceFrame,
        (id) => {
          const p = satellitePositions?.get(id);
          return p ? { position: [p.x, p.y, p.z], velocity: [p.vx, p.vy, p.vz] } : null;
        },
        (id) => entityPathToBodyId(id, bodyDefinitions) != null,
      ),
    [referenceFrame, satellitePositions, bodyDefinitions],
  );

  // Dev/E2E-only: expose the resolved frame semantics so tests can assert
  // inertial vs local-orbital behaviour without reading pixels.
  useEffect(() => {
    if (!IS_DEV) return;
    const w = window as unknown as Record<string, unknown>;
    w.__debug_scene_frame = { lvlhActive, cameraTracking, originPosition };
    return () => {
      delete w.__debug_scene_frame;
    };
  }, [lvlhActive, cameraTracking, originPosition]);

  // Determine sim time for sun direction from the first available satellite
  // position. Iterate the Map's values directly rather than materializing an
  // array every render just to find the first non-null entry.
  let firstPosition: OrbitPoint | null = null;
  if (satellitePositions) {
    for (const p of satellitePositions.values()) {
      if (p != null) {
        firstPosition = p;
        break;
      }
    }
  }
  const simTime = firstPosition?.t ?? 0;
  const quantizedSimTime = Math.floor(simTime / 60) * 60;

  // Earth rotation angle (ERA) via WASM — updates every frame via simTime (not quantized)
  const era = useMemo(() => {
    if (epochJd == null) return undefined;
    return earth_rotation_angle(epochJd, simTime);
  }, [epochJd, simTime]);

  // Sun direction + intensity (display frame). Shared by the lights and by the
  // lit bodies below, so the scene holds them and passes them down explicitly.
  const { sunDirection, sunIntensity, lightPosition } = useSunLighting({
    centralBody,
    epochJd,
    quantizedSimTime,
    isEcef,
    era,
    lvlhActive,
    lvlhAxes,
    sceneAmplification,
  });

  // Earth rotation angle for the mesh: ERA in ECI, 0 in ECEF (Earth is static)
  const earthRotation = isEcef ? 0 : era;

  // Central body position and orientation in LVLH frame
  const bodyLvlhPosition = useMemo<[number, number, number] | null>(() => {
    if (!lvlhActive || originPosition == null || lvlhAxes == null) return null;
    return displayPosition(
      { kind: "localOrbital", origin: originPosition, axes: lvlhAxes },
      0,
      0,
      0,
      effectiveScaleRadius,
    );
  }, [lvlhActive, originPosition, lvlhAxes, effectiveScaleRadius]);

  const bodyLvlhQuaternion = useMemo<[number, number, number, number] | null>(() => {
    if (!lvlhActive || lvlhAxes == null || originPosition == null) return null;
    let bodyToInertialQuat: THREE.Quaternion;
    if (centralBody === "earth") {
      // Keep Earth in the same simple ERA frame as position/geodetic transforms.
      // EarthBody applies the Three.js mesh pole alignment internally.
      bodyToInertialQuat = new THREE.Quaternion().setFromEuler(new THREE.Euler(0, 0, era ?? 0));
    } else {
      const q = epochJd != null ? body_orientation(centralBody, epochJd, simTime) : undefined;
      bodyToInertialQuat = q
        ? new THREE.Quaternion(q[1], q[2], q[3], q[0])
        : new THREE.Quaternion();

      // Non-Earth bodies use TexturedBody/FallbackBody, which do not apply
      // Three.js Y-pole → body-frame Z-pole alignment internally.
      bodyToInertialQuat.multiply(
        new THREE.Quaternion().setFromEuler(new THREE.Euler(Math.PI / 2, 0, 0)),
      );
    }

    // Body orientation in the display frame: R_inertial→lvlh * R_body→inertial,
    // with the rotation taken from the same basis the positions use.
    const bodyQuat = displayRotation({
      kind: "localOrbital",
      origin: originPosition,
      axes: lvlhAxes,
    }).multiply(bodyToInertialQuat);
    return [bodyQuat.x, bodyQuat.y, bodyQuat.z, bodyQuat.w];
  }, [lvlhActive, lvlhAxes, originPosition, epochJd, centralBody, simTime, era]);

  // Target offset for SmoothOriginGroup (non-LVLH satellite-centered fallback)
  const originOffset = useMemo<[number, number, number]>(() => {
    if (originPosition == null || lvlhActive) return [0, 0, 0];
    return [
      -originPosition[0] / centralBodyRadius,
      -originPosition[1] / centralBodyRadius,
      -originPosition[2] / centralBodyRadius,
    ];
  }, [originPosition, centralBodyRadius, lvlhActive]);

  // No useMemo: the Maps' references (from refs) never change, but the scene
  // re-renders each frame, so reading inline picks up newly-added satellites.
  // Satellites are rendered from the union of trails and positions, so one given
  // only a position (no trail) still shows a marker.
  const renderEntries = buildRenderEntries(trailBuffers, satellitePositions);

  return (
    <>
      {/* Default camera package: render only when `controls` is not false, so a
          bring-your-own-Canvas caller can take over the camera entirely. */}
      {controlsEnabled && (
        <>
          <CameraConfigurator profile={displayProfile} />
          <CameraDistanceTransition
            profile={displayProfile}
            overrideDistance={cameraDistanceOverride}
          />
          <OrbitControls
            enableDamping
            dampingFactor={0.1}
            minDistance={displayProfile.minDistance}
            maxDistance={displayProfile.maxDistance}
            {...controlsProps}
          />
          {/* Camera co-rotation only when the kernel asked for it (local-orbital
              approximated by the camera); an inertial centre keeps star-fixed axes. */}
          <CameraLvlhTracker
            originPosition={cameraTracking ? originPosition : null}
            originVelocity={originVelocity}
            lvlhActive={lvlhActive}
          />
        </>
      )}

      <SunLighting intensity={sunIntensity} position={lightPosition} />

      {/* Centered satellite/body: always exactly at world origin (0,0,0). */}
      {centeredSatId != null &&
        (() => {
          const pos = satellitePositions?.get(centeredSatId);
          if (!pos) return null;
          const idx = renderEntries.findIndex((e) => e.satId === centeredSatId);
          const color =
            satelliteColors?.get(centeredSatId) ??
            SATELLITE_COLORS[(idx < 0 ? 0 : idx) % SATELLITE_COLORS.length];
          const centeredBodyId = entityPathToBodyId(centeredSatId, bodyDefinitions);
          if (centeredBodyId != null) {
            // Render as CelestialBody at origin with physical radius + IAU orientation
            const bodyRadiusKm = getBodyRadius(centeredBodyId, bodyDefinitions);
            const bodyRadius = bodyRadiusKm != null ? bodyRadiusKm / centralBodyRadius : 0.01;
            const q =
              epochJd != null ? body_orientation(centeredBodyId, epochJd, pos.t) : undefined;
            const iauQuat = q
              ? new THREE.Quaternion(q[1], q[2], q[3], q[0]).multiply(
                  new THREE.Quaternion().setFromEuler(new THREE.Euler(Math.PI / 2, 0, 0)),
                )
              : undefined;
            return (
              <group quaternion={iauQuat ?? undefined}>
                <CelestialBody
                  bodyId={centeredBodyId}
                  radius={bodyRadius}
                  sunDirection={sunDirection}
                  textureRevision={textureRevision}
                  textureBaseUrl={textureBaseUrl}
                  bodyDefinitions={bodyDefinitions}
                />
              </group>
            );
          }
          return (
            <Satellite
              position={pos}
              scaleRadius={centralBodyRadius}
              color={color}
              referenceFrame={referenceFrame}
              epochJd={epochJd ?? undefined}
              satId={centeredSatId}
              satName={satelliteNames?.get(centeredSatId)}
              originPosition={originPosition}
              lvlhAxes={lvlhAxes}
              markerShape={resolveMarkerShape({
                override: satelliteShapes?.get(centeredSatId),
                simShape: satelliteSimShapes?.get(centeredSatId),
                globalDefault: defaultMarkerShape,
                hasAttitude: pos.qw != null,
              })}
            />
          );
        })()}

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
          ambientIntensity={AMBIENT_INTENSITY}
          sunIntensity={sunIntensity}
          physicalScale={physicalScale}
          textureRevision={textureRevision}
          textureBaseUrl={textureBaseUrl}
          bodyDefinitions={bodyDefinitions}
        />

        {/* One entry per satellite that has a trail and/or a position. */}
        {renderEntries.map(({ satId, buf, pos }, index) => {
          const color =
            satelliteColors?.get(satId) ?? SATELLITE_COLORS[index % SATELLITE_COLORS.length];
          const isCenteredSat = satId === centeredSatId;
          const trailScale = lvlhActive ? effectiveScaleRadius : centralBodyRadius;
          const bodyId = entityPathToBodyId(satId, bodyDefinitions);
          return (
            <group key={satId}>
              {/* Mount on buffer *existence*, not current length: contents may be
                  filled in a commit-phase effect after this render (#91), and an
                  empty buffer draws nothing while OrbitTrail picks up points in
                  useFrame — no re-render needed when they arrive. */}
              {buf && (
                <OrbitTrail
                  trailBuffer={buf}
                  visibleCount={trailVisibleCounts?.get(satId)}
                  drawStart={trailDrawStarts?.get(satId)}
                  scaleRadius={trailScale}
                  color={color}
                  referenceFrame={referenceFrame}
                  epochJd={epochJd}
                  originPosition={lvlhActive ? originPosition : null}
                  lvlhAxes={lvlhActive ? lvlhAxes : null}
                />
              )}
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
                  bodyDefinitions={bodyDefinitions}
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
                  markerShape={resolveMarkerShape({
                    override: satelliteShapes?.get(satId),
                    simShape: satelliteSimShapes?.get(satId),
                    globalDefault: defaultMarkerShape,
                    hasAttitude: pos.qw != null,
                  })}
                />
              )}
            </group>
          );
        })}
      </SmoothOriginGroup>

      {/* Reference axes: full ECI axes for body-centered, small LVLH reference for satellite-centered */}
      {axes && <axesHelper args={[isSatCentered ? 0.015 : 2]} />}
    </>
  );
}
