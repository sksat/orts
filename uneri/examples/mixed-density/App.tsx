import { useRef, useEffect, useState, useMemo, useCallback } from "react";
import {
  IngestBuffer,
  useDuckDB,
  useTimeSeriesStore,
  TimeSeriesChart,
  type TableSchema,
  type TimeRange,
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

  // WebSocket connection
  useEffect(() => {
    const ws = new WebSocket("ws://localhost:9003");
    let count = 0;
    ws.onmessage = (event) => {
      const msg = JSON.parse(event.data);
      if (msg.type === "state") {
        bufferRef.current.push({
          t: msg.t,
          value: msg.value,
          derivative: msg.derivative,
        });
        count++;
        // Update count periodically to avoid excessive re-renders
        if (count % 100 === 0) {
          setPointsReceived(count);
        }
      }
    };
    return () => ws.close();
  }, []);

  const { data } = useTimeSeriesStore({
    conn,
    schema: mixedSchema,
    mode: "realtime",
    replayPoints: null,
    ingestBufferRef: bufferRef,
    timeRange,
  });

  // Expose debug data on window for Playwright tests
  const queryRowCount = useCallback(async () => {
    if (!conn) return 0;
    const result = await conn.query(
      `SELECT COUNT(*) AS cnt FROM ${mixedSchema.tableName}`,
    );
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
    () =>
      data ? ([data.t, data.sine] as [Float64Array, Float64Array]) : null,
    [data],
  );
  const cosineData = useMemo(
    () =>
      data ? ([data.t, data.cosine] as [Float64Array, Float64Array]) : null,
    [data],
  );
  const amplitudeData = useMemo(
    () =>
      data
        ? ([data.t, data.amplitude] as [Float64Array, Float64Array])
        : null,
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
        Points received: {pointsReceived} | Chart points:{" "}
        {data?.t?.length ?? 0} | Buffer latestT:{" "}
        {bufferRef.current.latestT.toFixed(1)}
      </p>
      <div style={{ marginBottom: "1rem" }}>
        <button onClick={() => setTimeRange(null)}>All</button>
        <button onClick={() => setTimeRange(30)}>30s</button>
        <button onClick={() => setTimeRange(60)}>60s</button>
      </div>
      <TimeSeriesChart title="sine" yLabel="" data={sineData} color="#4af" />
      <TimeSeriesChart
        title="cosine"
        yLabel=""
        data={cosineData}
        color="#f84"
      />
      <TimeSeriesChart
        title="amplitude"
        yLabel=""
        data={amplitudeData}
        color="#8f4"
      />
    </div>
  );
}
