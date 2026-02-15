import type { SatelliteInfo } from "../hooks/useWebSocket.js";
import {
  type ReferenceFrame,
  type FrameCenter,
  type FrameOrientation,
  frameCenterEquals,
} from "../referenceFrame.js";

interface FrameSelectorProps {
  referenceFrame: ReferenceFrame;
  onChange: (frame: ReferenceFrame) => void;
  /** Available satellites (from simInfo or replay metadata). */
  satellites?: SatelliteInfo[];
  /** Whether epoch is available (needed for body-fixed frame). */
  hasEpoch?: boolean;
}

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
}: FrameSelectorProps) {
  const centerKey = encodeCenterKey(referenceFrame.center);
  const isSatCentered = referenceFrame.center.type === "satellite";

  function handleCenterChange(e: React.ChangeEvent<HTMLSelectElement>) {
    const newCenter = decodeCenterKey(e.target.value);
    // Reset to inertial when switching to satellite (body_fixed not supported)
    const newOrientation: FrameOrientation =
      newCenter.type === "satellite" ? "inertial" : referenceFrame.orientation;
    onChange({ center: newCenter, orientation: newOrientation });
  }

  function handleOrientationChange(orientation: FrameOrientation) {
    onChange({ center: referenceFrame.center, orientation });
  }

  return (
    <div className="frame-selector">
      <div className="frame-selector-row">
        <label className="frame-selector-label">Center</label>
        <select
          className="frame-selector-select"
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

      <div className="mode-toggle" style={{ marginTop: "4px" }}>
        <button
          className={`mode-toggle-btn ${referenceFrame.orientation === "inertial" ? "active" : ""}`}
          onClick={() => handleOrientationChange("inertial")}
        >
          Inertial
        </button>
        <button
          className={`mode-toggle-btn ${referenceFrame.orientation === "body_fixed" ? "active" : ""}`}
          onClick={() => handleOrientationChange("body_fixed")}
          disabled={isSatCentered || !hasEpoch}
          title={isSatCentered ? "Body-fixed not available for satellite center" : !hasEpoch ? "Requires epoch" : ""}
        >
          Body-Fixed
        </button>
      </div>
    </div>
  );
}
