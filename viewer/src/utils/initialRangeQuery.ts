import type { TimeRange } from "uneri";
import type { SimInfo } from "../hooks/useWebSocket.js";

/**
 * Parameters describing a `query_range` request the viewer should fire
 * immediately after receiving the initial history overview, to fill in
 * higher-resolution data for the user's current display window.
 */
export interface InitialRangeQuery {
  satId: string;
  tMin: number;
  tMax: number;
  maxPoints: number;
}

export interface PlanInitialRangeQueryInput {
  simInfo: SimInfo | null;
  timeRange: TimeRange;
  /**
   * Most recent sim time known to the viewer, typically the `t` of the
   * last point in any trail buffer. Used as the upper bound of the
   * range query and as the anchor for computing the window.
   */
  latestT: number;
  /**
   * True if we have already fired an initial range query for the current
   * WebSocket connection. Prevents duplicate requests on subsequent
   * history-event arrivals within the same session.
   */
  alreadyQueried: boolean;
}

/**
 * Decide whether the viewer should proactively fire `query_range`
 * requests after (re)connecting, and if so, what parameters to use for
 * each satellite.
 *
 * The contract with the server is: on connect, the server ships a small
 * bounded overview of the full simulation history. That is fast to
 * transfer and render, but too sparse for detailed chart zoom within a
 * finite display window. When the user has a finite `timeRange` selected
 * (e.g. "1 hour"), this helper plans one pull request **per satellite**
 * for higher-resolution data within that window. In "All" mode
 * (`timeRange = null`) the overview is considered sufficient and this
 * returns an empty array.
 *
 * Returns an empty array when no query should be fired.
 */
export function planInitialRangeQuery(input: PlanInitialRangeQueryInput): InitialRangeQuery[] {
  const { simInfo, timeRange, latestT, alreadyQueried } = input;

  if (alreadyQueried) return [];
  if (!simInfo) return [];
  // "All" mode: the server overview is intended to be the full view; no
  // proactive enrichment. Follow-up detail still flows via handleChartZoom.
  if (timeRange == null) return [];
  // No history has arrived yet — nothing to anchor the window on.
  if (latestT <= 0) return [];
  if (simInfo.satellites.length === 0) return [];

  const tMin = Math.max(0, latestT - timeRange);
  const tMax = latestT;
  // 2000 is the same density the chart zoom path requests — enough to
  // render a smooth line across a 1h window at 1.8s resolution.
  const maxPoints = 2000;

  return simInfo.satellites.map((sat) => ({
    satId: sat.id,
    tMin,
    tMax,
    maxPoints,
  }));
}
