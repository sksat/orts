/**
 * Values the app derives from the simulation metadata (`SimInfo`), with the
 * absent-simInfo defaults resolved in one place.
 *
 * Pure and React-free so it can be unit-tested directly; {@link useSimInfoDerived}
 * wraps it in a `useMemo` for referential stability across renders.
 */
import type { SimInfo } from "./hooks/useWebSocket.js";

export interface DerivedSimInfo {
  /** Central body id (default "earth" when no simInfo). */
  centralBody: string;
  /** Central body radius in km (default Earth equatorial radius). */
  centralBodyRadius: number;
  /** Epoch as Julian Date, or undefined when unset. */
  epochJd: number | undefined;
  /** Per-satellite id → name; undefined when no simInfo. */
  satelliteNames: Map<string, string | null> | undefined;
  /** De-duplicated union of active perturbation names across all satellites. */
  activePerturbations: string[];
}

/** Default central body radius (Earth equatorial radius, km). */
const DEFAULT_CENTRAL_BODY_RADIUS = 6378.137;

export function deriveSimInfo(simInfo: SimInfo | null): DerivedSimInfo {
  if (!simInfo) {
    return {
      centralBody: "earth",
      centralBodyRadius: DEFAULT_CENTRAL_BODY_RADIUS,
      epochJd: undefined,
      satelliteNames: undefined,
      activePerturbations: [],
    };
  }

  const satelliteNames = new Map<string, string | null>();
  const perturbations = new Set<string>();
  for (const satellite of simInfo.satellites) {
    satelliteNames.set(satellite.id, satellite.name);
    for (const p of satellite.perturbations) perturbations.add(p);
  }

  return {
    centralBody: simInfo.central_body,
    centralBodyRadius: simInfo.central_body_radius,
    epochJd: simInfo.epoch_jd ?? undefined,
    satelliteNames,
    activePerturbations: [...perturbations],
  };
}
