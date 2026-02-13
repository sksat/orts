/**
 * Merge query_range response points with any streaming trail points
 * that are newer than the response. Prevents the 3D satellite position
 * from rewinding when the user zooms to an older time range.
 */
export function mergeQueryRangePoints<T extends { t: number }>(
  responsePoints: T[],
  allTrailPoints: T[],
): T[] {
  const responseMaxT = responsePoints.length > 0
    ? responsePoints[responsePoints.length - 1].t
    : -Infinity;

  const newerPoints = allTrailPoints.filter((p) => p.t > responseMaxT);
  return [...responsePoints, ...newerPoints];
}
