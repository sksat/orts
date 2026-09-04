import type { CSSProperties } from "react";
import controlStyles from "../styles/controls.module.css";

/** One selectable orientation. */
export interface OrientationOption<T extends string> {
  value: T;
  label: string;
  /** Greyed out and unselectable — the data this orientation needs is missing. */
  disabled?: boolean;
  /** Tooltip, used to say *why* an option is disabled. */
  title?: string;
  /** `data-testid` for E2E. */
  testId?: string;
}

interface OrientationSelectorProps<T extends string> {
  value: T;
  options: readonly OrientationOption<T>[];
  onChange: (value: T) => void;
  style?: CSSProperties;
}

/**
 * A segmented toggle over the display orientations a view offers.
 *
 * Generic over the orientation type so the orbit view (whose orientations pair
 * with a frame centre) and the attitude view (whose spacecraft is always at the
 * origin, leaving only an orientation) get the same control without one view's
 * frame type leaking into the other. Which options exist, and which are
 * available for the current data, is the caller's decision.
 */
export function OrientationSelector<T extends string>({
  value,
  options,
  onChange,
  style,
}: OrientationSelectorProps<T>) {
  return (
    <div className={controlStyles.modeToggle} style={style}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          className={`${controlStyles.modeToggleBtn} ${value === option.value ? controlStyles.active : ""}`}
          data-testid={option.testId}
          onClick={() => onChange(option.value)}
          disabled={option.disabled}
          title={option.title ?? ""}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
