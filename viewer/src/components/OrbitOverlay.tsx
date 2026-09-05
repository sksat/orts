import styles from "../App.module.css";
import type { DirectionVectorKind, DirectionVectorOptions } from "../directionVectors.js";
import type { SatelliteInfo, SimInfo } from "../hooks/useWebSocket.js";
import type { ReferenceFrame } from "../referenceFrame.js";
import type { MarkerShape } from "../satelliteShapes.js";
import { DirectionVectorControls } from "./DirectionVectorControls.js";
import { FrameSelector } from "./FrameSelector.js";
import { MarkerShapeSelector } from "./MarkerShapeSelector.js";
import { SimInfoBar } from "./SimInfoBar.js";

export interface OrbitOverlayProps {
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
  /** Global default marker shape (null = automatic). */
  defaultMarkerShape: MarkerShape | null;
  onDefaultMarkerShapeChange: (shape: MarkerShape | null) => void;
  /** Per-satellite marker shape overrides. */
  markerShapeOverrides: Map<string, MarkerShape>;
  onMarkerShapeOverride: (satId: string, shape: MarkerShape | null) => void;
  /**
   * Reference-direction arrows. The same setting the attitude view uses, offered
   * here too so a change made in one view is visible in the other rather than
   * taking effect invisibly.
   */
  directionVectors: DirectionVectorOptions;
  onDirectionVectorsChange: (value: DirectionVectorOptions) => void;
  /** The centred satellite, or null in a central-body view (no arrows there). */
  centredSatelliteId: string | null;
  /**
   * Which arrows the scene could draw for that satellite, as its own resolver
   * answers. Not "does it have a position": a position that is present but zero
   * or non-finite yields no nadir, and a control offering an arrow the scene then
   * drops is lying.
   */
  drawableVectorKinds: readonly DirectionVectorKind[];
  /**
   * Whether the scene can place the centred spacecraft at all (default true).
   *
   * False takes the whole scene with it — no arrow is drawn at an unplaceable
   * centre, the Sun included — so both toggles then give that one reason. Comes
   * from the app because only it holds the centred spacecraft's position.
   */
  centreIsPlaceable?: boolean;
  /**
   * Why the Sun is unavailable, when it is. The app owns the wording because it
   * knows which of the two reasons applies — no epoch, or a central body arika
   * cannot place.
   */
  sunUnavailable?: string;
}

/**
 * The orbit view's overlay controls: frame selector, marker shapes, the optional
 * file-orbit summary and the simulation info bar.
 *
 * A fragment, not a positioned container: the app owns the overlay area so the
 * view switch — which must outlive either view — sits alongside this rather than
 * inside it.
 */
export function OrbitOverlay({
  referenceFrame,
  onReferenceFrameChange,
  satellites,
  centralBody,
  epochJd,
  orbitInfo,
  simInfo,
  totalPoints,
  activePerturbations,
  defaultMarkerShape,
  onDefaultMarkerShapeChange,
  markerShapeOverrides,
  onMarkerShapeOverride,
  directionVectors,
  onDirectionVectorsChange,
  centredSatelliteId,
  drawableVectorKinds,
  sunUnavailable,
  centreIsPlaceable = true,
}: OrbitOverlayProps) {
  const noCentre = centredSatelliteId == null ? "Centre on a satellite to draw it" : undefined;
  /**
   * Why nothing can be drawn at the centre, when that is the reason.
   *
   * A centre the frame cannot place takes the whole scene with it: no arrow is
   * drawn there, the Sun included, though it needs no position of its own. So
   * both toggles give this reason and neither is left enabled over a scene that
   * draws nothing. Asked of the frame's own predicate, which refuses a non-finite
   * position and accepts every other — the coordinate origin among them.
   */
  const unplaceableCentre = centreIsPlaceable ? undefined : "Requires a finite position";
  /**
   * Nadir's own condition. It is the bearing from the spacecraft to the body,
   * which a spacecraft at the body's centre does not have — and the scene draws
   * everything else there, so this reason belongs to nadir alone.
   */
  const noBearing = "Requires a non-zero position";
  return (
    <>
      <FrameSelector
        referenceFrame={referenceFrame}
        onChange={onReferenceFrameChange}
        satellites={satellites}
        hasEpoch={epochJd != null}
        centralBody={centralBody}
      />
      <MarkerShapeSelector
        defaultShape={defaultMarkerShape}
        onDefaultChange={onDefaultMarkerShapeChange}
        satellites={satellites}
        overrides={markerShapeOverrides}
        onOverrideChange={onMarkerShapeOverride}
      />
      <DirectionVectorControls
        value={directionVectors}
        onChange={onDirectionVectorsChange}
        unavailable={{
          sun:
            noCentre ??
            unplaceableCentre ??
            (drawableVectorKinds.includes("sun") ? undefined : sunUnavailable),
          nadir:
            noCentre ??
            unplaceableCentre ??
            (drawableVectorKinds.includes("nadir") ? undefined : noBearing),
        }}
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
    </>
  );
}
