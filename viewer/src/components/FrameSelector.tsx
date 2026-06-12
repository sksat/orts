import type { SatelliteInfo } from "../hooks/useWebSocket.js";
import type { FrameCenter, FrameOrientation, ReferenceFrame } from "../referenceFrame.js";
import controlStyles from "../styles/controls.module.css";
import styles from "./FrameSelector.module.css";

interface FrameSelectorProps {
  referenceFrame: ReferenceFrame;
  onChange: (frame: ReferenceFrame) => void;
  /** Available satellites (from simInfo or replay metadata). */
  satellites?: SatelliteInfo[];
  /** Whether epoch is available (needed for body-fixed frame). */
  hasEpoch?: boolean;
  /** Central body identifier (e.g. "earth"). Used for display labels. */
  centralBody?: string;
}

/** Body-specific orientation labels. Falls back to generic Inertial / Body-Fixed. */
const ORIENTATION_LABELS: Record<string, { inertial: string; body_fixed: string }> = {
  earth: { inertial: "ECI", body_fixed: "ECEF" },
};

/** Encode a FrameCenter to a string key for the <select> value. */
function encodeCenterKey(center: FrameCenter): string {
  if (center.type === "satellite") return `satellite:${center.id}`;
  return center.type;
}

/** Decode a select key back to a FrameCenter. */
function decodeCenterKey(key: string): FrameCenter {
  if (key.startsWith("satellite:")) {
    return { type: "satellite", id: key.slice("satellite:".length) };
  }
  return { type: key } as FrameCenter;
}

/**
 * Frame selection controls: center dropdown + orientation toggle.
 */
export function FrameSelector({
  referenceFrame,
  onChange,
  satellites = [],
  hasEpoch = false,
  centralBody,
}: FrameSelectorProps) {
  const centerKey = encodeCenterKey(referenceFrame.center);
  const isSatCentered = referenceFrame.center.type === "satellite";
  const labels = (centralBody ? ORIENTATION_LABELS[centralBody] : undefined) ?? {
    inertial: "Inertial",
    body_fixed: "Body-Fixed",
  };

  function handleCenterChange(e: React.ChangeEvent<HTMLSelectElement>) {
    const newCenter = decodeCenterKey(e.target.value);
    // Satellite centre defaults to LVLH (the classic "Earth below" view);
    // body_fixed / local_orbital don't carry across centre kinds.
    let newOrientation: FrameOrientation = referenceFrame.orientation;
    if (newCenter.type === "satellite") {
      if (newOrientation === "body_fixed") newOrientation = "local_orbital";
      if (referenceFrame.center.type !== "satellite") newOrientation = "local_orbital";
    } else if (newOrientation === "local_orbital") {
      newOrientation = "inertial";
    }
    onChange({ center: newCenter, orientation: newOrientation });
  }

  function handleOrientationChange(orientation: FrameOrientation) {
    onChange({ center: referenceFrame.center, orientation });
  }

  return (
    <div className={styles.frameSelector}>
      <div className={styles.row}>
        <label className={styles.label}>Center</label>
        <select
          className={styles.select}
          data-testid="frame-selector-select"
          value={centerKey}
          onChange={handleCenterChange}
        >
          <option value="central_body">Central Body</option>
          {satellites.map((sat) => (
            <option key={sat.id} value={`satellite:${sat.id}`}>
              {sat.name ?? sat.id}
            </option>
          ))}
        </select>
      </div>

      <div className={controlStyles.modeToggle} style={{ marginTop: "4px" }}>
        <button
          type="button"
          className={`${controlStyles.modeToggleBtn} ${referenceFrame.orientation === "inertial" ? controlStyles.active : ""}`}
          data-testid="frame-orientation-inertial"
          onClick={() => handleOrientationChange("inertial")}
        >
          {labels.inertial}
        </button>
        {isSatCentered ? (
          <button
            type="button"
            className={`${controlStyles.modeToggleBtn} ${referenceFrame.orientation === "local_orbital" ? controlStyles.active : ""}`}
            data-testid="frame-orientation-lvlh"
            onClick={() => handleOrientationChange("local_orbital")}
          >
            LVLH
          </button>
        ) : (
          <button
            type="button"
            className={`${controlStyles.modeToggleBtn} ${referenceFrame.orientation === "body_fixed" ? controlStyles.active : ""}`}
            data-testid="frame-orientation-body-fixed"
            onClick={() => handleOrientationChange("body_fixed")}
            disabled={!hasEpoch}
            title={!hasEpoch ? "Requires epoch" : ""}
          >
            {labels.body_fixed}
          </button>
        )}
      </div>
    </div>
  );
}
