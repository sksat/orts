import { AXIS_COLORS, axisColorCss } from "../axisTriad.js";
import {
  DIRECTION_VECTOR_COLORS,
  DIRECTION_VECTOR_LABELS,
  type DirectionVectorKind,
} from "../directionVectors.js";
import { DIRECTION_VECTOR_KINDS } from "./DirectionVectorControls.js";
import styles from "./SceneLegend.module.css";

function hex(color: number): string {
  return `#${color.toString(16).padStart(6, "0")}`;
}

interface SceneLegendProps {
  /** Kinds actually drawn right now; the rest are omitted from the legend. */
  vectorKinds: readonly DirectionVectorKind[];
  /** Whether the reference-frame triad is drawn (it can be turned off). */
  showFrameAxes?: boolean;
}

/**
 * What the coloured lines in the scene mean.
 *
 * The body axes and the reference-frame axes are both RGB triads, so naming
 * "X / Y / Z" once would not tell them apart — the two are listed separately,
 * with the frame's row dimmed to match how it is drawn.
 *
 * The swatches read {@link AXIS_COLORS}, the palette the scene draws the axes
 * and their letters with, so the legend cannot drift from the picture.
 */
export function SceneLegend({ vectorKinds, showFrameAxes = true }: SceneLegendProps) {
  return (
    <div className={styles.legend} data-testid="scene-legend">
      <div className={styles.row}>
        <span className={styles.swatches}>
          {AXIS_COLORS.map((c) => (
            <span key={c} className={styles.swatch} style={{ background: axisColorCss(c) }} />
          ))}
        </span>
        <span className={styles.group}>Body X / Y / Z</span>
      </div>
      {showFrameAxes && (
        <div className={`${styles.row} ${styles.dim}`}>
          <span className={styles.swatches}>
            {AXIS_COLORS.map((c) => (
              <span key={c} className={styles.swatch} style={{ background: axisColorCss(c) }} />
            ))}
          </span>
          <span className={styles.group}>Frame X / Y / Z</span>
        </div>
      )}
      {DIRECTION_VECTOR_KINDS.filter((kind) => vectorKinds.includes(kind)).map((kind) => (
        <div key={kind} className={styles.row}>
          <span className={styles.swatches}>
            <span
              className={styles.swatch}
              style={{ background: hex(DIRECTION_VECTOR_COLORS[kind]) }}
            />
          </span>
          <span>{DIRECTION_VECTOR_LABELS[kind]}</span>
        </div>
      ))}
    </div>
  );
}
