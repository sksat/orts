import { useState, useMemo } from "react";
import { TimeSeriesChart } from "./TimeSeriesChart.js";
import type { ChartData } from "../db/orbitStore.js";

interface GraphPanelProps {
  chartData: ChartData | null;
  isLoading: boolean;
}

export function GraphPanel({ chartData, isLoading }: GraphPanelProps) {
  const [collapsed, setCollapsed] = useState(false);

  const altitudeData = useMemo(
    () =>
      chartData
        ? ([chartData[0], chartData[1]] as [Float64Array, Float64Array])
        : null,
    [chartData]
  );
  const energyData = useMemo(
    () =>
      chartData
        ? ([chartData[0], chartData[2]] as [Float64Array, Float64Array])
        : null,
    [chartData]
  );
  const angMomData = useMemo(
    () =>
      chartData
        ? ([chartData[0], chartData[3]] as [Float64Array, Float64Array])
        : null,
    [chartData]
  );
  const velocityData = useMemo(
    () =>
      chartData
        ? ([chartData[0], chartData[4]] as [Float64Array, Float64Array])
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
          <TimeSeriesChart
            title="Altitude"
            yLabel="km"
            data={altitudeData}
            color="#4af"
          />
          <TimeSeriesChart
            title="Specific Orbital Energy"
            yLabel="km\u00B2/s\u00B2"
            data={energyData}
            color="#f84"
          />
          <TimeSeriesChart
            title="Angular Momentum"
            yLabel="km\u00B2/s"
            data={angMomData}
            color="#8f4"
          />
          <TimeSeriesChart
            title="Velocity"
            yLabel="km/s"
            data={velocityData}
            color="#f4f"
          />
        </div>
      )}
    </div>
  );
}
