import { useEffect, useRef } from "react";
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

/**
 * Compute Grafana-style legend isolation visibility.
 *
 * Click a series → isolate (show only that series).
 * Click the already-isolated series → show all.
 *
 * @param clickedIndex 1-based uPlot series index (0 = x-axis, ignored)
 * @param currentShow  Current visibility, indexed 0..N where 0 is x-axis
 * @returns New visibility array (same length as currentShow)
 */
export function computeLegendIsolation(clickedIndex: number, currentShow: boolean[]): boolean[] {
  if (clickedIndex < 1 || clickedIndex >= currentShow.length) return currentShow;

  // Is the clicked series currently the only visible y-series?
  const isAlreadyIsolated = currentShow.every(
    (show, i) => i === 0 || (i === clickedIndex ? show : !show),
  );

  if (isAlreadyIsolated) {
    // Un-isolate: show all
    return currentShow.map(() => true);
  }

  // Isolate: show only clicked, hide others
  return currentShow.map((_, i) => i === 0 || i === clickedIndex);
}

/** Build uPlot series config array from SeriesConfig[]. */
export function buildMultiSeriesConfig(configs: SeriesConfig[]): uPlot.Series[] {
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

/**
 * Attach Grafana-style legend click behavior to a uPlot chart.
 * Intercepts clicks on legend entries in the capture phase to prevent
 * uPlot's default toggle, then applies isolation logic.
 */
function attachLegendIsolation(chart: uPlot): void {
  const legend = chart.root.querySelector(".u-legend");
  if (!legend) return;

  legend.addEventListener(
    "click",
    (e) => {
      const target = (e.target as HTMLElement).closest(".u-series");
      if (!target) return;

      const entries = Array.from(legend.querySelectorAll(".u-series"));
      const clickedIdx = entries.indexOf(target as Element);
      // Ignore x-axis (index 0) or not found
      if (clickedIdx < 1) return;

      // Prevent uPlot's default series toggle
      e.stopPropagation();

      const currentShow = chart.series.map((s) => s.show !== false);
      const newShow = computeLegendIsolation(clickedIdx, currentShow);

      for (let i = 1; i < chart.series.length; i++) {
        if (currentShow[i] !== newShow[i]) {
          chart.setSeries(i, { show: newShow[i] });
        }
      }
    },
    { capture: true },
  );
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
  // Guard: suppress setScale callback during programmatic updates (setData / setSize / createChart).
  // Only user-initiated drag-zoom should trigger onZoom.
  // Uses a depth counter instead of boolean to handle overlapping programmatic operations
  // (e.g. setData + setSize in same frame). Each operation increments on start and
  // decrements via requestAnimationFrame to cover both sync and async setScale firings.
  const programmaticDepthRef = useRef(0);
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
        seriesConfig: [{}, { label: yLabel, stroke: color, width: 1.5 }],
      };
    }
    return null;
  }

  /** Build uPlot options. */
  function buildOpts(container: HTMLDivElement, seriesConfig: uPlot.Series[]): uPlot.Options {
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
            if (programmaticDepthRef.current > 0) return;
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
    programmaticDepthRef.current++;
    const chart = new uPlot(buildOpts(container, seriesConfig), plotData, container);
    requestAnimationFrame(() => {
      programmaticDepthRef.current--;
    });
    seriesCountRef.current = seriesConfig.length;

    // Attach Grafana-style legend isolation for multi-series charts (2+ y-series)
    if (seriesConfig.length > 2) {
      attachLegendIsolation(chart);
    }

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
  }, [color, createChart, resolveData, yLabel]);

  // Update data
  useEffect(() => {
    if (!chartRef.current || !containerRef.current) return;

    const resolved = resolveData();
    if (!resolved) return;

    const { plotData, seriesConfig } = resolved;

    // If series count changed, recreate the chart (uPlot series are fixed at construction).
    if (seriesConfig.length !== seriesCountRef.current) {
      chartRef.current.destroy();
      chartRef.current = createChart(containerRef.current, plotData, seriesConfig);
      return;
    }

    programmaticDepthRef.current++;
    try {
      chartRef.current.setData(plotData);
    } catch {
      chartRef.current!.destroy();
      chartRef.current = createChart(containerRef.current, plotData, seriesConfig);
    }
    requestAnimationFrame(() => {
      programmaticDepthRef.current--;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [createChart, resolveData]);

  // Handle resize
  useEffect(() => {
    if (!containerRef.current || !chartRef.current) return;

    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const width = entry.contentRect.width;
        if (chartRef.current && width > 0) {
          programmaticDepthRef.current++;
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
          requestAnimationFrame(() => {
            programmaticDepthRef.current--;
          });
        }
      }
    });

    observer.observe(containerRef.current);
    return () => observer.disconnect();
  }, [height, color, createChart, resolveData, yLabel]);

  return <div ref={containerRef} data-testid="time-series-chart" style={{ width: "100%" }} />;
}
