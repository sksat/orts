# OrbitViewer — embeddable orbit viewer

A React component that renders a central body (Earth, a planet, …) and a set of
satellites around it, with an orbit-controls camera. It's the reusable core of
the orts viewer, exposed via the package's `./lib` entry.

| Central-body view (ECI) | Satellite-centred view (LVLH) |
| :---: | :---: |
| ![Earth at the origin with a satellite and its orbit trail](./docs/orbit-viewer-central-body.png) | ![Camera tracking a satellite, Earth's limb below](./docs/orbit-viewer-satellite.png) |

> **Status: not yet published.** The package is still `private`. This entry is
> consumed inside the monorepo today (the app imports it from source). The build
> and packaging below are set up so it _can_ be published later without rework;
> the remaining publish steps (name, dropping `private`, `npm publish`) are
> deliberately deferred.

## Install (when published)

```sh
npm install <pkg>            # name TBD
```

`react`, `react-dom`, `three`, `@react-three/fiber`, `@react-three/drei` are
**peer dependencies** — you supply your single copy. The library bundles nothing
else; the arika WASM (Sun direction / body rotation) is inlined.

## Usage

```tsx
import { OrbitViewer } from "<pkg>/lib";

<OrbitViewer
  centralBody={{ id: "earth", radiusKm: 6378.137 }}
  satellites={[{ id: "sat-1", position: [7000, 0, 1500] }]}
/>;
```

`position` is ECI in km. Advance a `time` prop (and pass `epochJd`) for
physically-correct Sun lighting and body rotation; otherwise a fixed Sun is used
and the body is static. See [`examples/orbit-viewer`](./examples/orbit-viewer)
for an animated, backend-free example.

### Reference frames

`referenceFrame` selects what's at the origin and how the axes are aligned:

- `{ center: "centralBody", orientation: "inertial" }` — ECI-like (default)
- `{ center: "centralBody", orientation: "bodyFixed" }` — ECEF-like
- `{ center: { satelliteId }, orientation: "inertial" }` — satellite at origin,
  star-fixed axes (the body appears to move around it)
- `{ center: { satelliteId }, orientation: "localOrbital" }` — LVLH; the body
  stays "below" as the satellite orbits

### Trails

Pass `trail` (an array of points) per satellite. Trails are uploaded
incrementally: **append** to the array (new reference) to add points cheaply;
treat `satellites`/`trail` as immutable (a new reference on change — in-place
mutation isn't detected). Bump `trailVersion` to force a rebuild.

## Caveats

- **Client-only.** `OrbitViewer` mounts a `<canvas>` and uses `window`; it does
  not server-render. Under Next.js etc., import it client-side only
  (e.g. `next/dynamic` with `ssr: false`).
- **Textures.** Bodies render with a flat fallback colour unless textures are
  reachable. The built-in body texture paths are origin-relative
  (`/textures/earth_2k.jpg`, …); serve those assets, or the bodies simply fall
  back to colours. (A per-consumer base path for _built-in_ textures is a known
  gap; `textureBaseUrl` currently controls only the optional high-res upgrade
  fetched from an orts server.)
- **arika WASM.** Loaded automatically when you pass `epochJd`. To pre-load or
  point at an external `.wasm`, call `initArika({ wasmUrl })` before mounting
  (idempotent; first call wins).
- **React 19.** The peer range targets React 19 (what this is tested against).

## Public API & stability

The headline export is `OrbitViewer` plus its prop types
(`OrbitViewerProps`, `SatelliteState`, `ViewerReferenceFrame`, `Vec3`, …).

Lower-level building blocks are **also exported on purpose** so you can assemble
a custom scene: the primitives (`CelestialBody`, `EarthBody`, `OrbitTrail`,
`Satellite`, `SatelliteModel`, `BodyAxes`), the pure adapters/frame logic
(`toOrbitPoint`, `resolveFrameContext`, `computeLvlhAxes`, `TrailBuffer`), and
`initArika`. Note the trade-off: these primitives wrap internal Three.js /
react-three-fiber components, so they carry a wider semver surface — refactors to
their internals are breaking changes. If you only need the component, import just
`OrbitViewer` and its types.

## Building / packaging (in-repo)

```sh
pnpm --filter orts-viewer run build:lib   # → dist-lib/ (JS + .d.ts, wasm inlined)
pnpm --filter orts-viewer exec pnpm pack   # inspect the would-be tarball
```

`build:lib` (see [vite.lib.config.ts](./vite.lib.config.ts)) externalises the
peer deps and emits to `dist-lib/`, separate from the app build (`dist/`). The
top-level `exports["./lib"]` points at source for in-repo dev; `publishConfig`
repoints it at `dist-lib` on publish.
