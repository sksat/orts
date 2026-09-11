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
 * A non-finite component is no position: it would put the origin offset and the
 * camera's up vector at NaN. Everything finite is accepted, zero included — a
 * spacecraft at the body's centre is drawn at the origin like any other centre,
 * and only the directions needing a *bearing* from it drop out. The UI asks this
 * so a control cannot be disabled over a scene that draws.
 *
 * Answering false is how the NaN is avoided rather than a report of it:
 * `resolveSceneFrame` then keeps the entity as the centre with no origin, which
 * is how it holds a state that has not arrived, and the camera stays where it is
 * until a usable sample lands.
 *
 * The magnitude a float32 uniform can hold is deliberately not judged here.
 * Positions reach the renderer divided by the scene's scale radius — itself the
 * central body's radius over the amplification — so the limit lives in units this
 * function is not given, and testing the kilometres would reject positions the
 * renderer draws perfectly well. See #451, where the scale is in scope.
 */
export function centrePositionIsUsable(
  position: readonly number[] | null | undefined,
): position is readonly number[] {
  return position != null && position.length === 3 && position.every(Number.isFinite);
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
