import styles from "../App.module.css";
import type { DirectionVectorOptions } from "../directionVectors.js";
import type { AttitudeFrame } from "../lib/types.js";
import { DirectionVectorControls } from "./DirectionVectorControls.js";
import selectorStyles from "./FrameSelector.module.css";
import { SceneLegend } from "./SceneLegend.js";
import { type SegmentedOption, SegmentedToggle } from "./SegmentedToggle.js";

/** A spacecraft this view can show. */
export interface AttitudeSubject {
  id: string;
  name?: string;
}

export interface AttitudeOverlayProps {
  /**
   * Spacecraft this view can show — the ones actually being rendered, not the
   * ones the simulation declared. Offering a spacecraft whose state has not
   * arrived would leave the select with no matching option.
   */
  satellites: readonly AttitudeSubject[];
  selectedSatelliteId: string | null;
  onSelectedSatelliteChange: (id: string) => void;
  orientation: AttitudeFrame;
  onOrientationChange: (orientation: AttitudeFrame) => void;
  /**
   * Why an option cannot be used right now, or undefined when it can. The reasons
   * come from the app rather than being re-derived here: whether the scene can
   * actually build a local-orbital basis, or draw a nadir arrow, is a question for
   * the same code the scene resolves with — a control that offers what the scene
   * then drops is lying.
   */
  localOrbitalUnavailable?: string;
  bodyFixedUnavailable?: string;
  sunUnavailable?: string;
  nadirUnavailable?: string;
  directionVectors: DirectionVectorOptions;
  onDirectionVectorsChange: (value: DirectionVectorOptions) => void;
  /**
   * Whether a spacecraft is being drawn at all. Without one the scene is empty,
   * so the legend would name lines that are not there.
   */
  hasBody: boolean;
}

/**
 * The attitude view's overlay controls: which spacecraft, which display
 * orientation, which reference-direction arrows, and what the colours mean.
 *
 * A fragment, like the orbit view's overlay: the app owns the overlay area.
 *
 * Every option whose input the current data lacks is disabled with the reason
 * rather than left selectable — the scene falls back safely, but a control that
 * says "LVLH" while showing inertial axes would be lying.
 */
export function AttitudeOverlay({
  satellites,
  selectedSatelliteId,
  onSelectedSatelliteChange,
  orientation,
  onOrientationChange,
  localOrbitalUnavailable,
  bodyFixedUnavailable,
  sunUnavailable,
  nadirUnavailable,
  directionVectors,
  onDirectionVectorsChange,
  hasBody,
}: AttitudeOverlayProps) {
  const orientationOptions: SegmentedOption<AttitudeFrame>[] = [
    { value: "inertial", label: "Inertial", testId: "attitude-orientation-inertial" },
    {
      value: "localOrbital",
      label: "LVLH",
      testId: "attitude-orientation-lvlh",
      disabled: localOrbitalUnavailable != null,
      title: localOrbitalUnavailable,
    },
    {
      value: "bodyFixed",
      label: "Body-Fixed",
      testId: "attitude-orientation-body-fixed",
      disabled: bodyFixedUnavailable != null,
      title: bodyFixedUnavailable,
    },
  ];

  return (
    <>
      <div className={selectorStyles.frameSelector}>
        <div className={selectorStyles.row}>
          <label className={selectorStyles.label} htmlFor="attitude-spacecraft">
            Spacecraft
          </label>
          <select
            id="attitude-spacecraft"
            className={selectorStyles.select}
            data-testid="attitude-spacecraft-select"
            value={selectedSatelliteId ?? ""}
            onChange={(e) => onSelectedSatelliteChange(e.target.value)}
          >
            {satellites.map((sat) => (
              <option key={sat.id} value={sat.id}>
                {sat.name ?? sat.id}
              </option>
            ))}
          </select>
        </div>
        <SegmentedToggle
          value={orientation}
          options={orientationOptions}
          onChange={onOrientationChange}
          style={{ marginTop: "4px" }}
        />
      </div>

      <DirectionVectorControls
        value={directionVectors}
        onChange={onDirectionVectorsChange}
        unavailable={{ sun: sunUnavailable, nadir: nadirUnavailable }}
      />

      {hasBody && <SceneLegend />}

      <div className={styles.orbitInfo} data-testid="attitude-info">
        Attitude view — no central body or trails
      </div>
    </>
  );
}
