import type { TimeRange } from "@sksat/uneri";

/**
 * Read the `timeRange` query parameter from the current URL.
 * Returns the numeric value in seconds, or `null` if absent/invalid.
 */
export function readTimeRangeParam(): TimeRange {
  const params = new URLSearchParams(window.location.search);
  const raw = params.get("timeRange");
  if (raw == null || raw === "" || raw === "all") return null;
  const n = Number(raw);
  if (!Number.isFinite(n) || n <= 0) return null;
  return n;
}

/** Read the `view` query parameter. Anything unrecognised means the orbit view. */
export function readViewParam(): "orbit" | "attitude" {
  const raw = new URLSearchParams(window.location.search).get("view");
  return raw === "attitude" ? "attitude" : "orbit";
}

/**
 * Write the `view` value into the URL query string, so a link or a reload keeps
 * the presentation. Uses `history.replaceState`, like the other view params, so
 * no browser history entry is created.
 */
export function writeViewParam(view: "orbit" | "attitude"): void {
  const params = new URLSearchParams(window.location.search);
  if (view === "orbit") params.delete("view");
  else params.set("view", view);
  const qs = params.toString();
  history.replaceState(
    null,
    "",
    qs ? `${window.location.pathname}?${qs}` : window.location.pathname,
  );
}

/**
 * Write the `timeRange` value into the URL query string.
 * Uses `history.replaceState` so no browser history entry is created.
 * When `timeRange` is `null`, the parameter is removed.
 */
export function writeTimeRangeParam(timeRange: TimeRange): void {
  const params = new URLSearchParams(window.location.search);
  if (timeRange == null) {
    params.delete("timeRange");
  } else {
    params.set("timeRange", String(timeRange));
  }
  const qs = params.toString();
  const url = qs ? `${window.location.pathname}?${qs}` : window.location.pathname;
  history.replaceState(null, "", url);
}
