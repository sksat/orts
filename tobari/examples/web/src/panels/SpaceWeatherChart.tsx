import { useEffect, useRef, useState } from "react";
import { useCanvasSize } from "../hooks/useCanvasSize.js";
import { spaceWeatherSeriesAsync } from "../wasm/workerClient.js";

interface Props {
  epochJd: number;
}

interface SwSeries {
  jd: Float64Array;
  f107: Float64Array;
  ap: Float64Array;
}

const PADDING = { top: 20, right: 20, bottom: 40, left: 60 };

function jdToLabel(jd: number): string {
  const ms = (jd - 2440587.5) * 86400000;
  return new Date(ms).toISOString().slice(0, 7); // YYYY-MM
}

function renderChart(canvas: HTMLCanvasElement, series: SwSeries, epochJd: number) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;

  const w = canvas.width;
  const h = canvas.height;
  const plotW = w - PADDING.left - PADDING.right;
  const plotH = h - PADDING.top - PADDING.bottom;

  ctx.clearRect(0, 0, w, h);

  const n = series.jd.length;
  if (n === 0) return;

  const jdMin = series.jd[0];
  const jdMax = series.jd[n - 1];
  const jdSpan = jdMax - jdMin;
  if (jdSpan <= 0) return;

  const toX = (jd: number) => PADDING.left + ((jd - jdMin) / jdSpan) * plotW;

  // F10.7 chart (top half)
  const f107H = plotH * 0.45;
  const f107Top = PADDING.top;
  let f107Min = Number.POSITIVE_INFINITY;
  let f107Max = Number.NEGATIVE_INFINITY;
  for (let i = 0; i < n; i++) {
    if (series.f107[i] > 0) {
      if (series.f107[i] < f107Min) f107Min = series.f107[i];
      if (series.f107[i] > f107Max) f107Max = series.f107[i];
    }
  }
  f107Min = Math.floor(f107Min / 10) * 10;
  f107Max = Math.ceil(f107Max / 10) * 10;
  const toF107Y = (v: number) => f107Top + f107H * (1 - (v - f107Min) / (f107Max - f107Min));

  // Ap chart (bottom half)
  const apH = plotH * 0.45;
  const apTop = PADDING.top + f107H + plotH * 0.1;
  let apMax = 0;
  for (let i = 0; i < n; i++) {
    if (series.ap[i] > apMax) apMax = series.ap[i];
  }
  apMax = Math.ceil(apMax / 10) * 10;
  const toApY = (v: number) => apTop + apH * (1 - v / apMax);

  // Grid
  ctx.strokeStyle = "#333";
  ctx.lineWidth = 0.5;
  ctx.setLineDash([4, 4]);
  ctx.font = "11px monospace";
  ctx.fillStyle = "#666";
  ctx.textAlign = "right";

  // F10.7 Y grid
  for (let v = f107Min; v <= f107Max; v += 50) {
    const y = toF107Y(v);
    ctx.beginPath();
    ctx.moveTo(PADDING.left, y);
    ctx.lineTo(w - PADDING.right, y);
    ctx.stroke();
    ctx.fillText(`${v}`, PADDING.left - 6, y + 4);
  }

  // Ap Y grid
  for (let v = 0; v <= apMax; v += 20) {
    const y = toApY(v);
    ctx.beginPath();
    ctx.moveTo(PADDING.left, y);
    ctx.lineTo(w - PADDING.right, y);
    ctx.stroke();
    ctx.fillText(`${v}`, PADDING.left - 6, y + 4);
  }

  // X grid (monthly)
  ctx.textAlign = "center";
  const dayStep = Math.max(30, Math.floor(jdSpan / 12));
  for (let jd = jdMin; jd <= jdMax; jd += dayStep) {
    const x = toX(jd);
    ctx.beginPath();
    ctx.moveTo(x, PADDING.top);
    ctx.lineTo(x, h - PADDING.bottom);
    ctx.stroke();
    ctx.fillText(jdToLabel(jd), x, h - PADDING.bottom + 14);
  }
  ctx.setLineDash([]);

  // F10.7 line
  ctx.strokeStyle = "#ff8844";
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  let started = false;
  for (let i = 0; i < n; i++) {
    if (series.f107[i] <= 0) continue;
    const x = toX(series.jd[i]);
    const y = toF107Y(series.f107[i]);
    if (!started) {
      ctx.moveTo(x, y);
      started = true;
    } else {
      ctx.lineTo(x, y);
    }
  }
  ctx.stroke();

  // Ap line
  ctx.strokeStyle = "#44aaff";
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  started = false;
  for (let i = 0; i < n; i++) {
    const x = toX(series.jd[i]);
    const y = toApY(series.ap[i]);
    if (!started) {
      ctx.moveTo(x, y);
      started = true;
    } else {
      ctx.lineTo(x, y);
    }
  }
  ctx.stroke();

  // Labels
  ctx.font = "12px sans-serif";
  ctx.fillStyle = "#ff8844";
  ctx.textAlign = "left";
  ctx.fillText("F10.7 [SFU]", PADDING.left + 4, f107Top + 14);
  ctx.fillStyle = "#44aaff";
  ctx.fillText("Ap", PADDING.left + 4, apTop + 14);

  // Epoch cursor
  if (epochJd >= jdMin && epochJd <= jdMax) {
    const cx = toX(epochJd);
    ctx.strokeStyle = "#fff";
    ctx.lineWidth = 1;
    ctx.setLineDash([2, 2]);
    ctx.beginPath();
    ctx.moveTo(cx, PADDING.top);
    ctx.lineTo(cx, h - PADDING.bottom);
    ctx.stroke();
    ctx.setLineDash([]);

    // Find nearest values
    let idx = 0;
    for (let i = 1; i < n; i++) {
      if (Math.abs(series.jd[i] - epochJd) < Math.abs(series.jd[idx] - epochJd)) {
        idx = i;
      }
    }
    ctx.fillStyle = "#fff";
    ctx.font = "12px monospace";
    ctx.textAlign = "left";
    ctx.fillText(
      `F10.7=${series.f107[idx].toFixed(1)}  Ap=${series.ap[idx].toFixed(0)}`,
      cx + 6,
      PADDING.top + 14,
    );
  }

  // Attribution
  ctx.fillStyle = "#555";
  ctx.font = "10px sans-serif";
  ctx.textAlign = "right";
  ctx.fillText("Data: GFZ (CC BY 4.0), NOAA SWPC, CelesTrak", w - PADDING.right, h - 4);
}

