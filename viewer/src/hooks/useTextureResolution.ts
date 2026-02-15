import { useThree } from "@react-three/fiber";
import { useMemo } from "react";

export type TextureResolution = "2k" | "4k" | "8k";

/**
 * Select the best texture resolution based on GPU capabilities and screen size.
 * Must be called inside the R3F Canvas tree.
 */
export function useTextureResolution(): TextureResolution {
  const { gl } = useThree();

  return useMemo(() => {
    const maxTexSize = gl.capabilities.maxTextureSize;
    const dpr = window.devicePixelRatio ?? 1;
    const screenWidth = window.innerWidth * dpr;

    if (maxTexSize >= 8192 && screenWidth >= 2560) {
      return "8k";
    }
    if (maxTexSize >= 4096 && screenWidth >= 1280) {
      return "4k";
    }
    return "2k";
  }, [gl]);
}
