/**
 * Numbers arriving from a public prop, sanitised at the boundary.
 *
 * An embedder can hand a scene `NaN` or `±Infinity` — a parse that failed
 * upstream, a division by a zero timestep — and the value then spreads: a
 * non-finite epoch reaches the Sun direction and the light's position, and a
 * non-finite time reaches every quantised sample. Rejecting it once, where it
 * enters, keeps the scene at its documented fallback instead.
 */

/** The value if it is a finite number, else null. Zero passes through. */
export function finiteOrNull(value: number | null | undefined): number | null {
  return value != null && Number.isFinite(value) ? value : null;
}

/**
 * The first finite `t` among these samples, or null if none has one.
 *
 * Scanning for a usable time rather than for the first sample that exists: a
 * satellite whose `t` is `NaN` would otherwise end the search and leave the
 * caller with no time at all, while a later satellite carries a good one.
 */
export function firstFiniteTime(
  samples: Iterable<{ t?: number } | null | undefined> | null | undefined,
): number | null {
  if (samples == null) return null;
  for (const sample of samples) {
    const t = finiteOrNull(sample?.t);
    if (t != null) return t;
  }
  return null;
}
