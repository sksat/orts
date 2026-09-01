import { useEffect, useState } from "react";
import { initArika, isArikaReady } from "./arikaInit.js";

/**
 * Drive the bundled arika WASM to readiness when `enabled`. Returns `true` once
 * loaded so a scene can switch from its epoch-less defaults to ephemeris-accurate
 * Sun direction and body rotation. When `enabled` is false (no epoch), the WASM is
 * never loaded — epoch-less embedders pay no network/init cost.
 *
 * A failed load leaves the return value false: the scene keeps rendering without
 * the features that need an ephemeris.
 */
export function useArikaReady(enabled: boolean): boolean {
  const [ready, setReady] = useState(() => isArikaReady());
  useEffect(() => {
    if (!enabled || ready) return;
    let cancelled = false;
    initArika()
      .then(() => {
        if (!cancelled) setReady(true);
      })
      .catch(() => {
        // Leave `ready` false: the caller falls back to its epoch-less defaults.
      });
    return () => {
      cancelled = true;
    };
  }, [enabled, ready]);
  return ready;
}
