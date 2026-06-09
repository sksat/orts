/**
 * Standalone example for the embeddable {@link OrbitViewer} — no backend, no
 * WebSocket, no charts. A single satellite on a circular equatorial orbit with
 * a growing trail. Doubles as the target for tests/orbit-viewer-lib.spec.ts.
 *
 * Query param `?frame=eci|ecef|sat|lvlh` selects the reference frame.
 *
 * Exposes `window.__example` (advanceTime / appendTrail) so the E2E can drive
 * state changes and assert the trail buffer stays stable.
 */

import { useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import {
  OrbitViewer,
  type SatelliteState,
  type TrailPoint,
  type Vec3,
  type ViewerReferenceFrame,
} from "../../src/lib/index.js";

const EARTH_RADIUS_KM = 6378.137;
const MU = 398600.4418; // km^3/s^2
const ORBIT_RADIUS_KM = 7378; // ~1000 km altitude
const STEP_SECONDS = 30;

/** Deterministic circular equatorial orbit state at trail step `i`. */
function orbitStep(i: number): { position: Vec3; velocity: Vec3 } {
  const speed = Math.sqrt(MU / ORBIT_RADIUS_KM);
  const omega = speed / ORBIT_RADIUS_KM;
  const a = omega * i * STEP_SECONDS;
  return {
    position: [ORBIT_RADIUS_KM * Math.cos(a), ORBIT_RADIUS_KM * Math.sin(a), 0],
    velocity: [-speed * Math.sin(a), speed * Math.cos(a), 0],
  };
}

function frameFromQuery(): ViewerReferenceFrame {
  switch (new URLSearchParams(location.search).get("frame")) {
    case "ecef":
      return { center: "centralBody", orientation: "bodyFixed" };
    case "sat":
      return { center: { satelliteId: "demo" }, orientation: "inertial" };
    case "lvlh":
      return { center: { satelliteId: "demo" }, orientation: "localOrbital" };
    default:
      return { center: "centralBody", orientation: "inertial" };
  }
}

function Example() {
  const [trailCount, setTrailCount] = useState(60);
  const [time, setTime] = useState(0);
  const frame = useMemo(frameFromQuery, []);

  const satellites = useMemo<SatelliteState[]>(() => {
    const trail: TrailPoint[] = [];
    for (let i = 0; i < trailCount; i++) {
      trail.push({ position: orbitStep(i).position, time: i * STEP_SECONDS });
    }
    const head = orbitStep(trailCount - 1);
    return [{ id: "demo", name: "demo", position: head.position, velocity: head.velocity, trail }];
  }, [trailCount]);

  useEffect(() => {
    (window as unknown as Record<string, unknown>).__example = {
      advanceTime: (dt: number) => setTime((t) => t + dt),
      appendTrail: (n: number) => setTrailCount((c) => c + n),
    };
  }, []);

  return (
    <OrbitViewer
      centralBody={{ id: "earth", radiusKm: EARTH_RADIUS_KM }}
      satellites={satellites}
      referenceFrame={frame}
      time={time}
    />
  );
}

const rootEl = document.getElementById("root");
if (rootEl) createRoot(rootEl).render(<Example />);
