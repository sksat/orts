import {
  DIRECTION_VECTOR_COLORS,
  DIRECTION_VECTOR_LABELS,
  type DirectionVectorKind,
  type DirectionVectorOptions,
} from "../directionVectors.js";
import controlStyles from "../styles/controls.module.css";

/** Order the toggles appear in. */
const DIRECTION_VECTOR_KINDS: readonly DirectionVectorKind[] = ["sun", "nadir"];

interface DirectionVectorControlsProps {
  value: DirectionVectorOptions;
  onChange: (value: DirectionVectorOptions) => void;
  /**
   * Kinds whose input the current data lacks — greyed out with the reason, rather
   * than silently drawing nothing. Keeping the choice while disabling it means a
   * viewer's selection survives a frame change that made it unavailable.
   */
  unavailable?: Partial<Record<DirectionVectorKind, string | undefined>>;
}

/**
 * Independent on/off toggles for the reference-direction arrows.
 *
 * Not a {@link SegmentedToggle}: the arrows are not alternatives, any subset can
 * be on. Shares the toggle styling so it reads as the same family of control.
 */
export function DirectionVectorControls({
  value,
  onChange,
  unavailable,
}: DirectionVectorControlsProps) {
  return (
    <div className={controlStyles.modeToggle}>
      {DIRECTION_VECTOR_KINDS.map((kind) => {
        const reason = unavailable?.[kind];
        const on = value[kind] !== false;
        return (
          <button
            key={kind}
            type="button"
            className={`${controlStyles.modeToggleBtn} ${on ? controlStyles.active : ""}`}
            data-testid={`direction-vector-${kind}`}
            // The on/off state is otherwise only in the styling, so a screen
            // reader would announce an identical button either way.
            aria-pressed={on}
            // `aria-disabled` rather than `disabled`: a disabled button takes no
            // mouse events, so the reason in its `title` is never shown, and it
            // leaves the tab order, so a keyboard user cannot reach it either.
            // The reason is the whole point of the state here.
            aria-disabled={reason != null}
            // `title` renders as a tooltip, which needs a pointer to hover and
            // which screen readers treat as an optional description. Folding the
            // reason into the accessible name puts it where a name is always
            // announced. The visible label leads it, so speech input still
            // matches what is on screen.
            aria-label={reason == null ? undefined : `${DIRECTION_VECTOR_LABELS[kind]}: ${reason}`}
            title={reason ?? `Draw the ${DIRECTION_VECTOR_LABELS[kind]} direction`}
            onClick={() => {
              if (reason != null) return;
              onChange({ ...value, [kind]: !on });
            }}
          >
            <span
              aria-hidden="true"
              style={{
                display: "inline-block",
                width: "8px",
                height: "8px",
                marginRight: "6px",
                borderRadius: "50%",
                background: `#${DIRECTION_VECTOR_COLORS[kind].toString(16).padStart(6, "0")}`,
              }}
            />
            {DIRECTION_VECTOR_LABELS[kind]}
          </button>
        );
      })}
    </div>
  );
}