export function SpaceWeatherChart({ epochJd }: Props) {
  const { containerRef, canvasRef, size } = useCanvasSize();
  const [series, setSeries] = useState<SwSeries | null>(null);
  const seriesRef = useRef<SwSeries | null>(null);

  // Fetch series data once (tab only mounts after sw_ready)
  useEffect(() => {
    let cancelled = false;
    spaceWeatherSeriesAsync().then((data) => {
      if (cancelled || !data || data.length === 0) return;
      const n = data.length / 3;
      const jd = new Float64Array(n);
      const f107 = new Float64Array(n);
      const ap = new Float64Array(n);
      for (let i = 0; i < n; i++) {
        jd[i] = data[i * 3];
        f107[i] = data[i * 3 + 1];
        ap[i] = data[i * 3 + 2];
      }
      const s = { jd, f107, ap };
      seriesRef.current = s;
      setSeries(s);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // Redraw on epoch change or resize (cheap — main thread only)
  useEffect(() => {
    const canvas = canvasRef.current;
    const s = seriesRef.current;
    if (!canvas || !s || size.width <= 0) return;
    renderChart(canvas, s, epochJd);
  }, [epochJd, size, series]);

  return (
    <div
      ref={containerRef}
      style={{
        width: "100%",
        height: "100%",
        display: "flex",
        justifyContent: "center",
        alignItems: "center",
      }}
    >
      <canvas
        ref={canvasRef}
        width={size.width}
        height={size.height}
        style={{ background: "#111", borderRadius: "4px" }}
      />
    </div>
  );
}
