/**
 * Standalone example for the embeddable {@link OrbitViewer} — no backend, no
 * WebSocket, no charts. A single satellite on a circular equatorial orbit that
 * animates: the satellite advances along the orbit, the trail grows behind it,
 * and the Earth rotates as simulation time advances. Doubles as the target for
 * tests/orbit-viewer-lib.spec.ts.
 *
 * Query params:
 *   ?frame=eci|ecef|sat|lvlh   reference frame (default eci)
 *   ?animate=0                 freeze (deterministic for E2E)
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
const EPOCH_JD = 2451545.0; // J2000

/** Deterministic circular equatorial orbit state at step `i`. */
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
  const [points, setPoints] = useState(60); // trail length (also the satellite step)
  const [timeOffset, setTimeOffset] = useState(0); // extra sim time (for the E2E hook)
  const frame = useMemo(frameFromQuery, []);
  const animate = useMemo(() => new URLSearchParams(location.search).get("animate") !== "0", []);

  // Advance along the orbit ~16x/s: the satellite moves and the trail grows.
  useEffect(() => {
    if (!animate) return;
    const id = setInterval(() => setPoints((p) => p + 1), 60);
    return () => clearInterval(id);
  }, [animate]);

  const satellites = useMemo<SatelliteState[]>(() => {
    const trail: TrailPoint[] = [];
    for (let i = 0; i < points; i++) {
      trail.push({ position: orbitStep(i).position, time: i * STEP_SECONDS });
    }
    const head = orbitStep(points - 1);
    return [{ id: "demo", name: "demo", position: head.position, velocity: head.velocity, trail }];
  }, [points]);

  // Sim time: tracks the satellite's position so Sun/rotation animate with it.
  const time = (points - 1) * STEP_SECONDS + timeOffset;

  useEffect(() => {
    (window as unknown as Record<string, unknown>).__example = {
      advanceTime: (dt: number) => setTimeOffset((o) => o + dt),
      appendTrail: (n: number) => setPoints((p) => p + n),
    };
  }, []);

  return (
    <OrbitViewer
      centralBody={{ id: "earth", radiusKm: EARTH_RADIUS_KM }}
      satellites={satellites}
      referenceFrame={frame}
      epochJd={EPOCH_JD}
      time={time}
    />
  );
}

const rootEl = document.getElementById("root");
if (rootEl) createRoot(rootEl).render(<Example />);
