import { useEffect, useRef } from "react";
import { useCanvasSize } from "../hooks/useCanvasSize.js";
import { renderHeatmap } from "../render/heatmap.js";
import { diverging, viridis } from "../render/colorScale.js";
import { magneticFieldLatlonMapAsync } from "../wasm/workerClient.js";
import type { ViewerParams } from "../types.js";

interface Props {
  params: ViewerParams;
}

export function MagneticFieldMap({ params }: Props) {
  const { containerRef, canvasRef, size } = useCanvasSize();
  const rangeRef = useRef<{ min: number; max: number } | null>(null);
  const prevKeyRef = useRef("");

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || size.width <= 0) return;
    let cancelled = false;

    const nLon = params.nLat * 2;
    magneticFieldLatlonMapAsync(
      params.magModel,
      params.fieldComponent,
      params.altitudeKm,
      params.epochJd,
      params.nLat,
      nLon,
    ).then((data) => {
      if (cancelled || !data) return;

      const isDiverging =
        params.fieldComponent === "inclination" ||
        params.fieldComponent === "declination" ||
        params.fieldComponent === "north" ||
        params.fieldComponent === "east" ||
        params.fieldComponent === "down";

      const unitMap: Record<string, string> = {
        total: "nT", inclination: "°", declination: "°",
        north: "nT", east: "nT", down: "nT",
      };
      const labelMap: Record<string, string> = {
        total: "Total Intensity", inclination: "Inclination",
        declination: "Declination", north: "North Component",
        east: "East Component", down: "Down Component",
      };

      const key = `${params.magModel}:${params.fieldComponent}:${params.altitudeKm}`;
      if (!rangeRef.current || key !== prevKeyRef.current) {
        if (isDiverging) {
          let maxAbs = 0;
          for (let i = 0; i < data.length; i++) {
            const abs = Math.abs(data[i]);
            if (Number.isFinite(abs) && abs > maxAbs) maxAbs = abs;
          }
          rangeRef.current = { min: -maxAbs, max: maxAbs };
        } else {
          let min = Number.POSITIVE_INFINITY;
          let max = Number.NEGATIVE_INFINITY;
          for (let i = 0; i < data.length; i++) {
            if (Number.isFinite(data[i])) {
              if (data[i] < min) min = data[i];
              if (data[i] > max) max = data[i];
            }
          }
          rangeRef.current = { min, max };
        }
        prevKeyRef.current = key;
      }

      renderHeatmap(canvas, {
        nLat: params.nLat,
        nLon,
        data,
        min: rangeRef.current.min,
        max: rangeRef.current.max,
        colorScale: isDiverging ? diverging : viridis,
        label: `${params.magModel.toUpperCase()} — ${labelMap[params.fieldComponent]}`,
        unit: unitMap[params.fieldComponent],
      });
    });

    return () => { cancelled = true; };
  }, [params, size]);

  return (
    <div ref={containerRef} style={{ width: "100%", height: "100%", display: "flex", justifyContent: "center", alignItems: "center" }}>
      <canvas
        ref={canvasRef}
        width={size.width}
        height={size.height}
        style={{ background: "#111", borderRadius: "4px" }}
      />
    </div>
  );
}
