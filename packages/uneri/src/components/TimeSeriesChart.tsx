import { useRef, useEffect } from "react";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";

/**
 * Custom y-axis range function that prevents uPlot's axis split function
 * from crashing with near-constant data.
 *
 * When all y-values are nearly identical, the default range calculation
 * produces such a tiny axis range that the tick generation loop tries to
 * create an impossibly large array (RangeError: Invalid array length).
 *
 * Enforces a minimum visible range proportional to the data's magnitude.
 */
export function safeYRange(
  _u: uPlot,
  dataMin: number,
  dataMax: number,
  _scaleKey: string,
): uPlot.Range.MinMax {
  let min = dataMin;
  let max = dataMax;
  const delta = max - min;
  const magnitude = Math.max(Math.abs(min), Math.abs(max));
  // Guard: expand near-zero deltas to prevent uPlot's axis tick generator
  // from creating impossibly large arrays (RangeError).
  const minDelta = Math.max(magnitude * 1e-4, 1e-9);

  if (delta < minDelta) {
    const center = (min + max) / 2;
    const pad = minDelta / 2;
    min = center - pad;
    max = center + pad;
  }

  // Delegate to uPlot's built-in range calculation for nice-number rounding.
  // false = no soft limits at 0; axis tightly wraps the data.
  return uPlot.rangeNum(min, max, 0.1, false);
}

/** Configuration for a single series in a multi-series chart. */
export interface SeriesConfig {
  label: string;
  color: string;
}

/** Multi-series data: shared x-axis + multiple y arrays. */
export interface MultiSeriesData {
  /** Shared time axis. */
  t: Float64Array;
  /** One Float64Array per series (NaN for gaps). */
  values: Float64Array[];
  /** Config for each series (same order as values[]). */
  series: SeriesConfig[];
}

/** Build uPlot series config array from SeriesConfig[]. */
export function buildMultiSeriesConfig(
  configs: SeriesConfig[],
): uPlot.Series[] {
  const result: uPlot.Series[] = [{}]; // x-axis placeholder
  for (const cfg of configs) {
    result.push({
      label: cfg.label,
      stroke: cfg.color,
      width: 1.5,
    });
  }
  return result;
}

interface TimeSeriesChartProps {
  title: string;
  yLabel: string;
  /** Single-series data: [xValues, yValues]. */
  data?: [Float64Array, Float64Array] | null;
  /** Multi-series data (takes precedence over `data`). */
  multiData?: MultiSeriesData | null;
  height?: number;
  /** Color for single-series mode. Ignored when multiData is used. */
  color?: string;
  /** Called when the user zooms into a time range via drag. */
  onZoom?: (tMin: number, tMax: number) => void;
}

export function TimeSeriesChart({
  title,
  yLabel,
  data,
  multiData,
  height = 200,
  color = "#0f0",
  onZoom,
}: TimeSeriesChartProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<uPlot | null>(null);
  const onZoomRef = useRef(onZoom);
  onZoomRef.current = onZoom;
  // Guard: suppress setScale callback during programmatic updates (setData / setSize).
  // Only user-initiated drag-zoom should trigger onZoom.
  const isProgrammaticRef = useRef(false);
  // Track series count to detect when chart needs recreation.
  const seriesCountRef = useRef(0);

  /** Resolve the effective data and series config. */
  function resolveData(): {
    plotData: uPlot.AlignedData;
    seriesConfig: uPlot.Series[];
  } | null {
    if (multiData && multiData.series.length > 0 && multiData.t.length >= 2) {
      return {
        plotData: [multiData.t, ...multiData.values] as uPlot.AlignedData,
        seriesConfig: buildMultiSeriesConfig(multiData.series),
      };
    }
    if (data && data[0].length >= 2) {
      return {
        plotData: data,
        seriesConfig: [
          {},
          { label: yLabel, stroke: color, width: 1.5 },
        ],
      };
    }
    return null;
  }

  /** Build uPlot options. */
  function buildOpts(
    container: HTMLDivElement,
    seriesConfig: uPlot.Series[],
  ): uPlot.Options {
    return {
      title,
      width: container.clientWidth,
      height,
      scales: {
        x: { time: false },
        y: { range: safeYRange },
      },
      axes: [
        {
          label: "Time (s)",
          stroke: "#888",
          grid: { stroke: "rgba(255,255,255,0.05)" },
        },
        {
          label: yLabel,
          stroke: "#888",
          grid: { stroke: "rgba(255,255,255,0.05)" },
        },
      ],
      series: seriesConfig,
      cursor: {
        show: true,
        drag: { x: true, y: false },
      },
      legend: { show: true, live: true },
      hooks: {
        setScale: [
          (u: uPlot, scaleKey: string) => {
            if (isProgrammaticRef.current) return;
            if (scaleKey === "x") {
              const min = u.scales.x.min;
              const max = u.scales.x.max;
              if (min != null && max != null && onZoomRef.current) {
                onZoomRef.current(min, max);
              }
            }
          },
        ],
      },
    };
  }

  /** Create or recreate the chart. */
  function createChart(
    container: HTMLDivElement,
    plotData: uPlot.AlignedData,
    seriesConfig: uPlot.Series[],
  ): uPlot {
    const chart = new uPlot(
      buildOpts(container, seriesConfig),
      plotData,
      container,
    );
    seriesCountRef.current = seriesConfig.length;
    return chart;
  }

  // Create chart on mount
  useEffect(() => {
    if (!containerRef.current) return;

    const resolved = resolveData();
    const seriesConfig = resolved?.seriesConfig ?? [
      {},
      { label: yLabel, stroke: color, width: 1.5 },
    ];
    const plotData = resolved?.plotData ?? ([[], []] as uPlot.AlignedData);

    chartRef.current = createChart(containerRef.current, plotData, seriesConfig);

    return () => {
      chartRef.current?.destroy();
      chartRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Update data
  useEffect(() => {
    if (!chartRef.current || !containerRef.current) return;

    const resolved = resolveData();
    if (!resolved) return;

    const { plotData, seriesConfig } = resolved;

    // If series count changed, recreate the chart (uPlot series are fixed at construction).
    if (seriesConfig.length !== seriesCountRef.current) {
      chartRef.current.destroy();
      chartRef.current = createChart(
        containerRef.current,
        plotData,
        seriesConfig,
      );
      return;
    }

    isProgrammaticRef.current = true;
    try {
      chartRef.current.setData(plotData);
    } catch {
      chartRef.current!.destroy();
      chartRef.current = createChart(
        containerRef.current,
        plotData,
        seriesConfig,
      );
    }
    isProgrammaticRef.current = false;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data, multiData]);

  // Handle resize
  useEffect(() => {
    if (!containerRef.current || !chartRef.current) return;

    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const width = entry.contentRect.width;
        if (chartRef.current && width > 0) {
          isProgrammaticRef.current = true;
          try {
            chartRef.current.setSize({ width, height });
          } catch {
            const container = containerRef.current;
            const currentData = chartRef.current!.data;
            const resolved = resolveData();
            const seriesConfig = resolved?.seriesConfig ?? [
              {},
              { label: yLabel, stroke: color, width: 1.5 },
            ];
            if (container) {
              chartRef.current!.destroy();
              chartRef.current = createChart(container, currentData, seriesConfig);
            }
          }
          isProgrammaticRef.current = false;
        }
      }
    });

    observer.observe(containerRef.current);
    return () => observer.disconnect();
  }, [height]);

  return <div ref={containerRef} data-testid="time-series-chart" style={{ width: "100%" }} />;
}
