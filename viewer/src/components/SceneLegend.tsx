import { AXIS_COLORS, axisColorCss } from "../axisTriad.js";
import styles from "./SceneLegend.module.css";

/**
 * What the coloured axes in the scene mean.
 *
 * The body axes and the reference-frame axes are both RGB triads and both carry
 * the same X / Y / Z letters, so the letters alone do not say which triad a
 * reader is looking at. The two are listed here, with the frame's row dimmed to
 * match how it is drawn.
 *
 * The direction arrows are not listed: each one carries its own name at its tip,
 * which a reader can match without holding a colour in mind, and which survives
 * a screenshot. The swatches read {@link AXIS_COLORS}, the palette the scene
 * draws the axes and their letters with, so the legend cannot drift from the
 * picture.
 */
export function SceneLegend({ showFrameAxes = true }: { showFrameAxes?: boolean }) {
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
    </div>
  );
}
