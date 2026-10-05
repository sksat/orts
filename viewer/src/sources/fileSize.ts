/**
 * A loaded file keeps every point it holds (DESIGN.md, the file source
 * policy), so the memory it takes grows with its point count. Measured in
 * Chromium with a one-satellite CSV, the page's JS heap grew by 237 MB for
 * 300,000 points and by 582 MB for 1,000,000 (about 0.5 KB a point; the
 * DuckDB worker's memory is not counted). From this many points on, the
 * viewer says so.
 */
export const LARGE_FILE_POINTS = 1_000_000;

/** What to tell the user about a file of `points` points, or null when it is not large. */
export function largeFileWarning(points: number): string | null {
  if (points < LARGE_FILE_POINTS) return null;
  return (
    `Warning: ${points.toLocaleString("en-US")} points. The viewer keeps every point ` +
    "of a file in memory, so it may be slow or run out of memory."
  );
}
