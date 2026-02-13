/** A single named time series with independent time array. */
export interface NamedTimeSeries {
  label: string;
  t: Float64Array;
  values: Float64Array;
}

/** Result of alignment: shared time array + per-series values (NaN for gaps). */
export interface AlignedMultiSeries {
  /** Merged, sorted, deduplicated time array. */
  t: Float64Array;
  /** One array per input series, same length as t. Missing values are NaN. */
  values: Float64Array[];
  /** Labels in the same order as values[]. */
  labels: string[];
}

/**
 * Merge independent time series into a shared time axis.
 *
 * Each input series may have a different time array. The output has a single
 * merged time array with all unique time values (sorted), and each series
 * is filled with NaN where it has no data at a given time point.
 */
export function alignTimeSeries(inputs: NamedTimeSeries[]): AlignedMultiSeries {
  if (inputs.length === 0) {
    return { t: new Float64Array(0), values: [], labels: [] };
  }

  // 1. Collect all unique time values into a sorted set.
  const timeSet = new Set<number>();
  for (const input of inputs) {
    for (let i = 0; i < input.t.length; i++) {
      timeSet.add(input.t[i]);
    }
  }

  const sortedTimes = Float64Array.from(timeSet).sort();

  // 2. Build a time→index lookup for fast positioning.
  const timeIndex = new Map<number, number>();
  for (let i = 0; i < sortedTimes.length; i++) {
    timeIndex.set(sortedTimes[i], i);
  }

  // 3. For each series, fill the aligned array.
  const labels: string[] = [];
  const values: Float64Array[] = [];

  for (const input of inputs) {
    labels.push(input.label);
    const arr = new Float64Array(sortedTimes.length).fill(NaN);

    for (let i = 0; i < input.t.length; i++) {
      const idx = timeIndex.get(input.t[i]);
      if (idx !== undefined) {
        arr[idx] = input.values[i];
      }
    }

    values.push(arr);
  }

  return { t: sortedTimes, values, labels };
}
