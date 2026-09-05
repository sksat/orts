/**
 * Resolve a {@link ReferenceFrame} into everything the scene needs to render in
 * it: the frame centre's state, the LVLH axes (when the data should be
 * expressed in the local-orbital frame), and what the camera should do.
 *
 * This is the single place that decides the frame semantics (#90):
 *
 * - `central_body` centre — origin at the body; `body_fixed` rotation is
 *   applied by the primitives themselves (ECEF transforms), not here.
 * - satellite centre + `local_orbital` — the classic LVLH view: positions,
 *   trails and attitudes are transformed into the orbit frame (`lvlhActive`).
 *   When that's not possible (no velocity) or the centred entity is a celestial
 *   body (which keeps its IAU orientation in inertial axes), the LVLH feel is
 *   approximated by co-rotating the *camera* instead (`cameraTracking`).
 * - satellite centre + `inertial` — the centred entity sits at the origin but
 *   the axes stay star-fixed: data is offset only, and the camera must not
 *   co-rotate with the orbit.
 *
 * Pure and renderer-agnostic so the semantics are unit-testable; both the app
 * scene and the embeddable viewer resolve frames through this kernel.
 */

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
 * camera's up vector at NaN, which blanks the canvas. Everything finite is
 * usable, zero included — the centred spacecraft is drawn at the origin whatever
 * its coordinates, and only the directions that need a *bearing* from it drop
 * out. The UI asks this so a control cannot be disabled over a scene that draws.
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
