import { useState, useMemo } from "react";
import { TimeSeriesChart, type TimeRange, type ChartDataMap } from "@orts/uneri";

const TIME_RANGE_OPTIONS: { label: string; value: TimeRange }[] = [
  { label: "All", value: null },
  { label: "5 min", value: 300 },
  { label: "30 min", value: 1800 },
  { label: "1 h", value: 3600 },
];

interface GraphPanelProps {
  chartData: ChartDataMap | null;
  isLoading: boolean;
  timeRange: TimeRange;
  onTimeRangeChange: (range: TimeRange) => void;
  /** Called when the user drag-zooms into a time range on any chart. */
  onZoom?: (tMin: number, tMax: number) => void;
}

export function GraphPanel({
  chartData,
  isLoading,
  timeRange,
  onTimeRangeChange,
  onZoom,
}: GraphPanelProps) {
  const [collapsed, setCollapsed] = useState(false);

  const altitudeData = useMemo(
    () =>
      chartData
        ? ([chartData.t, chartData.altitude] as [Float64Array, Float64Array])
        : null,
    [chartData]
  );
  const energyData = useMemo(
    () =>
      chartData
        ? ([chartData.t, chartData.energy] as [Float64Array, Float64Array])
        : null,
    [chartData]
  );
  const angMomData = useMemo(
    () =>
      chartData
        ? ([chartData.t, chartData.angular_momentum] as [Float64Array, Float64Array])
        : null,
    [chartData]
  );
  const velocityData = useMemo(
    () =>
      chartData
        ? ([chartData.t, chartData.velocity] as [Float64Array, Float64Array])
        : null,
    [chartData]
  );
  const smaData = useMemo(
    () =>
      chartData
        ? ([chartData.t, chartData.a] as [Float64Array, Float64Array])
        : null,
    [chartData]
  );
  const eccData = useMemo(
    () =>
      chartData
        ? ([chartData.t, chartData.e] as [Float64Array, Float64Array])
        : null,
    [chartData]
  );
  const incData = useMemo(
    () =>
      chartData
        ? ([chartData.t, chartData.inc_deg] as [Float64Array, Float64Array])
        : null,
    [chartData]
  );
  const raanData = useMemo(
    () =>
      chartData
        ? ([chartData.t, chartData.raan_deg] as [Float64Array, Float64Array])
        : null,
    [chartData]
  );

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
          <TimeSeriesChart
            title="Altitude"
            yLabel="km"
            data={altitudeData}
            color="#4af"
            onZoom={onZoom}
          />
          <TimeSeriesChart
            title="Specific Orbital Energy"
            yLabel="km\u00B2/s\u00B2"
            data={energyData}
            color="#f84"
            onZoom={onZoom}
          />
          <TimeSeriesChart
            title="Angular Momentum"
            yLabel="km\u00B2/s"
            data={angMomData}
            color="#8f4"
            onZoom={onZoom}
          />
          <TimeSeriesChart
            title="Velocity"
            yLabel="km/s"
            data={velocityData}
            color="#f4f"
            onZoom={onZoom}
          />
          <TimeSeriesChart
            title="Semi-major Axis"
            yLabel="km"
            data={smaData}
            color="#4ff"
            onZoom={onZoom}
          />
          <TimeSeriesChart
            title="Eccentricity"
            yLabel="-"
            data={eccData}
            color="#ff4"
            onZoom={onZoom}
          />
          <TimeSeriesChart
            title="Inclination"
            yLabel="deg"
            data={incData}
            color="#f48"
            onZoom={onZoom}
          />
          <TimeSeriesChart
            title="RAAN"
            yLabel="deg"
            data={raanData}
            color="#84f"
            onZoom={onZoom}
          />
        </div>
      )}
    </div>
  );
}
