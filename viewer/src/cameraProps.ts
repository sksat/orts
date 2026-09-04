/**
 * What a caller's camera props have to satisfy for Three.js to build a camera
 * from them, and what to use instead when they do not.
 *
 * These reach `PerspectiveCamera` directly from a public prop, and each one
 * degenerates it on its own: a `zoom` of 0, a `near` at or below zero, a `far`
 * inside the near plane, a NaN in a position, a zero `up`. The result is a blank
 * canvas that no distance fitted for it can repair.
 *
 * The rule the checks follow is to compute what Three.js computes, in the same
 * arithmetic: not a safer equivalent, not the same formula regrouped, and not a
 * bound chosen per input. Positive and finite is repeatedly not enough, and each
 * departure from that rule accepted something: `Math.hypot` answers 1e200 for a
 * vector whose Three.js length is infinite, a half height checked alone accepts
 * `zoom: MAX_VALUE` whose projection coefficient is not, and the half height
 * regrouped accepts a `near` whose matrix comes out infinite.
 * `cameraProps.test.ts` holds the rule to Three.js itself — whatever these
 * functions accept has to yield an all-finite projection matrix and a unit view
 * direction, which is how the third of those was found.
 */

/** The camera settings an embedder may pass, as much of them as is checked here. */
export interface CameraPropsInput {
  fov?: number;
  zoom?: number;
  near?: number;
  far?: number;
  position?: readonly number[];
  up?: readonly number[];
}

/** Near plane the default framing uses, in scene units. */
export const DEFAULT_NEAR = 0.01;

/** Far plane; the initial fit pushes it out when it moves the camera. */
export const DEFAULT_FAR = 100;

/** A usable frustum: what to build the camera with. */
export interface UsableProjection {
  fov: number;
  zoom: number;
  near: number;
  far: number;
}

/**
 * A caller's vector if it names a direction Three.js can use, else the fallback.
 *
 * A NaN in a camera position spreads through its matrices and the canvas goes
 * blank; a zero `up` leaves the orientation undefined, since it is one of the two
 * vectors that define it.
 *
 * The length is measured as Three.js measures it, by summing the squared
 * components, rather than through the overflow-safe `Math.hypot`. Validating with
 * the safer algorithm is exactly what lets `[1e200, 0, 0]` through: `hypot`
 * answers 1e200 while `Vector3.normalize` divides by an infinite length and
 * returns the zero vector, the degeneracy this check exists to prevent. The same
 * sum fails from below for `[1e-200, 0, 0]`, where it underflows to zero and the
 * vector survives normalisation unchanged — a length no rotation can be built on.
 */
export function usableVector(
  value: readonly number[] | undefined,
  fallback: [number, number, number],
): [number, number, number] {
  if (value?.length !== 3 || !value.every((c) => Number.isFinite(c))) return fallback;
  const [x, y, z] = value;
  const squared = x * x + y * y + z * z;
  return Number.isFinite(squared) && squared > 0 ? [x, y, z] : fallback;
}

/**
 * The caller's projection settings, or the defaults where they name no frustum.
 *
 * `sceneExtent` is the radius of what the view draws around the origin, which is
 * the scale a near plane has to be judged against: from around 1e17 of these
 * units the extent rounds away against the plane — `near + extent` is `near`
 * again — so no camera distance leaves the scene between the planes, and a
 * position fitted for that plane squares to infinity when its length is taken,
 * which sets the far plane to infinity.
 */
export function usableProjection(
  camera: CameraPropsInput | undefined,
  fov: number,
  sceneExtent: number,
): UsableProjection {
  const positive = (value: number | undefined, fallback: number) =>
    value != null && Number.isFinite(value) && value > 0 ? value : fallback;
  const zoom = positive(camera?.zoom, 1);
  const near = positive(camera?.near, DEFAULT_NEAR);
  const far = positive(camera?.far, DEFAULT_FAR);
  // A far plane inside the near one has no volume between them to draw.
  const depth = far > near ? far : Math.max(DEFAULT_FAR, near * 2);
  // The frustum Three.js will build, in the arithmetic and the order it builds it
  // in: the half height at the near plane, its doubling, and the coefficient that
  // scales y in the projection matrix. Writing the same formula differently is
  // not the same check — grouping the half height as `(tan / zoom) * near` gives
  // 2.3e-24 for `near: MIN_VALUE, zoom: 1e-300` where Three.js's own order
  // underflows to 0, so the reassociated version accepts a camera whose matrix
  // comes out infinite. `aspect` is the canvas's and is not known yet; it scales
  // the width term the same way, and a square viewport is what the default
  // framing assumes.
  const halfHeight = (near * Math.tan((fov / 2) * (Math.PI / 180))) / zoom;
  const height = 2 * halfHeight;
  const projectionScale = (2 * near) / height;
  // And the two depth coefficients, in their order for the same reason: `-2 *
  // far` overflows on its own for a `far` of MAX_VALUE, so a depth range that
  // subtracts to a finite number still produces an infinite matrix entry.
  const range = depth - near;
  const depthScale = -(depth + near) / range;
  const depthOffset = (-2 * depth * near) / range;
  const representable =
    Number.isFinite(halfHeight) &&
    halfHeight > 0 &&
    Number.isFinite(projectionScale) &&
    projectionScale > 0 &&
    Number.isFinite(depthScale) &&
    Number.isFinite(depthOffset) &&
    near + sceneExtent - near > 0;
  return representable
    ? { fov, zoom, near, far: depth }
    : { fov, zoom: 1, near: DEFAULT_NEAR, far: DEFAULT_FAR };
}
