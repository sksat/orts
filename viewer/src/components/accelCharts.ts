/** Acceleration chart metadata and the rule for when each one is shown.
 *
 * Kept apart from the component so the rule can be tested on its own.
 */

export const ACCEL_CHART_DEFS: {
  metric: string;
  title: string;
  color: string;
  pertKey?: string;
}[] = [
  { metric: "accel_gravity", title: "Gravity", color: "#aaa" },
  { metric: "accel_drag", title: "Drag", color: "#f80", pertKey: "drag" },
  { metric: "accel_srp", title: "SRP", color: "#ff0", pertKey: "srp" },
  {
    metric: "accel_third_body_sun",
    title: "Sun 3rd-body",
    color: "#fa0",
    pertKey: "third_body_sun",
  },
  {
    metric: "accel_third_body_moon",
    title: "Moon 3rd-body",
    color: "#8af",
    pertKey: "third_body_moon",
  },
  {
    metric: "accel_perturbation_total",
    title: "Total Perturbation",
    color: "#f44",
    pertKey: "_any",
  },
];

/** Whether an acceleration chart should be shown for the active perturbations.
 *
 * A panel model computes the same physical force as its isotropic counterpart
 * and reports under that force's name, so `panel_srp` fills the `accel_srp`
 * column and has to satisfy the SRP chart's condition. See `force_channel` in
 * cli/src/sim/core.rs, which does the renaming on the way out.
 */
export function isAccelChartActive(
  pertKey: string | undefined,
  activePerturbations: string[] | undefined,
): boolean {
  if (!activePerturbations || activePerturbations.length === 0) return false;
  if (!pertKey) return true; // gravity: always shown once anything is active
  if (pertKey === "_any") return true; // total: any perturbation will do
  return activePerturbations.some((p) => p === pertKey || p === `panel_${pertKey}`);
}
