import type { ChartDataMap } from "../types.js";
import { lowerBound } from "./chartViewport.js";

/**
 * Concatenate cold snapshot + hot buffer into a single ChartDataMap.
 * Returns `cold` directly when `hot` is null or empty (zero allocation).
 */
export function mergeChartData(
  cold: ChartDataMap,
  hot: ChartDataMap | null,
  derivedNames: string[],
): ChartDataMap {
  if (!hot || hot.t.length === 0) return cold;

  const allKeys = ["t", ...derivedNames];
  const merged: ChartDataMap = { t: new Float64Array(0) };

  for (const key of allKeys) {
    const coldArr = cold[key];
    const hotArr = hot[key];
    if (!coldArr && !hotArr) continue;
    if (!coldArr) {
      merged[key] = hotArr!;
      continue;
    }
    if (!hotArr) {
      merged[key] = coldArr;
      continue;
    }
    const combined = new Float64Array(coldArr.length + hotArr.length);
    combined.set(coldArr, 0);
    combined.set(hotArr, coldArr.length);
    merged[key] = combined;
  }

  return merged;
}

/**
 * Trim data points where t < tMin using binary search + subarray (O(1) zero-copy).
 * Returns the original data if no trimming is needed.
 */
export function trimChartDataLeft(
  data: ChartDataMap,
  tMin: number,
  derivedNames: string[],
): ChartDataMap {
  const tArr = data.t;
  if (tArr.length === 0) return data;

  const startIdx = lowerBound(tArr, tMin, 0, tArr.length);
  if (startIdx === 0) return data;

  const allKeys = ["t", ...derivedNames];
  const trimmed: ChartDataMap = { t: new Float64Array(0) };

  for (const key of allKeys) {
    const arr = data[key];
    if (!arr) continue;
    trimmed[key] = arr.subarray(startIdx);
  }

  return trimmed;
}
