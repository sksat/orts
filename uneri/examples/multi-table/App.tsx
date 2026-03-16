import { useCallback, useEffect, useRef, useState } from "react";
import {
  createTable,
  IngestBuffer,
  insertPoints,
  queryDerived,
  type TableSchema,
  TimeSeriesChart,
  useDuckDB,
} from "../../src/index.js";
import { alignTimeSeries, type NamedTimeSeries } from "../../src/utils/alignTimeSeries.js";

interface DataPoint {
  t: number;
  value: number;
}

const baseSchema: TableSchema<DataPoint> = {
  tableName: "placeholder",
  columns: [
    { name: "t", type: "DOUBLE" },
    { name: "value", type: "DOUBLE" },
  ],
  derived: [{ name: "value", sql: "value", unit: "" }],
  toRow: (p) => [p.t, p.value],
};

function makeSchema(tableName: string): TableSchema<DataPoint> {
  return { ...baseSchema, tableName };
}

const DISPLAY_MAX_POINTS = 500;

declare global {
  interface Window {
    __multiTableDebug: {
      conn: unknown;
      alphaCount: number;
      betaCount: number;
      /** Query both tables with unified tMax and return alignment stats. */
      queryAlignment: () => Promise<{
        alphaT: number[];
        betaT: number[];
        alphaValues: number[];
        betaValues: number[];
        unifiedTMax: number;
        alignmentRatio: number;
        alphaNanCount: number;
        betaNanCount: number;
      }>;
    };
  }
}

