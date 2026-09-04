import { AXIS_COLORS, AXIS_DIRECTIONS, AXIS_LETTERS } from "../axisTriad.js";
import type { ArrowGeometry } from "../spacecraftScale.js";
import { Arrow } from "./Arrow.js";
import { AxisLabels } from "./AxisLabels.js";

interface AxisTriadProps {
  /** Extent and thickness, from `bodyAxisArrows` / `frameAxisArrows`. */
  geometry: ArrowGeometry;
  /** Below 1 the whole triad is drawn transparent. */
  opacity?: number;
  /** Draw X / Y / Z at the tips. */
  labels?: boolean;
}

/**
 * An RGB axis triad drawn as arrows.
 *
 * Replaces `AxesHelper` where the triad is something to read rather than a debug
 * aid: a helper's axes are one-pixel GL lines at any distance, so they thin out
 * to near-invisibility exactly when the spacecraft is framed to be looked at. The
 * arrows also carry heads, which say which end of each axis is positive.
 */
export function AxisTriad({ geometry, opacity = 1, labels = false }: AxisTriadProps) {
  return (
    <>
      {AXIS_LETTERS.map((letter, i) => (
        <Arrow
          key={letter}
          direction={AXIS_DIRECTIONS[i]}
          color={AXIS_COLORS[i]}
          opacity={opacity}
          {...geometry}
        />
      ))}
      {labels && <AxisLabels length={geometry.length} opacity={opacity} />}
    </>
  );
}
