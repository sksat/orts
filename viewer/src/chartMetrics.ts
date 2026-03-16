/** Base chart metric names (always shown). */
export const BASE_CHART_METRICS = [
  "altitude",
  "energy",
  "angular_momentum",
  "velocity",
  "a",
  "e",
  "inc_deg",
  "raan_deg",
];

/** Acceleration chart metric names (shown when perturbations active). */
export const ACCEL_CHART_METRICS = [
  "accel_gravity",
  "accel_drag",
  "accel_srp",
  "accel_third_body_sun",
  "accel_third_body_moon",
  "accel_perturbation_total",
];

/**
 * All derived metric names for multi-satellite alignment.
 * Passed to useMultiSatelliteStore so that buildMultiChartData
 * produces data for every chart metric.
 */
export const METRIC_NAMES = [...BASE_CHART_METRICS, ...ACCEL_CHART_METRICS];