export function App() {
  const { conn } = useDuckDB(baseSchema);
  const alphaBufferRef = useRef(new IngestBuffer<DataPoint>());
  const betaBufferRef = useRef(new IngestBuffer<DataPoint>());
  const [alphaCount, setAlphaCount] = useState(0);
  const [betaCount, setBetaCount] = useState(0);
  const [chartData, setChartData] = useState<{
    t: Float64Array;
    values: Float64Array[];
    labels: string[];
  } | null>(null);

  const alphaSchema = makeSchema("tbl_alpha");
  const betaSchema = makeSchema("tbl_beta");

  // Create tables on mount
  useEffect(() => {
    if (!conn) return;
    (async () => {
      await createTable(conn, alphaSchema);
      await createTable(conn, betaSchema);
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conn]);

  // WebSocket connection
  useEffect(() => {
    const ws = new WebSocket("ws://localhost:9005");
    let aCount = 0,
      bCount = 0;
    ws.onmessage = (event) => {
      const msg = JSON.parse(event.data);
      if (msg.type === "state") {
        const point: DataPoint = { t: msg.t, value: msg.value };
        if (msg.table === "alpha") {
          alphaBufferRef.current.push(point);
          aCount++;
          if (aCount % 50 === 0) setAlphaCount(aCount);
        } else if (msg.table === "beta") {
          betaBufferRef.current.push(point);
          bCount++;
          if (bCount % 50 === 0) setBetaCount(bCount);
        }
      }
    };
    return () => ws.close();
  }, []);

  // Tick loop: drain buffers → insert → query → align → render
  useEffect(() => {
    if (!conn) return;
    let cancelled = false;
    let tickCount = 0;

    const tick = async () => {
      if (cancelled) return;

      // Drain buffers and insert
      const alphaPts = alphaBufferRef.current.drain();
      const betaPts = betaBufferRef.current.drain();
      if (alphaPts.length > 0) await insertPoints(conn, alphaSchema, alphaPts);
      if (betaPts.length > 0) await insertPoints(conn, betaSchema, betaPts);

      tickCount++;
      if (tickCount % 4 === 0) {
        // Compute unified tMax
        const [aMaxRes, bMaxRes] = await Promise.all([
          conn.query("SELECT MAX(t) FROM tbl_alpha"),
          conn.query("SELECT MAX(t) FROM tbl_beta"),
        ]);
        const aMax = Number(aMaxRes.getChildAt(0)!.get(0));
        const bMax = Number(bMaxRes.getChildAt(0)!.get(0));
        const unifiedTMax = Math.max(
          Number.isFinite(aMax) ? aMax : -Infinity,
          Number.isFinite(bMax) ? bMax : -Infinity,
        );
        const tMax = Number.isFinite(unifiedTMax) ? unifiedTMax : undefined;

        // Query both tables with same tMax
        const [alphaData, betaData] = await Promise.all([
          queryDerived(conn, alphaSchema, undefined, DISPLAY_MAX_POINTS, tMax),
          queryDerived(conn, betaSchema, undefined, DISPLAY_MAX_POINTS, tMax),
        ]);

        // Align time series
        const inputs: NamedTimeSeries[] = [];
        if (alphaData.t.length > 0) {
          inputs.push({ label: "alpha", t: alphaData.t, values: alphaData.value as Float64Array });
        }
        if (betaData.t.length > 0) {
          inputs.push({ label: "beta", t: betaData.t, values: betaData.value as Float64Array });
        }

        if (inputs.length > 0) {
          const aligned = alignTimeSeries(inputs);
          setChartData(aligned);
        }
      }

      if (!cancelled) {
        setTimeout(tick, 200);
      }
    };

    setTimeout(tick, 200);
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conn]);

  // Expose debug API
  const queryAlignment = useCallback(async () => {
    if (!conn) throw new Error("no conn");

    // Get counts
    const [aCountRes, bCountRes] = await Promise.all([
      conn.query("SELECT COUNT(*) FROM tbl_alpha"),
      conn.query("SELECT COUNT(*) FROM tbl_beta"),
    ]);
    const aC = Number(aCountRes.getChildAt(0)!.get(0));
    const bC = Number(bCountRes.getChildAt(0)!.get(0));
    if (aC === 0 || bC === 0) {
      throw new Error(`Empty tables: alpha=${aC}, beta=${bC}`);
    }

    // Unified tMax
    const [aMaxRes, bMaxRes] = await Promise.all([
      conn.query("SELECT MAX(t) FROM tbl_alpha"),
      conn.query("SELECT MAX(t) FROM tbl_beta"),
    ]);
    const unifiedTMax = Math.max(
      Number(aMaxRes.getChildAt(0)!.get(0)),
      Number(bMaxRes.getChildAt(0)!.get(0)),
    );

    // Downsampled queries with unified tMax
    const [alphaData, betaData] = await Promise.all([
      queryDerived(conn, alphaSchema, undefined, DISPLAY_MAX_POINTS, unifiedTMax),
      queryDerived(conn, betaSchema, undefined, DISPLAY_MAX_POINTS, unifiedTMax),
    ]);

    const alphaT = Array.from(alphaData.t);
    const betaT = Array.from(betaData.t);
    const alphaValues = Array.from(alphaData.value as Float64Array);
    const betaValues = Array.from(betaData.value as Float64Array);

    // NaN check
    const alphaNanCount = alphaValues.filter((v) => Number.isNaN(v)).length;
    const betaNanCount = betaValues.filter((v) => Number.isNaN(v)).length;

    // Alignment check
    const aSet = new Set(alphaT);
    const bSet = new Set(betaT);
    let matching = 0;
    for (const t of aSet) {
      if (bSet.has(t)) matching++;
    }
    const totalUnique = new Set([...alphaT, ...betaT]).size;
    const alignmentRatio = totalUnique > 0 ? matching / totalUnique : 0;

    return {
      alphaT,
      betaT,
      alphaValues,
      betaValues,
      unifiedTMax,
      alignmentRatio,
      alphaNanCount,
      betaNanCount,
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conn]);

  useEffect(() => {
    window.__multiTableDebug = {
      conn,
      alphaCount,
      betaCount,
      queryAlignment,
    };
  }, [conn, alphaCount, betaCount, queryAlignment]);

  // Chart rendering
  const alphaChartData = chartData?.labels.includes("alpha")
    ? ([chartData.t, chartData.values[chartData.labels.indexOf("alpha")]] as [
        Float64Array,
        Float64Array,
      ])
    : null;
  const betaChartData = chartData?.labels.includes("beta")
    ? ([chartData.t, chartData.values[chartData.labels.indexOf("beta")]] as [
        Float64Array,
        Float64Array,
      ])
    : null;

  // Count NaN in aligned data
  const nanStats = chartData
    ? {
        alpha: chartData.labels.includes("alpha")
          ? Array.from(chartData.values[chartData.labels.indexOf("alpha")]).filter((v) =>
              Number.isNaN(v),
            ).length
          : 0,
        beta: chartData.labels.includes("beta")
          ? Array.from(chartData.values[chartData.labels.indexOf("beta")]).filter((v) =>
              Number.isNaN(v),
            ).length
          : 0,
      }
    : null;

  return (
    <div
      style={{
        padding: "1rem",
        background: "#1a1a2e",
        color: "#eee",
        minHeight: "100vh",
      }}
    >
      <h1>uneri: Multi-Table Alignment Test</h1>
      <p data-testid="stats">
        Alpha: {alphaCount} pts | Beta: {betaCount} pts | Chart: {chartData?.t?.length ?? 0} pts |
        NaN: alpha={nanStats?.alpha ?? "-"} beta={nanStats?.beta ?? "-"}
      </p>
      <TimeSeriesChart title="alpha (sin)" yLabel="" data={alphaChartData} color="#4af" />
      <TimeSeriesChart title="beta (cos+2)" yLabel="" data={betaChartData} color="#f84" />
    </div>
  );
}
