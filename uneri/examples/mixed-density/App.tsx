import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  IngestBuffer,
  type TableSchema,
  type TimeRange,
  TimeSeriesChart,
  useDuckDB,
  useTimeSeriesStore,
} from "../../src/index.js";

interface MixedPoint {
  t: number;
  value: number;
  derivative: number;
}

const mixedSchema: TableSchema<MixedPoint> = {
  tableName: "mixed_data",
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

declare global {
  interface Window {
    __uneriDebug: {
      chartData: Record<string, Float64Array> | null;
      rowCount: number;
      queryRowCount: () => Promise<number>;
    };
  }
}

export function App() {
  const { conn } = useDuckDB(mixedSchema);
  const bufferRef = useRef(new IngestBuffer<MixedPoint>());
  const [timeRange, setTimeRange] = useState<TimeRange>(null);
  const [pointsReceived, setPointsReceived] = useState(0);

  // Data source: WebSocket or mock (when ?mock is in URL)
  useEffect(() => {
    const isMock = new URLSearchParams(window.location.search).has("mock");
    let count = 0;

    const pushPoint = (t: number, value: number, derivative: number) => {
      bufferRef.current.push({ t, value, derivative });
      count++;
      if (count % 100 === 0) setPointsReceived(count);
    };

    if (isMock) {
      // Phase 1: sparse overview
      for (let i = 0; i < 100; i++) {
        const t = i * 50;
        pushPoint(t, Math.sin(t * 0.001), Math.cos(t * 0.001));
      }
      // Phase 2: dense streaming
      let streamT = 5000;
      const interval = setInterval(() => {
        pushPoint(streamT, Math.sin(streamT * 0.001), Math.cos(streamT * 0.001));
        streamT += 0.1;
      }, 10);
      return () => clearInterval(interval);
    }

    const ws = new WebSocket("ws://localhost:9003");
    ws.onmessage = (event) => {
      const msg = JSON.parse(event.data);
      if (msg.type === "state") {
        pushPoint(msg.t, msg.value, msg.derivative);
      }
    };
    return () => ws.close();
  }, []);

  const { data } = useTimeSeriesStore({
    conn,
    schema: mixedSchema,
    ingestBufferRef: bufferRef,
    timeRange,
    tickInterval: 100,
    queryEveryN: 1,
  });

  // Expose debug data on window for Playwright tests
  const queryRowCount = useCallback(async () => {
    if (!conn) return 0;
    const result = await conn.query(`SELECT COUNT(*) AS cnt FROM ${mixedSchema.tableName}`);
    return Number(result.getChildAt(0)!.toArray()[0]);
  }, [conn]);

  useEffect(() => {
    window.__uneriDebug = {
      chartData: data,
      rowCount: pointsReceived,
      queryRowCount,
    };
  }, [data, pointsReceived, queryRowCount]);

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
      <h1>uneri: Mixed-Density Test</h1>
      <p data-testid="points-received">
        Points received: {pointsReceived} | Chart points: {data?.t?.length ?? 0} | Buffer latestT:{" "}
        {bufferRef.current.latestT.toFixed(1)}
      </p>
      <div style={{ marginBottom: "1rem" }}>
        <button onClick={() => setTimeRange(null)}>All</button>
        <button onClick={() => setTimeRange(30)}>30s</button>
        <button onClick={() => setTimeRange(60)}>60s</button>
      </div>
      <TimeSeriesChart title="sine" yLabel="" data={sineData} color="#4af" />
      <TimeSeriesChart title="cosine" yLabel="" data={cosineData} color="#f84" />
      <TimeSeriesChart title="amplitude" yLabel="" data={amplitudeData} color="#8f4" />
    </div>
  );
}
