/**
 * Slice typed arrays to a visible time window.
 * Uses binary search (O(log n)) + Float64Array.subarray (zero-copy).
 *
 * @param arrays - Array of Float64Arrays where arrays[0] is the time column.
 * @param currentTime - Right edge of the viewport. If undefined, use all data.
 * @param timeRange - Duration of the viewport window. If null, no left-edge clipping.
 */
export function sliceArrays(
  arrays: Float64Array[] | null,
  currentTime: number | undefined,
  timeRange: number | null,
): Float64Array[] | null {
  if (!arrays) return null;
  const tArray = arrays[0];
  if (tArray.length === 0) return arrays;

  if (currentTime == null) {
    if (timeRange == null) return arrays;
    const tMax = tArray[tArray.length - 1];
    const tMin = tMax - timeRange;
    const loIdx = lowerBound(tArray, tMin, 0, tArray.length);
    return arrays.map((arr) => arr.subarray(loIdx));
  }

  // Binary search for right edge (first index with t > currentTime)
  const hiIdx = upperBound(tArray, currentTime, 0, tArray.length);

  // Left edge: apply timeRange window relative to currentTime
  let loIdx = 0;
  if (timeRange != null) {
    const tMin = currentTime - timeRange;
    loIdx = lowerBound(tArray, tMin, 0, hiIdx);
  }

  return arrays.map((arr) => arr.subarray(loIdx, hiIdx));
}

/**
 * Quantize time to 0.5s steps to reduce useMemo recalculations.
 * 60fps over 10s produces ~20 unique values instead of 600.
 */
export function quantizeChartTime(time: number | undefined): number | undefined {
  if (time == null) return undefined;
  return Math.round(time * 2) / 2;
}

/** Find first index where arr[i] >= value. */
export function lowerBound(arr: Float64Array, value: number, lo: number, hi: number): number {
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (arr[mid] < value) {
      lo = mid + 1;
    } else {
      hi = mid;
    }
  }
  return lo;
}

/** Find first index where arr[i] > value. */
export function upperBound(arr: Float64Array, value: number, lo: number, hi: number): number {
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (arr[mid] <= value) {
      lo = mid + 1;
    } else {
      hi = mid;
    }
  }
  return lo;
}
