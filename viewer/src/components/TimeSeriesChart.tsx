import { useRef, useEffect } from "react";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";

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
  height = 140,
  color = "#0f0",
  onZoom,
}: TimeSeriesChartProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<uPlot | null>(null);
  const onZoomRef = useRef(onZoom);
  onZoomRef.current = onZoom;

  // Create chart on mount
  useEffect(() => {
    if (!containerRef.current) return;

    const opts: uPlot.Options = {
      title,
      width: containerRef.current.clientWidth,
      height,
      scales: {
        x: { time: false },
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
      legend: { show: false },
      hooks: {
        setScale: [
          (u: uPlot, scaleKey: string) => {
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

    const emptyData: uPlot.AlignedData = [[], []];
    const chart = new uPlot(opts, data ?? emptyData, containerRef.current);
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
    try {
      chartRef.current.setData(data);
    } catch {
      // uPlot throws RangeError when y-range is near-zero (e.g. constant
      // energy in a circular orbit). Safe to ignore — next update with
      // more data will succeed.
    }
  }, [data]);

  // Handle resize
  useEffect(() => {
    if (!containerRef.current || !chartRef.current) return;

    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const width = entry.contentRect.width;
        if (chartRef.current && width > 0) {
          try {
            chartRef.current.setSize({ width, height });
          } catch {
            // uPlot may throw during axis recalculation with edge-case data
          }
        }
      }
    });

    observer.observe(containerRef.current);
    return () => observer.disconnect();
  }, [height]);

  return <div ref={containerRef} style={{ width: "100%" }} />;
}
