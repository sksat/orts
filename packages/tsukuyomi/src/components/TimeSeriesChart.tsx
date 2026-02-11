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

interface TimeSeriesChartProps {
  title: string;
  yLabel: string;
  /** uPlot-format data: [xValues, yValues] */
  data: [Float64Array, Float64Array] | null;
  height?: number;
  color?: string;
  /** Called when the user zooms into a time range via drag. */
  onZoom?: (tMin: number, tMax: number) => void;
}

export function TimeSeriesChart({
  title,
  yLabel,
  data,
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

  /** Build uPlot options. Extracted so chart can be recreated on error recovery. */
  function buildOpts(container: HTMLDivElement): uPlot.Options {
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
      series: [
        {},
        {
          label: yLabel,
          stroke: color,
          width: 1.5,
        },
      ],
      cursor: {
        show: true,
        drag: { x: true, y: false },
      },
      legend: { show: true, live: true },
      hooks: {
        setScale: [
          (u: uPlot, scaleKey: string) => {
            // Skip scale changes triggered by programmatic setData calls —
            // only fire onZoom for user-initiated drag-zoom interactions.
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

  // Create chart on mount
  useEffect(() => {
    if (!containerRef.current) return;

    const emptyData: uPlot.AlignedData = [[], []];
    const chart = new uPlot(
      buildOpts(containerRef.current),
      data ?? emptyData,
      containerRef.current,
    );
    chartRef.current = chart;

    return () => {
      chart.destroy();
      chartRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Update data (need at least 2 points for uPlot axis calculations)
  useEffect(() => {
    if (!chartRef.current || !data || data[0].length < 2) return;
    isProgrammaticRef.current = true;
    try {
      chartRef.current.setData(data);
    } catch {
      // uPlot's internal state is corrupted after a partial setData failure.
      // Destroy the broken instance and create a fresh one with the current data.
      const container = containerRef.current;
      if (container) {
        chartRef.current!.destroy();
        chartRef.current = new uPlot(buildOpts(container), data, container);
      }
    }
    isProgrammaticRef.current = false;
  }, [data]);

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
            // setSize can also trigger axis recalculation; recover if it fails
            const container = containerRef.current;
            const currentData = chartRef.current!.data;
            if (container) {
              chartRef.current!.destroy();
              chartRef.current = new uPlot(
                buildOpts(container),
                currentData,
                container,
              );
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
