import { useEffect } from "react";
import { useCanvasSize } from "../hooks/useCanvasSize.js";
import { renderAltitudeProfile } from "../render/altitudeProfile.js";
import { atmosphereAltitudeProfileAsync } from "../wasm/workerClient.js";
import type { ViewerParams } from "../types.js";

interface Props {
  params: ViewerParams;
}

export function AtmosphereProfile({ params }: Props) {
  const { containerRef, canvasRef, size } = useCanvasSize();

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || size.width <= 0) return;
    let cancelled = false;

    const nAlt = 200;
    const altMin = 100;
    const altMax = 1000;
    const altitudes = new Float64Array(nAlt);
    for (let i = 0; i < nAlt; i++) {
      altitudes[i] = altMin + ((altMax - altMin) * i) / (nAlt - 1);
    }

    atmosphereAltitudeProfileAsync(
      altitudes,
      0, 0,
      params.epochJd,
      params.f107,
      params.ap,
    ).then((result) => {
      if (cancelled || !result) return;

      const expValues: number[] = [];
      const hpValues: number[] = [];
      const msisValues: number[] = [];
      for (let i = 0; i < nAlt; i++) {
        expValues.push(result[i * 3]);
        hpValues.push(result[i * 3 + 1]);
        msisValues.push(result[i * 3 + 2]);
      }

      renderAltitudeProfile(canvas, {
        altitudes: Array.from(altitudes),
        curves: [
          { label: "Exponential", color: "#4488ff", values: expValues },
          { label: "Harris-Priester", color: "#ff8844", values: hpValues },
          { label: "NRLMSISE-00", color: "#44dd66", values: msisValues },
        ],
      });
    });

    return () => { cancelled = true; };
  }, [params.epochJd, params.f107, params.ap, size]);

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
