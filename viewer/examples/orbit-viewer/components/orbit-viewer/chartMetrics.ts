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

/** The models whose torque the viewer charts, in the order the charts appear. */
export const TORQUE_CHART_MODELS = ["gravity_gradient", "panel_srp", "panel_drag"] as const;

/** One of the models the viewer charts a torque for. */
export type TorqueChartModel = (typeof TORQUE_CHART_MODELS)[number];

/** The three axes of a body-frame torque, in the order they are plotted. */
export const TORQUE_AXES = ["x", "y", "z"];

/** Torque chart metric names: one per model and axis. */
export const TORQUE_CHART_METRICS = TORQUE_CHART_MODELS.flatMap((model) =>
  TORQUE_AXES.map((axis) => `torque_${model}_${axis}`),
);

/**
 * All derived metric names for multi-satellite alignment.
 * Passed to useMultiSatelliteStore so that buildMultiChartData
 * produces data for every chart metric.
 */
export const METRIC_NAMES = [
  ...BASE_CHART_METRICS,
  ...ACCEL_CHART_METRICS,
  ...TORQUE_CHART_METRICS,
];
