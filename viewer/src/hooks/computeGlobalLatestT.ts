/**
 * Compute the maximum latestT across all ingest buffers.
 *
 * Used by multi-satellite chart queries to ensure all satellites are queried
 * with the same time window (anchored to the global latest time), rather than
 * per-satellite latestT which causes terminated satellites to have a stale
 * time window.
 */
export function computeGlobalLatestT(
  buffers: Map<string, { latestT: number }>,
): number {
  let max = -Infinity;
  for (const buf of buffers.values()) {
    if (buf.latestT > max) max = buf.latestT;
  }
  return max;
}

/** Time range for chart display: null = all history, number = last N seconds. */
type TimeRange = number | null;

/**
 * Compute a unified tMin for multi-satellite DuckDB queries.
 *
 * Returns `undefined` (= no WHERE clause, show all) when:
 * - timeRange is null ("All" mode)
 * - no buffers have valid data (globalLatest is -Infinity)
 *
 * Otherwise returns `globalLatest - timeRange` so all satellites
 * share the same time window.
 */
export function computeUnifiedTMin(
  timeRange: TimeRange,
  buffers: Map<string, { latestT: number }>,
): number | undefined {
  if (timeRange == null) return undefined;
  const globalLatest = computeGlobalLatestT(buffers);
  if (!isFinite(globalLatest)) return undefined;
  return globalLatest - timeRange;
}
