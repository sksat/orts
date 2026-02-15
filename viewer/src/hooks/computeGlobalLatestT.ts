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
