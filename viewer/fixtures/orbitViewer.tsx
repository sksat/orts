/**
 * Mounts the published {@link OrbitViewer} with satellites taken from the URL,
 * so a test can put the scene in a state a live `orts serve` run cannot produce.
 *
 * The app's own E2E covers the app against a real server, where every satellite
 * shares one clock and every sample is a number. What that cannot reach is the
 * state the public props allow: satellites at *different* times (a terminated one
 * frozen at its last sample), or a `time` that arrived as `NaN` from a source.
 * Both decide which epoch the Sun and the body rotations are evaluated at, so
 * they need a fixture.
 *
 *   /fixtures/orbit-viewer.html?sats=0:7000,0,0:0,7.546,0&epoch=2460000.5
 *
 * | param   | meaning                                                        |
 * |---------|----------------------------------------------------------------|
 * | sats    | `;`-separated satellites, each `t:x,y,z:vx,vy,vz` (t may be `nan`) |
 * | centre  | index into `sats` to centre on; omitted centres the central body |
 * | frame   | `inertial` (default) / `localOrbital` / `bodyFixed`             |
 * | epoch   | epoch JD (UTC); omitted means no epoch                         |
 * | arrows  | `sun` / `nadir` / `sun,nadir`; omitted leaves the prop out, which |
 * |         | is the documented default of drawing none                        |
 * | att     | attitude `w,x,y,z` given to every satellite; makes the display   |
 * |         | frame readable through the rendered world quaternion            |
 * | t       | the scene's elapsed seconds — the epoch a central-body view uses |
 * | body    | central body id (default `earth`)                                |
 * | radius  | central body radius in km (default Earth's)                      |
 * | shape   | marker shape forced on every satellite (`axes-cube` / `sphere`)  |
 * | name    | display name of every satellite — `ISS` resolves a model config   |
 */
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import type { Quat, SatelliteState, Vec3, ViewerReferenceFrame } from "../src/lib/index.js";
import { isArikaReady, OrbitViewer } from "../src/lib/index.js";

const params = new URLSearchParams(window.location.search);

declare global {
  interface Window {
    __fixture_arika_ready?: () => boolean;
  }
}
window.__fixture_arika_ready = () => isArikaReady();

function vec3(text: string): Vec3 {
  const parts = text.split(",").map(Number);
  return [parts[0] ?? 0, parts[1] ?? 0, parts[2] ?? 0];
}

/**
 * One satellite per `;`-separated group, as `t:position:velocity`.
 *
 * `nan` is spelled out rather than written as a literal, because a URL carrying
 * `NaN` through `Number()` is exactly the shape a failed parse upstream has.
 */
const attitudeParam = params.get("att")?.split(",").map(Number);
const attitude: Quat | undefined =
  attitudeParam?.length === 4
    ? [attitudeParam[0], attitudeParam[1], attitudeParam[2], attitudeParam[3]]
    : undefined;

const satellites: SatelliteState[] = (params.get("sats") ?? "0:7000,0,0:0,7.546,0")
  .split(";")
  .map((group, i) => {
    const [time, position, velocity] = group.split(":");
    return {
      id: `fixture-sat-${i}`,
      position: vec3(position ?? "7000,0,0"),
      velocity: velocity == null ? undefined : vec3(velocity),
      attitude,
      markerShape: (params.get("shape") as SatelliteState["markerShape"]) ?? undefined,
      name: params.get("name") ?? undefined,
      time: time === "nan" ? Number.NaN : Number(time ?? 0),
    };
  });

const centre = params.get("centre");
const orientation = params.get("frame") ?? "inertial";
const referenceFrame: ViewerReferenceFrame =
  centre == null
    ? { center: "centralBody", orientation: orientation === "bodyFixed" ? "bodyFixed" : "inertial" }
    : {
        center: { satelliteId: satellites[Number(centre)]?.id ?? satellites[0].id },
        orientation: orientation === "localOrbital" ? "localOrbital" : "inertial",
      };

const arrows = params.get("arrows");
const epoch = params.get("epoch");
const sceneTime = params.get("t");

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <OrbitViewer
      centralBody={{
        id: params.get("body") ?? "earth",
        // Given explicitly: a body arika does not model has no radius to resolve,
        // and the scene needs one to scale itself by.
        radiusKm: Number(params.get("radius") ?? 6378.137),
      }}
      satellites={satellites}
      referenceFrame={referenceFrame}
      epochJd={epoch == null ? undefined : Number(epoch)}
      time={sceneTime == null ? undefined : Number(sceneTime)}
      directionVectors={
        arrows == null
          ? undefined
          : { sun: arrows.includes("sun"), nadir: arrows.includes("nadir") }
      }
    />
  </StrictMode>,
);
