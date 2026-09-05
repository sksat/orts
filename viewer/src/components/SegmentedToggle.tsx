import type { CSSProperties } from "react";
import controlStyles from "../styles/controls.module.css";

/** One selectable segment. */
export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
  /** Greyed out and unselectable — the data this option needs is missing. */
  disabled?: boolean;
  /** Tooltip, used to say *why* an option is disabled. */
  title?: string;
  /** `data-testid` for E2E. */
  testId?: string;
}

interface SegmentedToggleProps<T extends string> {
  value: T;
  options: readonly SegmentedOption<T>[];
  onChange: (value: T) => void;
  style?: CSSProperties;
}

/**
 * A one-of-N toggle: the app's segmented control.
 *
 * Generic over the value type so every such choice looks the same without one
 * caller's domain type leaking into another — the display orientations (whose
 * meaning differs between the orbit and attitude views) and the view switch
 * itself both use it. Which options exist, and which are available for the
 * current data, is the caller's decision.
 */
export function SegmentedToggle<T extends string>({
  value,
  options,
  onChange,
  style,
}: SegmentedToggleProps<T>) {
  return (
    <div className={controlStyles.modeToggle} style={style}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          className={`${controlStyles.modeToggleBtn} ${value === option.value ? controlStyles.active : ""}`}
          data-testid={option.testId}
          // Which segment is selected is otherwise only in the styling.
          aria-pressed={value === option.value}
          // See `DirectionVectorControls`: a `disabled` button shows no tooltip
          // and takes no focus, and an unavailable option here carries the reason
          // it is unavailable.
          aria-disabled={option.disabled}
          title={option.title}
          onClick={() => {
            if (option.disabled) return;
            onChange(option.value);
          }}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
