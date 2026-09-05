import type { ReferenceFrame } from "./referenceFrame.js";
import { computeLvlhAxes, type LvlhAxes } from "./sceneFrame.js";

/** Current state of a centred entity, in the central-body inertial frame [km, km/s]. */
export interface FrameEntityState {
  position: [number, number, number];
  velocity: [number, number, number] | null;
}

/** Look up an entity's current state by id; null when unknown/not yet known. */
export type FrameEntityLookup = (id: string) => FrameEntityState | null;

/** Everything the scene graph needs to render in the resolved frame. */
export interface SceneFrameContext {
  /** Centred satellite/entity id, or null for a central-body centre. */
  centeredSatId: string | null;
  /** ECI position [km] of the frame centre, or null for the central body. */
  originPosition: [number, number, number] | null;
  /** ECI velocity [km/s] of the frame centre, when known. */
  originVelocity: [number, number, number] | null;
  /** LVLH axes when `lvlhActive`; null otherwise. */
  lvlhAxes: LvlhAxes | null;
  /** True when positions/trails/attitudes get the LVLH (data) transform. */
  lvlhActive: boolean;
  /** True when the camera should co-rotate / radial-track the centre instead. */
  cameraTracking: boolean;
}

/**
 * Whether a spacecraft's position can centre the scene on it.
 *
 * Two ways to fail, and both blank the canvas rather than draw something wrong. A
 * non-finite component puts the origin offset and the camera's up vector at NaN.
 * And the offset reaches the renderer as float32, where anything past 3.4e38 is
 * `Infinity`: `[1e100, 0, 0]` is a perfectly good double and no place to put a
 * spacecraft. What the matrix carries is the distance rather than the components,
 * so that is what is measured — `[3e38, 3e38, 0]` has both components inside
 * float32 and a length of 4.24e38 that is not, and a per-component check passes
 * it. The same test guards the camera in `usablePosition`.
 *
 * Zero is usable, which is where this parts company with the camera's version: a
 * spacecraft at the body's centre is drawn at the origin like any other centre,
 * and only the directions needing a *bearing* from it drop out. The UI asks this
 * so a control cannot be disabled over a scene that draws.
 */
export function centrePositionIsUsable(
  position: readonly number[] | null | undefined,
): position is readonly number[] {
  if (position == null || position.length !== 3) return false;
  const [x, y, z] = position;
  if (!(Number.isFinite(x) && Number.isFinite(y) && Number.isFinite(z))) return false;
  return Number.isFinite(Math.fround(Math.hypot(x, y, z)));
}

export function resolveSceneFrame(
  frame: ReferenceFrame,
  getEntity: FrameEntityLookup,
  isBodyEntity: (id: string) => boolean,
): SceneFrameContext {
  const inert: SceneFrameContext = {
    centeredSatId: null,
    originPosition: null,
    originVelocity: null,
    lvlhAxes: null,
    lvlhActive: false,
    cameraTracking: false,
  };

  if (frame.center.type !== "satellite") return inert;

  const id = frame.center.id;
  const state = getEntity(id);
  // Treated like a state that has not arrived — the entity is still the centre,
  // so the camera stays put until a usable sample lands.
  if (state == null || !centrePositionIsUsable(state.position)) {
    return { ...inert, centeredSatId: id };
  }

  // Snapshot the caller-owned tuples: the context is returned from public API
  // surfaces, and an embedder mutating its position array in place must not
  // retroactively change an already-resolved frame.
  const originPosition: [number, number, number] = [...state.position];
  const originVelocity: [number, number, number] | null = state.velocity
    ? [...state.velocity]
    : null;
  const localOrbital = frame.orientation === "local_orbital";

  // Data-LVLH: only for a real satellite (bodies keep their IAU orientation in
  // inertial axes) and only when the axes are computable from pos/vel.
  const lvlhAxes =
    localOrbital && !isBodyEntity(id) ? computeLvlhAxes(originPosition, originVelocity) : null;
  const lvlhActive = lvlhAxes != null;

  return {
    centeredSatId: id,
    originPosition,
    originVelocity,
    lvlhAxes,
    lvlhActive,
    // LVLH requested but not expressible in the data → approximate with the
    // camera. An inertial centre never tracks: the axes stay star-fixed.
    cameraTracking: localOrbital && !lvlhActive,
  };
}
