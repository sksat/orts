import { useEffect, useMemo, useRef, useState } from "react";
import {
  IngestBuffer,
  type TableSchema,
  type TimeRange,
  TimeSeriesChart,
  useDuckDB,
  useTimeSeriesStore,
} from "../../src/index.js";

interface SinePoint {
  t: number;
  value: number;
  derivative: number;
}

const sineSchema: TableSchema<SinePoint> = {
  tableName: "sine_data",
  columns: [
    { name: "t", type: "DOUBLE" },
    { name: "value", type: "DOUBLE" },
    { name: "derivative", type: "DOUBLE" },
  ],
  derived: [
    { name: "sine", sql: "value", unit: "" },
    { name: "cosine", sql: "derivative", unit: "" },
    {
      name: "amplitude",
      sql: "sqrt(value*value + derivative*derivative)",
      unit: "",
    },
  ],
  toRow: (p) => [p.t, p.value, p.derivative],
};

export function App() {
  const { conn } = useDuckDB(sineSchema);
  const bufferRef = useRef(new IngestBuffer<SinePoint>());
  const [timeRange, setTimeRange] = useState<TimeRange>(null);

  // Data source: WebSocket or mock (when ?mock is in URL)
  useEffect(() => {
    const isMock = new URLSearchParams(window.location.search).has("mock");
    if (isMock) {
      const startTime = Date.now();
      const interval = setInterval(() => {
        const t = (Date.now() - startTime) / 1000;
        bufferRef.current.push({ t, value: Math.sin(t), derivative: Math.cos(t) });
      }, 100);
      return () => clearInterval(interval);
    }
    const ws = new WebSocket("ws://localhost:9002");
    ws.onmessage = (event) => {
      const msg = JSON.parse(event.data);
      if (msg.type === "state") {
        bufferRef.current.push({
          t: msg.t,
          value: msg.value,
          derivative: msg.derivative,
        });
      }
    };
    return () => ws.close();
  }, []);

  const { data } = useTimeSeriesStore({
    conn,
    schema: sineSchema,
    mode: "realtime",
    replayPoints: null,
    ingestBufferRef: bufferRef,
    timeRange,
    tickInterval: 100,
    queryEveryN: 1,
  });

  // Slice data for individual charts
  const sineData = useMemo(
    () => (data ? ([data.t, data.sine] as [Float64Array, Float64Array]) : null),
    [data],
  );
  const cosineData = useMemo(
    () => (data ? ([data.t, data.cosine] as [Float64Array, Float64Array]) : null),
    [data],
  );
  const amplitudeData = useMemo(
    () => (data ? ([data.t, data.amplitude] as [Float64Array, Float64Array]) : null),
    [data],
  );

  return (
    <div
      style={{
        padding: "1rem",
        background: "#1a1a2e",
        color: "#eee",
        minHeight: "100vh",
      }}
    >
      <h1>uneri Example: Sine Wave</h1>
      <div style={{ marginBottom: "1rem" }}>
        <button onClick={() => setTimeRange(null)}>All</button>
        <button onClick={() => setTimeRange(30)}>30s</button>
        <button onClick={() => setTimeRange(60)}>60s</button>
      </div>
      <TimeSeriesChart title="sin(t)" yLabel="" data={sineData} color="#4af" />
      <TimeSeriesChart title="cos(t)" yLabel="" data={cosineData} color="#f84" />
      <TimeSeriesChart title="amplitude" yLabel="" data={amplitudeData} color="#8f4" />
    </div>
  );
}
