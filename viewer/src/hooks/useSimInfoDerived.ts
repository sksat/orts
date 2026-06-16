import { useMemo } from "react";
import { type DerivedSimInfo, deriveSimInfo } from "../simInfoDerived.js";
import type { SimInfo } from "./useWebSocket.js";

/**
 * Memoised {@link deriveSimInfo}. Recomputes only when `simInfo` identity
 * changes, so the returned `satelliteNames` map stays referentially stable for
 * the scene between renders.
 */
export function useSimInfoDerived(simInfo: SimInfo | null): DerivedSimInfo {
  return useMemo(() => deriveSimInfo(simInfo), [simInfo]);
}
