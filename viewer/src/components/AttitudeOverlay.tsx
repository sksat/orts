import styles from "../App.module.css";
import type { DirectionVectorKind, DirectionVectorOptions } from "../directionVectors.js";
import type { SatelliteInfo } from "../hooks/useWebSocket.js";
import type { AttitudeFrame } from "../lib/types.js";
import { DirectionVectorControls } from "./DirectionVectorControls.js";
import selectorStyles from "./FrameSelector.module.css";
import { SceneLegend } from "./SceneLegend.js";
import { type SegmentedOption, SegmentedToggle } from "./SegmentedToggle.js";

export interface AttitudeOverlayProps {
  /** Spacecraft available to show (from simInfo or replay metadata). */
  satellites: SatelliteInfo[] | undefined;
  selectedSatelliteId: string | null;
  onSelectedSatelliteChange: (id: string) => void;
  orientation: AttitudeFrame;
  onOrientationChange: (orientation: AttitudeFrame) => void;
  /** True when an epoch is known — the body-fixed frame and the Sun need it. */
  hasEpoch: boolean;
  /** True when the shown spacecraft has a position (nadir, local-orbital need it). */
  hasPosition: boolean;
  /** True when it also has a velocity (the local-orbital basis needs it). */
  hasVelocity: boolean;
  /** True when the central body is the one whose rotation the viewer models. */
  supportsBodyFixed: boolean;
  directionVectors: DirectionVectorOptions;
  onDirectionVectorsChange: (value: DirectionVectorOptions) => void;
  /** Kinds actually drawn, for the legend. */
  drawnVectorKinds: readonly DirectionVectorKind[];
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
  hasEpoch,
  hasPosition,
  hasVelocity,
  supportsBodyFixed,
  directionVectors,
  onDirectionVectorsChange,
  drawnVectorKinds,
  hasBody,
}: AttitudeOverlayProps) {
  const orientationOptions: SegmentedOption<AttitudeFrame>[] = [
    { value: "inertial", label: "Inertial", testId: "attitude-orientation-inertial" },
    {
      value: "localOrbital",
      label: "LVLH",
      testId: "attitude-orientation-lvlh",
      disabled: !hasPosition || !hasVelocity,
      title: !hasPosition
        ? "Requires a position"
        : !hasVelocity
          ? "Requires a velocity"
          : undefined,
    },
    {
      value: "bodyFixed",
      label: "Body-Fixed",
      testId: "attitude-orientation-body-fixed",
      disabled: !hasEpoch || !supportsBodyFixed,
      title: !supportsBodyFixed
        ? "The viewer models only Earth's rotation"
        : !hasEpoch
          ? "Requires epoch"
          : undefined,
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
            {(satellites ?? []).map((sat) => (
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
        unavailable={{
          ...(hasEpoch ? {} : { sun: "Requires epoch" }),
          ...(hasPosition ? {} : { nadir: "Requires a position" }),
        }}
      />

      {hasBody && <SceneLegend vectorKinds={drawnVectorKinds} />}

      <div className={styles.orbitInfo} data-testid="attitude-info">
        Attitude view — no central body or trails
      </div>
    </>
  );
}
