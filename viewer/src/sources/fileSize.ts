/**
 * A loaded file keeps every point it holds (DESIGN.md, the file source
 * policy), so the memory it takes grows with its point count. Measured in
 * Chromium with a one-satellite CSV, the page's JS heap grew by 237 MB for
 * 300,000 points and by 582 MB for 1,000,000 (about 0.5 KB a point; the
 * DuckDB worker's memory is not counted). From this many points on, the
 * viewer says so.
 */
export const LARGE_FILE_POINTS = 1_000_000;

/**
 * The threshold in force: {@link LARGE_FILE_POINTS}, or in a dev build the
 * number an E2E test put on `window.__debug_large_file_points`, so a small
 * file can stand in for a large one.
 */
function largeFilePoints(): number {
  if (import.meta.env.DEV && typeof window !== "undefined") {
    const override = (window as unknown as Record<string, unknown>).__debug_large_file_points;
    if (typeof override === "number") return override;
  }
  return LARGE_FILE_POINTS;
}

/** What to tell the user about a file of `points` points, or null when it is not large. */
export function largeFileWarning(points: number, threshold = largeFilePoints()): string | null {
  if (points < threshold) return null;
  return (
    `Warning: ${points.toLocaleString("en-US")} points. The viewer keeps every point ` +
    "of a file in memory, so it may be slow or run out of memory."
  );
}
