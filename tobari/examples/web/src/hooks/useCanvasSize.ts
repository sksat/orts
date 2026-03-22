import { useEffect, useRef, useState } from "react";

/** Track parent container size and update canvas dimensions accordingly. */
export function useCanvasSize(padding = 16) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [size, setSize] = useState<{ width: number; height: number }>({ width: 800, height: 500 });

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const update = () => {
      const w = container.clientWidth - padding * 2;
      const h = container.clientHeight - padding * 2;
      if (w > 0 && h > 0) {
        setSize({ width: w, height: h });
        if (canvasRef.current) {
          canvasRef.current.width = w;
          canvasRef.current.height = h;
        }
      }
    };

    update();
    const observer = new ResizeObserver(update);
    observer.observe(container);
    return () => observer.disconnect();
  }, [padding]);

  return { containerRef, canvasRef, size };
}
