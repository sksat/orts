/**
 * Merge query_range response points with any streaming trail points
 * that are newer than the response. Prevents the 3D satellite position
 * from rewinding when the user zooms to an older time range.
 */
export function mergeQueryRangePoints<T extends { t: number }>(
  responsePoints: T[],
  allTrailPoints: T[],
): T[] {
  const responseMaxT =
    responsePoints.length > 0 ? responsePoints[responsePoints.length - 1].t : -Infinity;

  const newerPoints = allTrailPoints.filter((p) => p.t > responseMaxT);
  return [...responsePoints, ...newerPoints];
}

/**
 * Resolve which trail buffer to merge a `query_range` response against.
 *
 * The correct target is the satellite the response is about — determined
 * from the first point's `entityPath`. Using a hard-coded fallback (e.g.
 * `simInfo.satellites[0].id`) would cause cross-entity contamination:
 * a sat B response merged with sat A's streaming tail, then dispatched
 * as a rebuild of sat B's trail buffer, leaves sat B's orbit polluted
 * with sat A's position points.
 *
 * Returns `null` when the response has no points (nothing to merge) or
 * when the matching trail buffer is absent from the map.
 */
export function pickTrailBufferForResponse<B>(
  responsePoints: { entityPath?: string }[],
  trailBuffers: Map<string, B>,
  fallbackSatId: string | null,
): B | null {
  const targetSatId = responsePoints[0]?.entityPath ?? fallbackSatId;
  if (targetSatId == null) return null;
  return trailBuffers.get(targetSatId) ?? null;
}
