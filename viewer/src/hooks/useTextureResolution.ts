import { useThree } from "@react-three/fiber";
import { useMemo } from "react";

export type TextureResolution = "2k" | "4k" | "8k" | "16k";

/**
 * Select the best texture resolution based on GPU capabilities and screen size.
 * When `forceMax` is true, use the highest resolution the GPU supports
 * (useful for satellite-centered view where the surface is close).
 * Must be called inside the R3F Canvas tree.
 */
export function useTextureResolution(forceMax = false): TextureResolution {
  const { gl } = useThree();

  return useMemo(() => {
    const maxTexSize = gl.capabilities.maxTextureSize;

    if (forceMax) {
      if (maxTexSize >= 16384) return "16k";
      if (maxTexSize >= 8192) return "8k";
      if (maxTexSize >= 4096) return "4k";
      return "2k";
    }

    const dpr = window.devicePixelRatio ?? 1;
    const screenWidth = window.innerWidth * dpr;

    if (maxTexSize >= 8192 && screenWidth >= 1280) {
      return "8k";
    }
    if (maxTexSize >= 4096 && screenWidth >= 960) {
      return "4k";
    }
    return "2k";
  }, [gl, forceMax]);
}
