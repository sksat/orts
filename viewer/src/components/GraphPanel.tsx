import { memo, useMemo, useState } from "react";
import { type ChartDataMap, type TimeRange, TimeSeriesChart } from "uneri";
import type { MultiChartDataMap } from "../hooks/buildMultiChartData.js";
import styles from "./GraphPanel.module.css";

const TIME_RANGE_OPTIONS: { label: string; value: TimeRange }[] = [
  { label: "All", value: null },
  { label: "5 min", value: 300 },
  { label: "30 min", value: 1800 },
  { label: "1 h", value: 3600 },
];

/** Chart definitions: metric name → display config. */
const CHART_DEFS: { metric: string; title: string; yLabel: string; color: string }[] = [
  { metric: "altitude", title: "Altitude", yLabel: "km", color: "#4af" },
  { metric: "energy", title: "Specific Orbital Energy", yLabel: "km\u00B2/s\u00B2", color: "#f84" },
  { metric: "angular_momentum", title: "Angular Momentum", yLabel: "km\u00B2/s", color: "#8f4" },
  { metric: "velocity", title: "Velocity", yLabel: "km/s", color: "#f4f" },
  { metric: "a", title: "Semi-major Axis", yLabel: "km", color: "#4ff" },
  { metric: "e", title: "Eccentricity", yLabel: "-", color: "#ff4" },
  { metric: "inc_deg", title: "Inclination", yLabel: "deg", color: "#f48" },
  { metric: "raan_deg", title: "RAAN", yLabel: "deg", color: "#84f" },
];

/** Acceleration chart definitions. Shown conditionally based on active perturbations. */
const ACCEL_CHART_DEFS: { metric: string; title: string; color: string; pertKey?: string }[] = [
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

interface GraphPanelProps {
  /** Single-satellite chart data (replay mode / single sat). */
  chartData?: ChartDataMap | null;
  /** Multi-satellite chart data (comparison mode). */
  multiChartData?: MultiChartDataMap | null;
  isLoading: boolean;
  timeRange: TimeRange;
  onTimeRangeChange: (range: TimeRange) => void;
  /** Called when the user drag-zooms into a time range on any chart. */
  onZoom?: (tMin: number, tMax: number) => void;
  /** Active perturbation names from SimInfo (union across all satellites). */
  activePerturbations?: string[];
}

export const GraphPanel = memo(function GraphPanel({
  chartData,
  multiChartData,
  isLoading,
  timeRange,
  onTimeRangeChange,
  onZoom,
  activePerturbations,
}: GraphPanelProps) {
  const [collapsed, setCollapsed] = useState(false);

  // Filter acceleration charts: show gravity always + perturbations that are active
  const visibleAccelDefs = useMemo(() => {
    if (!activePerturbations || activePerturbations.length === 0) return [];
    return ACCEL_CHART_DEFS.filter((def) => {
      if (!def.pertKey) return true; // gravity: always show
      if (def.pertKey === "_any") return true; // total: show when any perturbation is active
      return activePerturbations.includes(def.pertKey);
    });
  }, [activePerturbations]);

  const allDefs = useMemo(
    () => [...CHART_DEFS, ...visibleAccelDefs.map((d) => ({ ...d, yLabel: "km/s\u00B2" }))],
    [visibleAccelDefs],
  );

  // Single-series data extraction (for backward compat / single satellite)
  const singleSeriesData = useMemo(() => {
    if (!chartData) return null;
    const result: Record<string, [Float64Array, Float64Array] | null> = {};
    for (const def of allDefs) {
      result[def.metric] = chartData[def.metric]
        ? ([chartData.t, chartData[def.metric]] as [Float64Array, Float64Array])
        : null;
    }
    return result;
  }, [chartData, allDefs]);

  return (
    <div className={`${styles.graphPanel} ${collapsed ? styles.collapsed : ""}`}>
      <button className={styles.toggle} onClick={() => setCollapsed((c) => !c)}>
        {collapsed ? "\u25C0 Graphs" : "\u25B6"}
      </button>
      {!collapsed && (
        <div className={styles.content}>
          {isLoading && <div className={styles.loading}>Loading DuckDB...</div>}
          <div className={styles.timeRangeSelector}>
            {TIME_RANGE_OPTIONS.map((opt) => (
              <button
                key={opt.label}
                className={`${styles.timeRangeBtn} ${timeRange === opt.value ? styles.active : ""}`}
                onClick={() => onTimeRangeChange(opt.value)}
              >
                {opt.label}
              </button>
            ))}
          </div>
          {allDefs.map((def) => (
            <TimeSeriesChart
              key={def.metric}
              title={def.title}
              yLabel={def.yLabel}
              data={multiChartData ? null : (singleSeriesData?.[def.metric] ?? null)}
              multiData={multiChartData?.[def.metric]}
              color={def.color}
              onZoom={onZoom}
            />
          ))}
        </div>
      )}
    </div>
  );
});
