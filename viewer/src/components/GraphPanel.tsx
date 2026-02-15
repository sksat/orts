import { useState, useMemo } from "react";
import { TimeSeriesChart, type TimeRange, type ChartDataMap } from "uneri";
import type { MultiChartDataMap } from "../hooks/buildMultiChartData.js";

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
}

export function GraphPanel({
  chartData,
  multiChartData,
  isLoading,
  timeRange,
  onTimeRangeChange,
  onZoom,
}: GraphPanelProps) {
  const [collapsed, setCollapsed] = useState(false);

  // Single-series data extraction (for backward compat / single satellite)
  const singleSeriesData = useMemo(() => {
    if (!chartData) return null;
    const result: Record<string, [Float64Array, Float64Array] | null> = {};
    for (const def of CHART_DEFS) {
      result[def.metric] = chartData[def.metric]
        ? [chartData.t, chartData[def.metric]] as [Float64Array, Float64Array]
        : null;
    }
    return result;
  }, [chartData]);

  return (
    <div className={`graph-panel ${collapsed ? "collapsed" : ""}`}>
      <button
        className="graph-panel-toggle"
        onClick={() => setCollapsed((c) => !c)}
      >
        {collapsed ? "\u25C0 Graphs" : "\u25B6"}
      </button>
      {!collapsed && (
        <div className="graph-panel-content">
          {isLoading && <div className="graph-loading">Loading DuckDB...</div>}
          <div className="time-range-selector">
            {TIME_RANGE_OPTIONS.map((opt) => (
              <button
                key={opt.label}
                className={`time-range-btn ${timeRange === opt.value ? "active" : ""}`}
                onClick={() => onTimeRangeChange(opt.value)}
              >
                {opt.label}
              </button>
            ))}
          </div>
          {CHART_DEFS.map((def) => (
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
}
