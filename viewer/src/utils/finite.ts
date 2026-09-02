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
