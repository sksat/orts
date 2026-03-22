/**
 * Canvas 2D altitude profile chart renderer.
 *
 * Draws atmospheric density vs altitude with logarithmic Y-axis.
 */

export interface ProfileCurve {
  label: string;
  color: string;
  /** Density values [kg/m³], one per altitude step. */
  values: number[];
}

export interface ProfileOptions {
  /** Altitude values [km]. */
  altitudes: number[];
  /** Curves to draw. */
  curves: ProfileCurve[];
}

const PADDING = { top: 20, right: 20, bottom: 40, left: 80 };

export function renderAltitudeProfile(canvas: HTMLCanvasElement, opts: ProfileOptions): void {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;

  const w = canvas.width;
  const h = canvas.height;
  const plotW = w - PADDING.left - PADDING.right;
  const plotH = h - PADDING.top - PADDING.bottom;

  ctx.clearRect(0, 0, w, h);

  // Find data range
  let minDensity = Number.POSITIVE_INFINITY;
  let maxDensity = Number.NEGATIVE_INFINITY;
  for (const curve of opts.curves) {
    for (const v of curve.values) {
      if (v > 0 && v < minDensity) minDensity = v;
      if (v > maxDensity) maxDensity = v;
    }
  }
  if (minDensity >= maxDensity) return;

  // Round to nice log powers
  const logMin = Math.floor(Math.log10(minDensity));
  const logMax = Math.ceil(Math.log10(maxDensity));

  const altMin = opts.altitudes[0];
  const altMax = opts.altitudes[opts.altitudes.length - 1];

  const toX = (alt: number) => PADDING.left + ((alt - altMin) / (altMax - altMin)) * plotW;
  const toY = (rho: number) => {
    if (rho <= 0) return PADDING.top + plotH;
    const t = (Math.log10(rho) - logMin) / (logMax - logMin);
    return PADDING.top + plotH * (1 - t);
  };

  // Grid
  ctx.strokeStyle = "#333";
  ctx.lineWidth = 0.5;
  ctx.setLineDash([4, 4]);

  // Y grid (log decades)
  ctx.font = "11px monospace";
  ctx.fillStyle = "#888";
  ctx.textAlign = "right";
  for (let p = logMin; p <= logMax; p++) {
    const y = toY(10 ** p);
    ctx.beginPath();
    ctx.moveTo(PADDING.left, y);
    ctx.lineTo(w - PADDING.right, y);
    ctx.stroke();
    ctx.fillText(`1e${p}`, PADDING.left - 6, y + 4);
  }

  // X grid
  ctx.textAlign = "center";
  const xStep = altMax - altMin > 500 ? 200 : 100;
  for (let alt = Math.ceil(altMin / xStep) * xStep; alt <= altMax; alt += xStep) {
    const x = toX(alt);
    ctx.beginPath();
    ctx.moveTo(x, PADDING.top);
    ctx.lineTo(x, PADDING.top + plotH);
    ctx.stroke();
    ctx.fillText(`${alt}`, x, h - PADDING.bottom + 16);
  }

  ctx.setLineDash([]);

  // Axes
  ctx.strokeStyle = "#555";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(PADDING.left, PADDING.top);
  ctx.lineTo(PADDING.left, PADDING.top + plotH);
  ctx.lineTo(w - PADDING.right, PADDING.top + plotH);
  ctx.stroke();

  // Curves
  for (const curve of opts.curves) {
    ctx.strokeStyle = curve.color;
    ctx.lineWidth = 2;
    ctx.beginPath();
    let started = false;
    for (let i = 0; i < opts.altitudes.length; i++) {
      const v = curve.values[i];
      if (v <= 0) continue;
      const x = toX(opts.altitudes[i]);
      const y = toY(v);
      if (!started) {
        ctx.moveTo(x, y);
        started = true;
      } else {
        ctx.lineTo(x, y);
      }
    }
    ctx.stroke();
  }

  // Legend
  const legendX = PADDING.left + 10;
  let legendY = PADDING.top + 16;
  ctx.font = "12px sans-serif";
  for (const curve of opts.curves) {
    ctx.fillStyle = curve.color;
    ctx.fillRect(legendX, legendY - 8, 16, 3);
    ctx.fillStyle = "#ccc";
    ctx.textAlign = "left";
    ctx.fillText(curve.label, legendX + 22, legendY);
    legendY += 18;
  }

  // Axis labels
  ctx.fillStyle = "#aaa";
  ctx.font = "12px sans-serif";
  ctx.textAlign = "center";
  ctx.fillText("Altitude [km]", PADDING.left + plotW / 2, h - 4);

  ctx.save();
  ctx.translate(14, PADDING.top + plotH / 2);
  ctx.rotate(-Math.PI / 2);
  ctx.fillText("Density [kg/m³]", 0, 0);
  ctx.restore();
}
