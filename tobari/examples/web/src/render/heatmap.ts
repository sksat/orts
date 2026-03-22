/**
 * Canvas 2D equirectangular heatmap renderer.
 *
 * Renders a lat/lon grid of scalar values as a colored image.
 */

import { type ColorScale, linearNorm, logNorm, viridis } from "./colorScale.js";

export interface HeatmapOptions {
  /** Number of latitude bins. */
  nLat: number;
  /** Number of longitude bins. */
  nLon: number;
  /** Data values, row-major (south to north, west to east). Length = nLat * nLon. */
  data: Float64Array;
  /** Minimum value for color scale. If undefined, computed from data. */
  min?: number;
  /** Maximum value for color scale. If undefined, computed from data. */
  max?: number;
  /** Use logarithmic color scale. */
  logScale?: boolean;
  /** Color palette. Default: viridis. */
  colorScale?: ColorScale;
  /** Label for the color bar. */
  label?: string;
  /** Unit string for the color bar. */
  unit?: string;
}

/** Render a heatmap to a canvas element. */
export function renderHeatmap(canvas: HTMLCanvasElement, opts: HeatmapOptions): void {
  const { nLat, nLon, data, logScale = false, colorScale = viridis, label, unit } = opts;

  const barWidth = 60;
  const labelHeight = 24;
  const mapWidth = canvas.width - barWidth;
  const mapHeight = canvas.height - labelHeight;

  const ctx = canvas.getContext("2d");
  if (!ctx) return;

  // Compute data range
  let min = opts.min ?? Number.POSITIVE_INFINITY;
  let max = opts.max ?? Number.NEGATIVE_INFINITY;
  if (opts.min === undefined || opts.max === undefined) {
    for (let i = 0; i < data.length; i++) {
      const v = data[i];
      if (!Number.isFinite(v)) continue;
      if (opts.min === undefined && v < min) min = v;
      if (opts.max === undefined && v > max) max = v;
    }
  }

  // Render the map
  const imageData = ctx.createImageData(mapWidth, mapHeight);
  const pixels = imageData.data;
  const norm = logScale ? logNorm : linearNorm;

  for (let py = 0; py < mapHeight; py++) {
    // Flip Y: canvas top = north (+90), data row 0 = south (-90)
    const iLat = Math.floor((1 - py / mapHeight) * nLat);
    const latIdx = Math.max(0, Math.min(nLat - 1, iLat));

    for (let px = 0; px < mapWidth; px++) {
      const iLon = Math.floor((px / mapWidth) * nLon);
      const lonIdx = Math.max(0, Math.min(nLon - 1, iLon));

      const val = data[latIdx * nLon + lonIdx];
      const t = Math.max(0, Math.min(1, norm(val, min, max)));
      const [r, g, b] = colorScale(t);

      const off = (py * mapWidth + px) * 4;
      pixels[off] = r;
      pixels[off + 1] = g;
      pixels[off + 2] = b;
      pixels[off + 3] = 255;
    }
  }

  ctx.putImageData(imageData, 0, 0);

  // Draw color bar
  const barX = mapWidth + 8;
  const barH = mapHeight - 20;
  for (let y = 0; y < barH; y++) {
    const t = 1 - y / barH;
    const [r, g, b] = colorScale(t);
    ctx.fillStyle = `rgb(${r},${g},${b})`;
    ctx.fillRect(barX, y + 10, 20, 1);
  }

  // Color bar labels
  ctx.fillStyle = "#aaa";
  ctx.font = "11px monospace";
  ctx.textAlign = "left";

  const fmt = (v: number) => {
    if (logScale && v > 0) return v.toExponential(1);
    if (Math.abs(v) >= 1000) return v.toFixed(0);
    if (Math.abs(v) >= 1) return v.toFixed(1);
    return v.toExponential(1);
  };

  ctx.fillText(fmt(max), barX + 22, 18);
  ctx.fillText(fmt(min), barX + 22, barH + 8);

  // Label
  if (label) {
    ctx.fillStyle = "#ccc";
    ctx.font = "12px sans-serif";
    ctx.textAlign = "center";
    ctx.fillText(`${label}${unit ? ` [${unit}]` : ""}`, mapWidth / 2, mapHeight + 16);
  }
}
