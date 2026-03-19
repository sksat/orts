import { useCallback, useEffect, useRef, useState } from "react";
import { type MultiSeriesData, TimeSeriesChart } from "../../src/components/TimeSeriesChart.js";
import { alignTimeSeries, type NamedTimeSeries } from "../../src/utils/alignTimeSeries.js";

/** Bounded buffer for a single named time series. */
class SeriesBuffer {
  readonly label: string;
  private times: number[] = [];
  private vals: number[] = [];
  private maxLen = 500;

  constructor(label: string) {
    this.label = label;
  }

  push(t: number, v: number) {
    this.times.push(t);
    this.vals.push(v);
    if (this.times.length > this.maxLen) {
      this.times.splice(0, this.times.length - this.maxLen);
      this.vals.splice(0, this.vals.length - this.maxLen);
    }
  }

  toNamedTimeSeries(): NamedTimeSeries {
    return {
      label: this.label,
      t: Float64Array.from(this.times),
      values: Float64Array.from(this.vals),
    };
  }
}

export function App() {
  const buffersRef = useRef(new Map<string, SeriesBuffer>());
  const [multiData, setMultiData] = useState<MultiSeriesData | null>(null);

  const COLORS: Record<string, string> = {
    slow: "#4af",
    fast: "#f84",
  };

  const buildMultiData = useCallback(() => {
    const buffers = buffersRef.current;
    if (buffers.size === 0) return;

    const inputs: NamedTimeSeries[] = [];
    for (const buf of buffers.values()) {
      inputs.push(buf.toNamedTimeSeries());
    }

    const aligned = alignTimeSeries(inputs);
    if (aligned.t.length < 2) return;

    setMultiData({
      t: aligned.t,
      values: aligned.values,
      series: aligned.labels.map((label) => ({
        label,
        color: COLORS[label] ?? "#0f0",
      })),
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const isMock = new URLSearchParams(window.location.search).has("mock");
    let tickCount = 0;

    const pushPoint = (series: string, t: number, value: number) => {
      let buf = buffersRef.current.get(series);
      if (!buf) {
        buf = new SeriesBuffer(series);
        buffersRef.current.set(series, buf);
      }
      buf.push(t, value);
      tickCount++;
      if (tickCount % 10 === 0) buildMultiData();
    };

    if (isMock) {
      const startTime = Date.now();
      let toggle = false;
      const interval = setInterval(() => {
        const t = (Date.now() - startTime) / 1000;
        toggle = !toggle;
        if (toggle) {
          pushPoint("slow", t, Math.sin(t));
        } else {
          pushPoint("fast", t, Math.sin(t * 3));
        }
      }, 50);
      return () => clearInterval(interval);
    }

    const ws = new WebSocket("ws://localhost:9004");
    ws.onmessage = (event) => {
      const msg = JSON.parse(event.data);
      if (msg.type === "state") {
        pushPoint(msg.series as string, msg.t, msg.value);
      }
    };
    return () => ws.close();
  }, [buildMultiData]);

  return (
    <div
      style={{
        padding: "1rem",
        background: "#1a1a2e",
        color: "#eee",
        minHeight: "100vh",
      }}
    >
      <h1>uneri Example: Multi-Series</h1>
      <p>Two sine waves with different frequencies on the same chart.</p>
      <TimeSeriesChart title="sin(t) vs sin(3t)" yLabel="" multiData={multiData} />
    </div>
  );
}
