import { alignTimeSeries, type NamedTimeSeries } from "@sksat/uneri/align";

/** Generic chart data: keyed by column name. Compatible with uneri ChartDataMap. */
interface ChartDataMap {
  t: Float64Array;
  [derivedName: string]: Float64Array;
}

/** Configuration for a single series. Compatible with uneri SeriesConfig. */
interface SeriesConfig {
  label: string;
  color: string;
}

/** Multi-series data. Compatible with uneri MultiSeriesData. */
export interface MultiSeriesData {
  t: Float64Array;
  values: Float64Array[];
  series: SeriesConfig[];
}

/** Configuration for one satellite in the multi-store. */
export interface SatelliteConfig {
  id: string;
  label: string;
  color: string;
}

/** Map from metric name to MultiSeriesData (aligned across satellites). */
export type MultiChartDataMap = {
  [metricName: string]: MultiSeriesData | null;
};

/**
 * Build MultiChartDataMap from per-satellite ChartDataMap results.
 *
 * Pure function: takes the query results from each satellite's DuckDB table,
 * aligns them on a shared time axis, and produces MultiSeriesData per metric.
 */
export function buildMultiChartData(
  perSatelliteData: Map<string, ChartDataMap>,
  metricNames: string[],
  satelliteConfigs: SatelliteConfig[],
): MultiChartDataMap | null {
  // Filter to configs that have data
  const activeSats = satelliteConfigs.filter((cfg) => perSatelliteData.has(cfg.id));

  if (activeSats.length === 0) return null;

  const result: MultiChartDataMap = {};

  for (const metric of metricNames) {
    // Build per-satellite NamedTimeSeries for this metric
    const inputs: NamedTimeSeries[] = [];
    const seriesConfigs: SeriesConfig[] = [];

    for (const sat of activeSats) {
      const data = perSatelliteData.get(sat.id);
      if (!data || !data[metric]) continue;

      inputs.push({
        label: sat.label,
        t: data.t,
        values: data[metric],
      });
      seriesConfigs.push({ label: sat.label, color: sat.color });
    }

    if (inputs.length === 0) {
      result[metric] = null;
      continue;
    }

    const aligned = alignTimeSeries(inputs);

    result[metric] = {
      t: aligned.t,
      values: aligned.values,
      series: seriesConfigs,
    };
  }

  return result;
}
