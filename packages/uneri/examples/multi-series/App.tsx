import { useRef, useEffect, useState, useCallback } from "react";
import {
  TimeSeriesChart,
  type MultiSeriesData,
} from "../../src/components/TimeSeriesChart.js";
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
  }, []);

  useEffect(() => {
    const ws = new WebSocket("ws://localhost:9004");
    let tickCount = 0;

    ws.onmessage = (event) => {
      const msg = JSON.parse(event.data);
      if (msg.type === "state") {
        const seriesId = msg.series as string;
        let buf = buffersRef.current.get(seriesId);
        if (!buf) {
          buf = new SeriesBuffer(seriesId);
          buffersRef.current.set(seriesId, buf);
        }
        buf.push(msg.t, msg.value);

        // Update chart every 10 messages to reduce renders
        tickCount++;
        if (tickCount % 10 === 0) {
          buildMultiData();
        }
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
      <h1>@orts/uneri Example: Multi-Series</h1>
      <p>Two sine waves with different frequencies on the same chart.</p>
      <TimeSeriesChart
        title="sin(t) vs sin(3t)"
        yLabel=""
        multiData={multiData}
      />
    </div>
  );
}
