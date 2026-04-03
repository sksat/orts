import type { SimInfo } from "../hooks/useWebSocket.js";
import { jd_to_utc_string } from "../wasm/kanameInit.js";
import styles from "./SimInfoBar.module.css";

interface SimInfoBarProps {
  simInfo: SimInfo;
  totalPoints: number;
  epochJd?: number;
  activePerturbations: string[];
}

export function SimInfoBar({
  simInfo,
  totalPoints,
  epochJd,
  activePerturbations,
}: SimInfoBarProps) {
  const satNames = simInfo.satellites.map((sat) => sat.name ?? sat.id).join(" | ");

  return (
    <>
      <div className={styles.infoBar} data-testid="orbit-info-sim">
        {satNames && (
          <>
            <strong>{satNames}</strong> |{" "}
          </>
        )}
        {epochJd != null && <>{jd_to_utc_string(epochJd, 0)} | </>}
        mu={simInfo.mu.toFixed(2)} km^3/s^2 | dt={simInfo.dt.toFixed(1)} s | stream=
        {simInfo.stream_interval.toFixed(1)} s
        {activePerturbations.length > 0 && (
          <span className="pert-tags">
            {" | "}
            {activePerturbations.map((p) => (
              <span key={p} className={styles.pertTag}>
                {p}
              </span>
            ))}
          </span>
        )}
      </div>

      {totalPoints > 0 && (
        <div className={styles.infoBar} data-testid="orbit-info-points">
          {totalPoints} points
        </div>
      )}
    </>
  );
}
