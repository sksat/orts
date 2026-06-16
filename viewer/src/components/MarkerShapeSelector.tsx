import type { SatelliteInfo } from "../hooks/useWebSocket.js";
import {
  isMarkerShape,
  MARKER_SHAPE_LABELS,
  MARKER_SHAPES,
  type MarkerShape,
} from "../satelliteShapes.js";
import styles from "./MarkerShapeSelector.module.css";

interface MarkerShapeSelectorProps {
  /** Global default marker shape (null = automatic per attitude). */
  defaultShape: MarkerShape | null;
  onDefaultChange: (shape: MarkerShape | null) => void;
  /** Available satellites for per-satellite overrides (from simInfo). */
  satellites?: SatelliteInfo[];
  /** Per-satellite shape overrides (absent key = follow the default). */
  overrides: Map<string, MarkerShape>;
  onOverrideChange: (satId: string, shape: MarkerShape | null) => void;
}

/** Sentinel <option> value meaning "no explicit choice" (auto / follow default). */
const AUTO = "__auto__";

function parseShapeValue(value: string): MarkerShape | null {
  return value !== AUTO && isMarkerShape(value) ? value : null;
}

/**
 * Marker-shape controls: a global default plus per-satellite overrides. Display
 * concern only — does not affect the simulation. Collapsed by default to keep the
 * scene overlay tidy.
 */
export function MarkerShapeSelector({
  defaultShape,
  onDefaultChange,
  satellites = [],
  overrides,
  onOverrideChange,
}: MarkerShapeSelectorProps) {
  return (
    <details className={styles.shapeSelector} data-testid="marker-shape-selector">
      <summary className={styles.summary}>Markers</summary>
      <div className={styles.row}>
        <label className={styles.label}>Default</label>
        <select
          className={styles.select}
          data-testid="marker-shape-default"
          value={defaultShape ?? AUTO}
          onChange={(e) => onDefaultChange(parseShapeValue(e.target.value))}
        >
          <option value={AUTO}>Auto</option>
          {MARKER_SHAPES.map((s) => (
            <option key={s} value={s}>
              {MARKER_SHAPE_LABELS[s]}
            </option>
          ))}
        </select>
      </div>
      {satellites.map((sat) => (
        <div className={styles.row} key={sat.id}>
          <label className={styles.label} title={sat.id}>
            {sat.name ?? sat.id}
          </label>
          <select
            className={styles.select}
            data-testid={`marker-shape-override-${sat.id}`}
            value={overrides.get(sat.id) ?? AUTO}
            onChange={(e) => onOverrideChange(sat.id, parseShapeValue(e.target.value))}
          >
            <option value={AUTO}>Default</option>
            {MARKER_SHAPES.map((s) => (
              <option key={s} value={s}>
                {MARKER_SHAPE_LABELS[s]}
              </option>
            ))}
          </select>
        </div>
      ))}
    </details>
  );
}
