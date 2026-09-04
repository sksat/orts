/**
 * Mounts the published {@link AttitudeViewer} with props taken from the URL, so a
 * test can drive the public entry point instead of the app that wraps it.
 *
 * The app's own E2E covers the app. This covers the seam an embedder uses: prop
 * wiring, WASM readiness, the frame fallbacks, and what reaches the scene graph.
 * Served by the dev server only — the production build's input is `index.html`.
 *
 *   /fixtures/attitude-viewer.html?orientation=localOrbital&epoch=2460000.5
 *
 * | param       | meaning                                        |
 * |-------------|------------------------------------------------|
 * | orientation | `inertial` (default) / `bodyFixed` / `localOrbital` |
 * | body        | central body id (default `earth`)              |
 * | epoch       | epoch JD (UTC); omitted means no epoch         |
 * | t           | seconds since the epoch                        |
 * | attitude    | `w,x,y,z` (default: +90° about Z)              |
 * | position    | `x,y,z` km; omitted means no position          |
 * | velocity    | `x,y,z` km/s                                   |
 * | shape       | marker shape override                          |
 * | controls    | `0` mounts no OrbitControls                    |
 */
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { AttitudeViewer } from "../src/lib/index.js";
import type { AttitudeFrame, MarkerShape, Quat, Vec3 } from "../src/lib/index.js";

const params = new URLSearchParams(window.location.search);

function numbers(name: string): number[] | undefined {
  const raw = params.get(name);
  if (raw == null) return undefined;
  return raw.split(",").map((part) => Number(part));
}

function vec3(name: string): Vec3 | undefined {
  const v = numbers(name);
  return v?.length === 3 ? [v[0], v[1], v[2]] : undefined;
}

const attitudeParam = numbers("attitude");
/** +90° about Z, the same known attitude the other specs use. */
const attitude: Quat =
  attitudeParam?.length === 4
    ? [attitudeParam[0], attitudeParam[1], attitudeParam[2], attitudeParam[3]]
    : [Math.SQRT1_2, 0, 0, Math.SQRT1_2];

const epoch = params.get("epoch");
const time = params.get("t");

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <AttitudeViewer
      centralBody={{ id: params.get("body") ?? "earth" }}
      body={{
        id: "fixture-sat",
        attitude,
        position: vec3("position"),
        velocity: vec3("velocity"),
        markerShape: (params.get("shape") as MarkerShape | null) ?? undefined,
      }}
      orientation={(params.get("orientation") as AttitudeFrame | null) ?? "inertial"}
      epochJd={epoch == null ? undefined : Number(epoch)}
      time={time == null ? undefined : Number(time)}
      controls={params.get("controls") !== "0"}
    />
  </StrictMode>,
);
