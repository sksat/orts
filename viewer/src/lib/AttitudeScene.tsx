import { useMemo } from "react";
import { AttitudeSceneContents } from "../components/AttitudeSceneContents.js";
import { useSunLighting } from "../components/SunLighting.js";
import { type DirectionVectorOptions, resolveDirectionVectors } from "../directionVectors.js";
import {
  type DisplayOrientation,
  displayQuaternion,
  type Quat,
  resolveDisplayOrientation,
  type Vec3,
} from "../displayFrame.js";
import { computeLvlhAxes } from "../sceneFrame.js";
import { finiteOrNull } from "../utils/finite.js";
import { earth_rotation_angle } from "../wasm/arikaInit.js";
import { useArikaReady } from "../wasm/useArikaReady.js";
import type { AttitudeSceneProps } from "./types.js";

/**
 * The one central body whose rotation the viewer models: `earth_rotation_angle`
 * is Earth's. Other bodies rotate too, but by a different angle, so offering a
 * body-fixed frame for them would be a wrong picture rather than a missing one.
 */
const BODY_FIXED_BODY_ID = "earth";

/** Sun direction is recomputed at this granularity [s]; it moves ~0.04°/min. */
const SUN_TIME_QUANTUM = 60;

/**
 * The attitude scene graph, for mounting inside your own @react-three/fiber
 * `<Canvas>`.
 *
 * Shows one spacecraft's orientation and nothing else: the spacecraft sits at the
 * origin with its body axes, the reference frame's axes are drawn around it, and
 * the reference directions (Sun, nadir) are drawn as arrows. There is no central
 * body, no trail and no physical scale — the spacecraft is normalised to one
 * scene unit across, so the camera framing does not depend on the real size.
 *
 * Sibling of {@link OrbitScene} rather than a mode of it: the two need different
 * data (an attitude here, a position there), different spatial scales and
 * different cameras. What they share are the drawing primitives and the display
 * frame, so a spacecraft and its arrows look and point the same in both.
 *
 * `orientation` requests a display frame; a request whose inputs are absent falls
 * back to inertial. An arrow whose input is absent is not drawn — without an
 * epoch there is no Sun direction, and a fixed one would read as a measurement.
 *
 * @example
 * ```tsx
 * <Canvas camera={{ position: [3, 0, 1.5], up: SCENE_UP }}>
 *   <AttitudeScene
 *     centralBody={{ id: "earth" }}
 *     body={{ id: "sat-1", attitude: [1, 0, 0, 0] }}
 *   />
 * </Canvas>
 * ```
 */
export function AttitudeScene({
  centralBody,
  body,
  orientation = "inertial",
  epochJd,
  time = 0,
  defaultMarkerShape,
  directionVectors,
  controls = true,
  axes = true,
}: AttitudeSceneProps) {
  // A non-finite epoch or time is treated as absent, at the boundary: it would
  // otherwise reach `earth_rotation_angle`, the quantised Sun time and the
  // light's position, and the fallbacks below exist precisely for the case of
  // not knowing the epoch.
  const epoch = finiteOrNull(epochJd);

  // Only load the arika WASM when an epoch is supplied (Sun direction / rotation).
  const arikaReady = useArikaReady(epoch != null);
  const effectiveEpochJd = arikaReady ? epoch : null;

  const t = finiteOrNull(body.time ?? time) ?? 0;

  // The caller's tuples are copied here, and everything downstream keys its
  // memoisation on these copies. Keying on the caller's array would key on its
  // *identity*: an embedder feeding a high-rate stream may reuse one array and
  // write the new sample into it — this framework hands out a mutable
  // `TrailBuffer` for exactly that shape of feed — and what is drawn would then
  // stay frozen at the first sample. A wrong picture, drawn confidently.
  //
  // The attitude needs this most. In the inertial frame `displayQuaternion`
  // returns the caller's tuple unchanged, and `BodyAxes`, `SatelliteModel` and
  // `PrimitiveMarker` all apply a quaternion from an effect keyed on its
  // identity — so a mutated attitude would leave the spacecraft's orientation,
  // the subject of this whole view, standing still.
  const [qw, qx, qy, qz] = body.attitude ?? [];
  const attitude = useMemo<Quat | undefined>(
    () => (qw != null && qx != null && qy != null && qz != null ? [qw, qx, qy, qz] : undefined),
    [qw, qx, qy, qz],
  );

  const [px, py, pz] = body.position ?? [];
  const [vx, vy, vz] = body.velocity ?? [];
  const position = useMemo<Vec3 | null>(
    () => (px != null && py != null && pz != null ? [px, py, pz] : null),
    [px, py, pz],
  );
  const velocity = useMemo<Vec3 | null>(
    () => (vx != null && vy != null && vz != null ? [vx, vy, vz] : null),
    [vx, vy, vz],
  );

  // Same reasoning for the options object.
  const drawSun = directionVectors?.sun;
  const drawNadir = directionVectors?.nadir;
  const vectorOptions = useMemo<DirectionVectorOptions | undefined>(
    () =>
      drawSun === undefined && drawNadir === undefined
        ? undefined
        : { sun: drawSun, nadir: drawNadir },
    [drawSun, drawNadir],
  );

  // The requested orientation, gated on what this central body supports.
  const requested: DisplayOrientation =
    orientation === "bodyFixed" && centralBody.id !== BODY_FIXED_BODY_ID ? "inertial" : orientation;

  const era = useMemo(() => {
    if (requested !== "bodyFixed" || effectiveEpochJd == null) return null;
    return earth_rotation_angle(effectiveEpochJd, t);
  }, [requested, effectiveEpochJd, t]);

  const lvlhAxes = useMemo(
    () => (requested === "localOrbital" ? computeLvlhAxes(position, velocity) : null),
    [requested, position, velocity],
  );

  const frame = useMemo(
    () => resolveDisplayOrientation(requested, { era, originPosition: position, lvlhAxes }),
    [requested, era, position, lvlhAxes],
  );

  // The Sun in this display frame, from the same hook that lights the orbit
  // scene: one computation feeds both the arrow and the light, so they agree.
  const { sunDirection, sunDirectionEci } = useSunLighting({
    centralBody: centralBody.id,
    epochJd: effectiveEpochJd,
    quantizedSimTime: Math.floor(t / SUN_TIME_QUANTUM) * SUN_TIME_QUANTUM,
    displayFrame: frame,
    sceneAmplification: 1,
  });

  const vectors = useMemo(
    () =>
      resolveDirectionVectors({
        frame,
        // Without an epoch the hook falls back to a fixed direction, which is
        // fine for lighting and wrong for an arrow claiming where the Sun is.
        sunEci: effectiveEpochJd != null ? sunDirectionEci : null,
        positionEci: position,
        options: vectorOptions,
      }),
    [frame, effectiveEpochJd, sunDirectionEci, position, vectorOptions],
  );

  const displayQuat = useMemo(() => displayQuaternion(frame, attitude), [frame, attitude]);

  // The light always gets a direction (a fixed one with no epoch, so a 3D model
  // is not left black); it is the *arrow* that is dropped when the Sun is unknown.
  const sunSceneDirection: Vec3 = [sunDirection.x, sunDirection.y, sunDirection.z];

  return (
    <AttitudeSceneContents
      quaternion={displayQuat}
      satId={body.id}
      satName={body.name ?? null}
      color={body.color}
      markerShape={body.markerShape}
      defaultMarkerShape={defaultMarkerShape}
      vectors={vectors}
      sunDirection={sunSceneDirection}
      controls={controls}
      axes={axes}
    />
  );
}
