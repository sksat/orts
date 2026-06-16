import styles from "../App.module.css";
import type { SatelliteInfo, SimInfo } from "../hooks/useWebSocket.js";
import type { ReferenceFrame } from "../referenceFrame.js";
import { FrameSelector } from "./FrameSelector.js";
import { SimInfoBar } from "./SimInfoBar.js";

export interface SceneOverlayProps {
  referenceFrame: ReferenceFrame;
  onReferenceFrameChange: (frame: ReferenceFrame) => void;
  /** Satellites available for the frame selector (from simInfo or replay). */
  satellites: SatelliteInfo[] | undefined;
  centralBody: string;
  epochJd: number | undefined;
  /** File-source orbit summary line; empty string when no file is loaded. */
  orbitInfo: string;
  /** Live simulation metadata; null hides the info bar. */
  simInfo: SimInfo | null;
  totalPoints: number;
  activePerturbations: string[];
}

/**
 * Top-left canvas overlay: frame selector, optional file-orbit summary, and the
 * simulation info bar. Grouped out of App so the orchestrator stays focused on
 * data flow rather than canvas chrome layout.
 */
export function SceneOverlay({
  referenceFrame,
  onReferenceFrameChange,
  satellites,
  centralBody,
  epochJd,
  orbitInfo,
  simInfo,
  totalPoints,
  activePerturbations,
}: SceneOverlayProps) {
  return (
    <div className={styles.sceneOverlay}>
      <FrameSelector
        referenceFrame={referenceFrame}
        onChange={onReferenceFrameChange}
        satellites={satellites}
        hasEpoch={epochJd != null}
        centralBody={centralBody}
      />
      {orbitInfo && (
        <div className={styles.orbitInfo} data-testid="orbit-info-file">
          {orbitInfo}
        </div>
      )}
      {simInfo && (
        <SimInfoBar
          simInfo={simInfo}
          totalPoints={totalPoints}
          epochJd={epochJd}
          activePerturbations={activePerturbations}
        />
      )}
    </div>
  );
}
