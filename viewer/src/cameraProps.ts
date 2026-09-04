/**
 * What a caller's camera props have to satisfy for Three.js to build a camera
 * from them, and what to use instead when they do not.
 *
 * These reach `PerspectiveCamera` directly from a public prop, and each one
 * degenerates it on its own: a `zoom` of 0, a `near` at or below zero, a `far`
 * inside the near plane, a NaN in a position, a zero `up`. The result is a blank
 * canvas that no distance fitted for it can repair.
 *
 * The rule the checks follow is to compute what the renderer computes, in the
 * same arithmetic and in the same precision: not a safer equivalent, not the
 * same formula regrouped, and not a bound chosen per input. Positive and finite
 * is repeatedly not enough, and each departure from that rule accepted
 * something: `Math.hypot` answers 1e200 for a vector whose Three.js length is
 * infinite, a half height checked alone accepts `zoom: MAX_VALUE` whose
 * projection coefficient is not, the half height regrouped accepts a `near`
 * whose matrix comes out infinite, and a coefficient that is a fine double
 * becomes `Infinity` in the float32 uniform WebGL is handed — 3.4e38 is where
 * the numbers stop, not `Number.MAX_VALUE`.
 *
 * Finite is not the whole of it either, because float32 runs out at both ends:
 * `[1e-100, 0, 0]` is a camera position whose uploaded translation is exactly
 * zero, so the GPU has the camera at the origin looking at the origin, and a
 * `zoom` of 1e-300 leaves a y coefficient that rounds to zero, so nothing
 * projects. Both matrices are perfectly finite.
 *
 * `cameraProps.test.ts` holds the rule to Three.js and to that precision:
 * whatever these functions accept has to yield a projection matrix that still
 * describes a volume as a `Float32Array`, a camera the renderer can tell the
 * position of, and a unit view direction.
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

/** Far plane; the scene keeps it beyond what it draws while the controls are on. */
export const DEFAULT_FAR = 100;

/** A usable frustum: what to build the camera with. */
export interface UsableProjection {
  fov: number;
  zoom: number;
  near: number;
  far: number;
  /**
   * Whether the far plane is the default rather than the caller's.
   *
   * A caller's depth range is theirs, including a deliberately tight one; the
   * default is only a starting value and has to keep up with where the camera
   * goes. Whoever moves the camera needs to know which of the two it is.
   */
  farIsDefault: boolean;
}

/**
 * A caller's vector if it names a direction Three.js can use, else the fallback.
 *
 * For `up`, which only has to name a direction: a zero leaves the camera's
 * orientation undefined, since it is one of the two vectors that define it, and a
 * NaN spreads through the matrices until the canvas goes blank. Its magnitude
 * does not matter, because `lookAt` normalises it.
 *
 * The length is measured as Three.js measures it, by summing the squared
 * components, rather than through the overflow-safe `Math.hypot`. Validating with
 * the safer algorithm is exactly what lets `[1e200, 0, 0]` through: `hypot`
 * answers 1e200 while `Vector3.normalize` divides by an infinite length and
 * returns the zero vector, the degeneracy this check exists to prevent. The same
 * sum fails from below for `[1e-200, 0, 0]`, where it underflows to zero and the
 * vector survives normalisation unchanged — a length no rotation can be built on.
 */
export function usableDirection(
  value: readonly number[] | undefined,
  fallback: [number, number, number],
): [number, number, number] {
  if (value?.length !== 3 || !value.every((c) => Number.isFinite(c))) return fallback;
  const [x, y, z] = value;
  const squared = x * x + y * y + z * z;
  return Number.isFinite(squared) && squared > 0 ? [x, y, z] : fallback;
}

/**
 * A caller's camera position if the renderer can place a camera there, else the
 * fallback.
 *
 * A position has to name a direction from the origin, as `up` does, *and* survive
 * the trip to the GPU: it becomes the translation in a view matrix that WebGL
 * receives as float32, where anything past 3.4e38 is `Infinity` and blanks the
 * scene. `[1e100, 0, 0]` is a perfectly good double and no place to put a camera.
 *
 * What the matrix carries is the *distance*, not the components, so that is what
 * is measured: `[3e38, 3e38, 0]` has both components inside float32 and a length
 * of 4.24e38 that is not, and the view matrix comes out infinite while a
 * per-component check passes it. The same measurement catches the other end,
 * where `[1e-100, 0, 0]` rounds to a translation of exactly zero and the renderer
 * has the camera at the origin, looking at the origin.
 */
export function usablePosition(
  value: readonly number[] | undefined,
  fallback: [number, number, number],
): [number, number, number] {
  const direction = usableDirection(value, fallback);
  if (direction === fallback) return fallback;
  const [x, y, z] = direction;
  const uploaded = Math.fround(Math.hypot(x, y, z));
  return Number.isFinite(uploaded) && uploaded > 0 ? direction : fallback;
}

/**
 * The caller's projection settings, or the defaults where they name no frustum.
 *
 * `sceneExtent` is the radius of what the view draws around the origin, which is
 * the scale a near plane has to be judged against: from around 1e17 of these
 * units the extent rounds away against the plane — `near + extent` is `near`
 * again — so there is no room between the planes for the scene to be drawn in,
 * however far back the camera stands.
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
  // comes out infinite.
  //
  // `aspect` belongs to a canvas that has no size yet, so this is the square
  // viewport the default framing assumes. The horizontal coefficient divides by
  // it, so a portrait canvas scales that term by 1/aspect and could overflow one
  // whose square-viewport counterpart is finite. Reaching that takes a zoom
  // already within a few orders of the float32 ceiling *and* a viewport narrow
  // enough to multiply it past: at the default framing's coefficient of 2.14, an
  // aspect below 1e-38.
  const halfHeight = (near * Math.tan((fov / 2) * (Math.PI / 180))) / zoom;
  const height = 2 * halfHeight;
  const projectionScale = (2 * near) / height;
  // And the two depth coefficients, in their order for the same reason: `-2 *
  // far` overflows on its own for a `far` of MAX_VALUE, so a depth range that
  // subtracts to a finite number still produces an infinite matrix entry.
  const range = depth - near;
  const depthScale = -(depth + near) / range;
  const depthOffset = (-2 * depth * near) / range;
  // Every coefficient is checked in the precision it is used in, and at both ends
  // of it. Three.js keeps the matrix as doubles and WebGL is handed a float32
  // copy, so a coefficient of 2.1e200 — which `zoom: 1e200` produces — is a fine
  // double and an infinite uniform, while 2.1e-300, from a `zoom` of 1e-300,
  // rounds to zero and flattens the frustum. Either way the canvas is blank with
  // every double-precision check passing.
  const uploadable = (value: number) => {
    const uploaded = Math.fround(value);
    return Number.isFinite(uploaded) && uploaded !== 0;
  };
  const representable =
    Number.isFinite(halfHeight) &&
    halfHeight > 0 &&
    projectionScale > 0 &&
    uploadable(projectionScale) &&
    uploadable(depthScale) &&
    uploadable(depthOffset) &&
    near + sceneExtent - near > 0;
  return representable
    ? { fov, zoom, near, far: depth, farIsDefault: depth !== camera?.far }
    : { fov, zoom: 1, near: DEFAULT_NEAR, far: DEFAULT_FAR, farIsDefault: true };
}
