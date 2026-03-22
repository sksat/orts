import { useEffect, useRef } from "react";
import { useCanvasSize } from "../hooks/useCanvasSize.js";
import { renderHeatmap } from "../render/heatmap.js";
import type { ViewerParams } from "../types.js";
import { atmosphereLatlonMapAsync } from "../wasm/workerClient.js";

interface Props {
  params: ViewerParams;
}

export function AtmosphereMap({ params }: Props) {
  const { containerRef, canvasRef, size } = useCanvasSize();
  const rangeRef = useRef<{ min: number; max: number } | null>(null);
  const prevModelRef = useRef(params.atmoModel);
  const prevAltRef = useRef(params.altitudeKm);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || size.width <= 0) return;
    let cancelled = false;

    const nLon = params.nLat * 2;
    atmosphereLatlonMapAsync(
      params.atmoModel,
      params.altitudeKm,
      params.epochJd,
      params.nLat,
      nLon,
      params.f107,
      params.ap,
    ).then((data) => {
      if (cancelled || !data) return;

      const modelChanged = params.atmoModel !== prevModelRef.current;
      const altChanged = params.altitudeKm !== prevAltRef.current;
      if (!rangeRef.current || modelChanged || altChanged) {
        let min = Number.POSITIVE_INFINITY;
        let max = Number.NEGATIVE_INFINITY;
        for (let i = 0; i < data.length; i++) {
          const v = data[i];
          if (Number.isFinite(v) && v > 0) {
            if (v < min) min = v;
            if (v > max) max = v;
          }
        }
        rangeRef.current = { min: min * 0.5, max: max * 2 };
        prevModelRef.current = params.atmoModel;
        prevAltRef.current = params.altitudeKm;
      }

      const labelMap: Record<string, string> = {
        exponential: "Exponential",
        "harris-priester": "Harris-Priester",
        nrlmsise00: "NRLMSISE-00",
      };

      renderHeatmap(canvas, {
        nLat: params.nLat,
        nLon,
        data,
        logScale: true,
        min: rangeRef.current!.min,
        max: rangeRef.current!.max,
        label: `${labelMap[params.atmoModel]} — ${params.altitudeKm} km`,
        unit: "kg/m³",
      });
    });

    return () => {
      cancelled = true;
    };
  }, [params, size]);

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
