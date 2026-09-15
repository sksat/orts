import { type SegmentedOption, SegmentedToggle } from "./SegmentedToggle.js";

/** Which of the app's two presentations is showing. */
export type ViewMode = "orbit" | "attitude";

const OPTIONS: readonly SegmentedOption<ViewMode>[] = [
  { value: "orbit", label: "Orbit", testId: "view-orbit" },
  {
    value: "attitude",
    label: "Attitude",
    testId: "view-attitude",
    title: "One spacecraft's orientation, without the central body or trails",
  },
];

interface ViewSelectorProps {
  view: ViewMode;
  onChange: (view: ViewMode) => void;
}

/**
 * Switch between the orbit and attitude presentations.
 *
 * Rendered by the app rather than by either view's own overlay: switching must
 * not unmount the control doing the switching.
 */
export function ViewSelector({ view, onChange }: ViewSelectorProps) {
  return <SegmentedToggle value={view} options={OPTIONS} onChange={onChange} />;
}
