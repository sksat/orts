/**
 * Decision about whether to downsample old data in DuckDB.
 */
export interface RetentionDecision {
  shouldDownsample: boolean;
  /** Keep every Nth row in the older half of the table. */
  keepEveryN: number;
}

/**
 * Compute whether DuckDB table needs downsampling to keep query latency stable.
 *
 * When totalRows exceeds maxRows, the older half of the data is downsampled
 * by keeping every Nth row, where N is chosen to bring the total below maxRows.
 *
 * @param totalRows - Current number of rows in the table.
 * @param maxRows - Maximum desired rows before triggering downsampling.
 */
export function computeRetention(
  totalRows: number,
  maxRows: number
): RetentionDecision {
  if (totalRows <= maxRows || maxRows < 2) {
    return { shouldDownsample: false, keepEveryN: 1 };
  }

  // We want to reduce the older half. Target: bring total to ~maxRows * 0.75
  const olderHalf = Math.floor(totalRows / 2);
  const newerHalf = totalRows - olderHalf;
  const targetOlder = Math.max(1, Math.floor(maxRows * 0.75) - newerHalf);
  const keepEveryN = Math.max(2, Math.ceil(olderHalf / targetOlder));

  return { shouldDownsample: true, keepEveryN };
}
