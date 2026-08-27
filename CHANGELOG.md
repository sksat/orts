# Changelog

All notable changes to this project will be documented in this file.

The format loosely follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versioning follows [Semantic Versioning](https://semver.org/).

orts is a multi-package workspace (Rust crates on crates.io and npm packages
on npm). Releases are tagged together on the same version, and each version
section is subdivided by package.

## [Unreleased]

### `orts` (Rust, crates.io)

#### Added
- Ground-station contact-window detection (`visibility` module): `GroundStation`
  (WGS-84 location + elevation mask), `ContactWindow` (interpolated AOS/LOS, max
  elevation, span-clip flags), the pure `PassTracker` state machine, and a
  frame-aware `VisibilityMonitor<F: EarthFixedTransform>` that turns ECI samples
  into per-station topocentric look angles. ([#112](https://github.com/sksat/orts/pull/112))
- `IndependentGroup::propagate_to_with(t_target, observer)` — propagate while a
  `FnMut(&SatId, f64, &State)` observer runs on every accepted integration step,
  so callers can sample state at integrator resolution. `propagate_to` delegates
  to it with a no-op observer; trajectories stay bit-identical. ([#112](https://github.com/sksat/orts/pull/112))
- Node-messaging layer (`plugin::message`, "msg-io") for flight-software command
  & telemetry: `Message`, `NodeId` (`Ground` / `Satellite(u32)`), `Payload`,
  `NamedValue`, and `Value` (`Boolean`/`Integer`/`Number`/`Text`/`Bytes`),
  re-exported from `orts::plugin`. ([#58](https://github.com/sksat/orts/pull/58))
- `PluginController` transport hooks (default no-op, implemented by the WASM
  backends): msg-io `deliver` / `take_outbound`, and stream-io `stream_deliver`
  / `stream_take` / `stream_close` for raw byte streams. ([#58](https://github.com/sksat/orts/pull/58), [#84](https://github.com/sksat/orts/pull/84))
- WIT v0 plugin interface extended with the msg-io and stream-io channels. ([#58](https://github.com/sksat/orts/pull/58), [#84](https://github.com/sksat/orts/pull/84))

#### Changed
- `StateEffector` is now frame-generic — `StateEffector<S, F: frame::Eci =
  SimpleEci>` returning `ExternalLoads<F>`, like `Model<S, F>` — so effectors
  produce loads already in the host inertial frame. The defaulted `F` keeps
  existing `StateEffector<S>` impls compiling unchanged. ([#148](https://github.com/sksat/orts/pull/148))
- Sun- and Moon-dependent results move with `arika`'s ephemeris frame fix (SRP,
  third-body, eclipse geometry, sun sensors, and the Harris-Priester density
  bulge): the ephemerides return J2000 directions now instead of
  mean-equinox-of-date ones, a 0.335° change in 2024. Agreement with Orekit
  improves accordingly — the GEO 3-day third-body oracle goes from 218 m to
  0.33 m, and the three shorter Harris-Priester oracles by 20-40%. ([#359](https://github.com/sksat/orts/pull/359))

#### Fixed
- Removed an unsound frame re-tag in `SpacecraftDynamics`: effector loads tagged
  `ExternalLoads<SimpleEci>` were relabeled as the host frame `F` without
  conversion, silently mislabeling coordinates for any `F != SimpleEci`. Latent
  (the only shipped effector is torque-only) but wrong for a translational
  effector. ([#148](https://github.com/sksat/orts/pull/148), [#103](https://github.com/sksat/orts/issues/103))

#### Removed
- **BREAKING**: the `orts::tle` module is removed; TLE parsing moved to
  `arika::tle` (decoding into the shared `arika::elements::Sgp4Elements`).
  Downstream code using `orts::tle` must migrate to `arika`. ([#87](https://github.com/sksat/orts/pull/87))

### `orts-cli` (Rust, crates.io, binary)

#### Added
- Ground-station contact-window reporting in `orts run`: declare stations with
  `[[ground_station]]` (`name`, `latitude_deg`, `longitude_deg`, `altitude_km`,
  `min_elevation_deg`); detected windows print to stderr ordered by AOS, with
  UTC timestamps, sim-time offsets, and max elevation (`*` marks windows clipped
  by the sim span). Earth-centered, epoch-required. ([#112](https://github.com/sksat/orts/pull/112))
- Contact windows are sampled at integrator resolution (every accepted step /
  control tick) rather than at `--output-interval`, so detection no longer
  depends on the output decimation. (A pass shorter than one integrator /
  control sample gap can still be missed.) ([#112](https://github.com/sksat/orts/pull/112))
- `--omm <file>` for CCSDS OMM input (JSON / KVN / XML; `-` for stdin), parsed
  with `arika::elements::parse`; rejects a TLE payload and points to `--tle`. ([#87](https://github.com/sksat/orts/pull/87))
- `--stream-stdio SAT/STREAM` on `orts serve` — wire one declared stream-io
  stream to stdin/stdout over the kble-socket protocol so orts can run as a kble
  `exec:` plug. That stream's WebSocket endpoint then answers HTTP 409; the
  server shuts down when the stdio peer closes. ([#114](https://github.com/sksat/orts/pull/114))
- stream-io kble bridge in `orts serve`: each declared stream is a binary
  WebSocket endpoint at `/stream/{sat}/{stream}` driven by a realtime loop;
  undeclared pairs return HTTP 404. Streams are declared per satellite via the
  config `streams` field. ([#106](https://github.com/sksat/orts/pull/106))
- Config-driven command timeline for flight-software command & telemetry:
  `[[command]]` entries with `t` (sim-time), `sat`, `kind`, and an optional typed
  `args` table, delivered deterministically by the host at the scheduled tick
  (`orts run`). ([#58](https://github.com/sksat/orts/pull/58))
- WebSocket protocol TypeScript types are generated from the Rust types via
  `ts-rs` (`#[derive(TS)]` on the protocol enums, `SimConfig`, `SatelliteInfo`,
  …). Bindings are emitted into the viewer when `cargo test -p orts-cli` runs,
  and CI fails if they drift. ([#95](https://github.com/sksat/orts/pull/95))
- `orts run --json` emits a machine-readable run summary on stdout — `status`,
  the resolved `simulation` parameters, each satellite's `samples` count and
  `final` position/velocity, and the output `artifacts` — while diagnostics stay
  on stderr. Aimed at scripts and coding agents driving orts. Because stdout then
  carries the JSON, the simulation data must be written to a file: combining
  `--json` with data on stdout is rejected. ([#214](https://github.com/sksat/orts/pull/214))
- `orts run --output -` writes the simulation data to stdout. `-` is the
  canonical stdout sentinel; the previous `stdout` keyword is kept as an alias. ([#214](https://github.com/sksat/orts/pull/214))
- `orts config` subcommand group steering users and coding agents toward the
  config file as canonical input: `config example [--format toml|json|yaml]`
  prints a ready-to-edit example config, and `config validate <path> [--json]`
  checks a config and reports the verdict (human-readable on stderr, or a
  machine-readable JSON verdict on stdout with `--json`; exit 0 valid / 2
  invalid). The `--sat` help now points to this path. ([#216](https://github.com/sksat/orts/pull/216))
- `orts --help` (and `-h`) now ends with copy-pasteable examples covering the
  main workflows (run to a file, the `--json` run summary, `config
  example`/`validate`, and `serve`), so the common — and agent-relevant — paths
  are discoverable without leaving the terminal. ([#217](https://github.com/sksat/orts/pull/217))

#### Changed
- `--tle` is TLE-only again (2LE/3LE; `-` for stdin) and pairs with the new
  `--omm`; element-set parsing is now backed by `arika::tle` / `arika::omm`
  instead of the removed `orts::tle`. (Previously `--tle` also auto-accepted OMM.) ([#87](https://github.com/sksat/orts/pull/87))
- Mutually-exclusive orbit-source flags now error instead of silently letting one
  win: `--sat` vs `--tle` / `--omm` / `--tle-line1` / `--tle-line2` /
  `--norad-id`; `--tle` vs `--omm`; file sources vs inline `--tle-line1` /
  `--tle-line2`; and `--tle-line1` / `--tle-line2` must be given together. ([#87](https://github.com/sksat/orts/pull/87))
- TLE epoch day-of-year is validated against the (leap-aware) year length, so a
  malformed field is rejected rather than rolling into another year. ([#87](https://github.com/sksat/orts/pull/87))

#### Fixed
- `orts run --format csv --output <path>` now writes the CSV to `<path>`.
  Previously every `--format csv` run wrote to stdout regardless of `--output`,
  silently ignoring the given path. ([#214](https://github.com/sksat/orts/pull/214))
- `orts serve` started with a `--config` file now rejects a `[[command]]`
  timeline with a clear error (command timelines run only under `orts run`)
  instead of silently dropping it. ([#58](https://github.com/sksat/orts/pull/58))

### `orts-plugin-sdk` (Rust, crates.io)

#### Added
- `msg-io` node-messaging layer for flight-software command & telemetry (and
  future inter-satellite links): a WIT `interface msg-io` (`recv-batch` /
  `send-message`) carrying datagrams addressed by logical `node-id`
  (`ground` / `satellite(u32)`) with a typed `payload`, separate from the
  `tick-io` control plane. The SDK adds a `msg` module (`recv_batch`, `recv_all`,
  `send`, `send_to`, `key_value`, `get`, `get_text`) re-exporting `Message` /
  `Outbound` / `NodeId` / `Payload` / `Value` / `NamedValue`. ([#58](https://github.com/sksat/orts/pull/58))
- `stream-io` raw byte-stream channel for kble virtual-harness integration: a WIT
  `interface stream-io` (`read` / `write` over named streams). orts is a dumb
  byte conduit; framing is left to the FSW + kble pipeline. The SDK adds a
  `stream` module (`read`, `write`, `read_bytes`) re-exporting `StreamRead` /
  `StreamError`. ([#84](https://github.com/sksat/orts/pull/84))
- Example FSWs gain a detumble→nadir mode-transition guard (`commandable-mode-ff`,
  `commandable-mode-rr`). ([#58](https://github.com/sksat/orts/pull/58))

#### Changed
- **BREAKING**: `world plugin` now also imports `msg-io` and `stream-io`. The
  change is purely additive (nothing removed or altered), so callback-style
  guests using `orts_plugin!` are unaffected; hand-written `impl Guest` guests
  must regenerate bindings and link the two new host imports. ([#58](https://github.com/sksat/orts/pull/58), [#84](https://github.com/sksat/orts/pull/84))

### `arika` (Rust, crates.io)

#### Added
- Element-set parsing ([#87](https://github.com/sksat/orts/pull/87)). A shared
  no-alloc `elements::Sgp4Elements` — a *validated* mean-element set (catalog
  number, UTC epoch, six SGP4 mean elements, B\* drag; angles in radians, mean
  motion in rad/s). Built with `Sgp4Elements::try_new` /
  `TryFrom<Sgp4ElementsFields>`, which enforce finite fields, a positive mean
  motion and an eccentricity in `[0, 1)` (returning `ElementsError`); fields are
  read back with `fields()`, with `semi_major_axis(mu)` and `period()` display
  helpers. The text parsers return `elements::ParsedElementSet` (the elements
  plus owned `OBJECT_NAME` / `OBJECT_ID` identity) and reject element sets that
  fail validation; the format-detecting `elements::parse` dispatches to them.
  - `tle` — NORAD TLE / 2LE / 3LE parser (`tle::parse`) → `ParsedElementSet`, with Alpha-5
    alphanumeric catalog numbers and `OBJECT_ID` normalization.
  - `omm::json` / `omm::kvn` / `omm::xml` — CCSDS OMM parsers for the JSON, KVN,
    and XML serializations. JSON accepts a single object or a 1-element array
    (CelesTrak single-satellite GP) and Space-Track string-encoded numbers.
  - `elements::detect` + `elements::parse` — format sniffing (`elements::Format`) plus a
    unified, BOM-tolerant entry point that auto-detects and dispatches
    TLE / OMM-JSON / OMM-KVN / OMM-XML.
- SGP4 / SDP4 propagation behind the optional `sgp4` feature
  (`sgp4::Sgp4Propagator`): builds from an `Sgp4Elements`, reuses the
  epoch `Constants`, and propagates to a `(Vec3<Teme>, Vec3<Teme>)` state in km /
  km·s. Wraps the `sgp4` crate in AFSPC compatibility mode (WGS72). The
  dependency is pulled with only `libm`, so propagation works in `no_std`
  builds without `alloc`. Validated against the Vallado verification vectors for
  near-earth (SGP4) and deep-space (SDP4) satellites. ([#235](https://github.com/sksat/orts/pull/235))
- TEME↔GCRS / TEME↔SimpleEci frame rotations, turning an SGP4 `Vec3<Teme>`
  state into an integration-frame state. `earth::fk5` adds the equinox-based
  IAU-76/FK5 reduction (IAU-76 precession, full 106-term IAU-80 nutation, mean
  obliquity, equation of the equinoxes, GMST 1982, each reproducing the matching
  ERFA routine); `earth::teme` adds `Rotation<Teme, Gcrs>::teme_to_gcrs`,
  `Rotation<Teme, SimpleEci>::teme_to_simple_eci` (an `R3(GMST−ERA)` z-rotation),
  and the `FrameTransform<Teme, Gcrs>` / `FrameTransform<Teme, SimpleEci>` state
  (position+velocity) transforms (ω = 0). The J2000→GCRS frame
  bias (~tens of mas, ≈ sub-metre at LEO) is neglected. Cross-validated against
  ERFA (components, 1e-11) and Orekit (authoritative TEME, ~0.8 m). ([#240](https://github.com/sksat/orts/pull/240))
- `kepler` module (moved into `arika` from `orts`): `KeplerianElements`
  (`from_state_vector` / `to_state_vector` / `period` / `energy`) and the anomaly
  conversions (`solve_kepler_equation`, `mean_to_true_anomaly`, …). Now a public
  `arika::kepler` surface; `orts::orbital::kepler` re-exports it. ([#87](https://github.com/sksat/orts/pull/87))
- `frame::Teme` marker — True Equator, Mean Equinox (the SGP4 / TLE output
  frame). ([#87](https://github.com/sksat/orts/pull/87))
- `earth::topocentric` — ground-site look angles: `TopocentricSite<F: Ecef>`
  (from a WGS-84 `Geodetic`, precomputing the local ENU basis) and `LookAngles`
  (azimuth / elevation / slant range), via `look_angles(target)`. ([#112](https://github.com/sksat/orts/pull/112))
- `frame::MeanEquinoxOfDate` marker — the mean equator and equinox of date (MOD),
  in the `Eci` category: the frame the classical analytic series are referred to
  and the equinox GMST is measured from. `earth::mean_equinox` carries the IAU
  1976 precession between it and `Gcrs`
  (`Rotation<MeanEquinoxOfDate, Gcrs>::iau1976_precession` and the reverse), so a
  consumer building a local hour angle `GMST + λ − α` can put its right ascension
  in the frame GMST belongs to. ([#359](https://github.com/sksat/orts/pull/359))
- `EopTable::clamped()` / `EopTable::into_clamped()` → `ClampedEop`, an EOP
  provider that answers out-of-range queries with its nearest endpoint. dUT1 is
  held through the continuous `UT1 − TAI`, so a leap second past the end of the
  table moves dUT1 by a second instead of stepping UT1. ([#359](https://github.com/sksat/orts/pull/359))

#### Changed
- `Epoch::from_iso8601` also accepts the ordinal / day-of-year form
  (`YYYY-DDDTHH:MM:SS`, used by CCSDS OMM), and the trailing `Z` is now optional.
  A strict relaxation — previously-accepted inputs still parse. ([#87](https://github.com/sksat/orts/pull/87))
- **BREAKING**: `EopTable` no longer implements the EOP capability traits
  (`Ut1Offset`, `PolarMotion`, `NutationCorrections`, `LengthOfDay`). Those traits
  are infallible, and a table covering a finite MJD span has no correct infallible
  answer outside it — it used to `.expect()`, turning an ordinary out-of-range
  epoch into a process abort from inside `Epoch::to_ut1` and the IAU 2006 full
  chain. Pass `table.clamped()` (borrowing) or `table.into_clamped()` (owning) to
  name the out-of-range policy, or use the `*_checked` accessors to get
  `EopLookupError::OutOfRange`. ([#359](https://github.com/sksat/orts/pull/359))
- `KeplerianElements::from_state_vector` now states its degenerate-geometry
  conventions on the type (a table of what `raan` /
  `argument_of_periapsis` / `true_anomaly` hold for circular, equatorial and
  circular-equatorial orbits), and computes every in-plane angle with one
  `atan2`-based helper measured about the orbit normal — recovering the half
  mantissa the previous `acos` + quadrant tests lost near ν = 0 and i = 0.
  Non-degenerate orbits are unchanged. ([#359](https://github.com/sksat/orts/pull/359))

#### Fixed
- `KeplerianElements::from_state_vector` lost the periapsis direction of an
  eccentric equatorial orbit: it zeroed both the RAAN and the argument of
  periapsis while still measuring the true anomaly from the eccentricity vector,
  so the in-plane periapsis longitude was stored nowhere. a = 10,000 km, e = 0.2,
  i = 0, ϖ = π/2 came back rotated 90°, an 11,313.7 km round-trip position error.
  The equatorial branch now stores the true longitude of periapsis ϖ = Ω + ω
  (negated for retrograde, matching `to_state_vector` at i = π). ([#359](https://github.com/sksat/orts/pull/359))
- The Meeus Sun and Moon ephemerides returned mean-equinox-of-date vectors typed
  `Vec3<Gcrs>`. Their mean longitudes advance at the tropical rate and their
  ecliptic → equatorial rotation uses the mean obliquity of date, so the result
  carried the whole precession accumulated since J2000: 0.335° in 2024, growing
  ~1.4°/century — ~2,250 km transverse on the Moon vector, an order of magnitude
  above the series' own ~1′ accuracy. They now rotate back to J2000 with the IAU
  1976 precession (nutation, ≤ 17″, and the J2000→GCRS frame bias, ~20 mas, stay
  out). This moves Sun/Moon-dependent results — SRP, third-body, eclipse, sun
  sensors — by that angle. The 0.35° figure previously documented as Meeus model
  accuracy was this rotation, not model error. ([#359](https://github.com/sksat/orts/pull/359))
- `sun::sun_direction_from_body`'s planet branch rotated the Standish
  heliocentric elements — referred to the J2000 mean ecliptic — into equatorial
  coordinates with the obliquity *of date*, leaving 11″ of frame error in 2024 and
  35″ by 2075. It now uses the fixed J2000 obliquity. ([#359](https://github.com/sksat/orts/pull/359))
- `EopTable::dut1_checked` interpolated dUT1 straight through a leap second,
  smearing half of the 1 s step over the preceding day: the IERS rows bracketing
  2017-01-01 (−0.5928 s / +0.4068 s) gave −0.093 s at the midpoint instead of
  ≈ −0.593 s, a 0.5 s UT1 error — 3.7e-5 rad of ERA, ~230 m at the equator. It now
  interpolates the continuous `UT1 − TAI = dUT1 − (TAI − UTC)` and adds the query
  instant's own `TAI − UTC` back. ([#359](https://github.com/sksat/orts/pull/359))

### `utsuroi` (Rust, crates.io)

#### Added
- `IntegrationError` now implements `core::error::Error` (by hand, no
  `thiserror`, works under `no_std`), so it participates in `?` chains and
  `Box<dyn Error>`. ([#147](https://github.com/sksat/orts/pull/147))

### `tobari` (Rust, crates.io)

#### Changed
- Renamed the CSSI space-weather download feature `fetch` to `fetch-cssi`,
  matching the `fetch-<source>` convention (`fetch-igrf`, and `arika`'s
  `fetch-horizons`). `fetch` is retained as an umbrella feature that enables
  every `fetch-*` source, so `features = ["fetch"]` keeps building (and now
  also pulls in `fetch-igrf`). ([#150](https://github.com/sksat/orts/pull/150))

### `viewer`

#### Added
- Embeddable viewer library at a new `./lib` entry (`viewer/src/lib`), so the
  orbit viewer can be dropped into any React + `@react-three/fiber` app, not only
  the bundled SPA. Layered API:
  - `OrbitViewer` — batteries-included: renders its own sized `<div>` +
    `<Canvas>`; drive it with a `centralBody` and a `SatelliteState[]`.
  - `OrbitScene` — the scene graph to mount inside your own `<Canvas>`
    (bring-your-own Canvas), initialised with the exported `SCENE_UP`.
  - The viewer's own app is now built on the public `OrbitScene` API
    (dogfooded), so the library and the app cannot drift apart.
  ([#89](https://github.com/sksat/orts/pull/89), [#175](https://github.com/sksat/orts/pull/175), [#176](https://github.com/sksat/orts/pull/176))
- Distribution as a shadcn registry (`registry.json`, item `orbit-viewer`): the
  component and its primitives can be vendored into a consumer app via
  `shadcn add`. Ships a standalone consumer example (`viewer/examples/orbit-viewer/`)
  that installs and renders the registry item. ([#168](https://github.com/sksat/orts/pull/168), [#169](https://github.com/sksat/orts/pull/169))
- Extensible central bodies: custom definitions via the `bodies` prop
  (`BodyDefinitions`) merged over the built-in `DEFAULT_BODIES`
  (Earth / Moon / Sun / Mars). Exports `BodyDefinition` / `BodyDefinitions` /
  `BodyTexture` / `DEFAULT_BODIES`. ([#164](https://github.com/sksat/orts/pull/164))
- Injectable arika WASM: `initArika({ wasmUrl? })` / `isArikaReady()` are
  exported so an embedder can preload the module or point at an external `.wasm`
  URL. The arika WASM was extracted into its own workspace package (`arika-wasm`),
  imported by name (required for registry distribution). ([#159](https://github.com/sksat/orts/pull/159), [#167](https://github.com/sksat/orts/pull/167))
- Public `TrailBuffer` streaming primitive (`TrailBuffer` + `TrailBufferLike`): a
  caller can own a bounded trail buffer and mutate it outside React
  (`SatelliteState.trailBuffer`); the scene reads it each frame so streamed points
  reach the GPU without a React re-render. Exports `toTrailBuffer` /
  `trailPointToOrbitPoint` and the `OrbitPoint` / `TrailPoint` types. ([#176](https://github.com/sksat/orts/pull/176))
- Per-satellite display props on `SatelliteState`: `color`, `name`,
  `markerShape`, `trailDisplay` (`visibleCount` / `drawStart`, for playback
  scrubbing), and a per-satellite `time` so a frozen/scrubbed satellite keeps its
  marker aligned with its own body-fixed trail. ([#89](https://github.com/sksat/orts/pull/89), [#176](https://github.com/sksat/orts/pull/176))
- Satellites render from their current position, not only from trails — a
  position-only satellite still shows a marker. ([#89](https://github.com/sksat/orts/pull/89))
- Selectable marker shapes (`MarkerShape`: `"sphere"` | `"axes-cube"`), including
  a non-sphere XYZ orientation cube that shows attitude without a hosted 3D
  model; resolvable per-satellite or scene-wide, and declarable by the simulation
  over the wire (viewer-overridable). ([#158](https://github.com/sksat/orts/pull/158))
- Satellite-centred frames now honour the requested orientation: star-fixed
  `inertial` (axes don't co-rotate) or `localOrbital` (LVLH). Previously a
  satellite-centred view always collapsed to LVLH. ([#111](https://github.com/sksat/orts/pull/111), [#90](https://github.com/sksat/orts/issues/90))

#### Changed
- The `./lib` public barrel is intentionally narrow: the Three.js / r3f building
  blocks and the internal frame wiring are not exported. Supported surface:
  `OrbitViewer`, `OrbitScene`, `TrailBuffer` / `TrailBufferLike`, `toTrailBuffer`
  / `trailPointToOrbitPoint`, `initArika` / `isArikaReady`, `SCENE_UP`,
  `DEFAULT_BODIES`, `DEFAULT_VIEWER_FRAME`, and the supporting types. ([#177](https://github.com/sksat/orts/pull/177))
- DuckDB-wasm assets are self-hosted by the viewer (Vite `?url` imports passed to
  uneri's `initDuckDB({ bundles })`) instead of the jsDelivr CDN, removing a
  third-party-CDN runtime dependency. ([#171](https://github.com/sksat/orts/pull/171))
- Earth-specific rendering (day/night terminator, atmosphere, Earth spin) is
  gated to the `earth` body id, not "has a night texture"; custom bodies render
  via the generic textured-sphere path. ([#164](https://github.com/sksat/orts/pull/164))
- `OrbitScene` / `OrbitViewer` throw a clear error when the central body has no
  resolvable radius, instead of silently using radius 1 and a wrong scene scale. ([#164](https://github.com/sksat/orts/pull/164))
- The arika WASM loads only when an `epochJd` is supplied; epoch-less embedders
  pay no init cost (fixed Sun direction, no body rotation). ([#89](https://github.com/sksat/orts/pull/89))
- WS protocol types are now the `ts-rs`-generated bindings (see `orts-cli`),
  replacing the hand-written wire types and adding the `satellite_added` variant. ([#95](https://github.com/sksat/orts/pull/95))

#### Fixed
- Default WebSocket URL on static deploys falls back to `ws://localhost:9001/ws`
  instead of deriving an unreachable host from `window.location`. ([#143](https://github.com/sksat/orts/pull/143))
- High-resolution body textures restored in static deployment (server-only fetch,
  off-thread decode, bounded upgrade retries, in-flight guard). ([#88](https://github.com/sksat/orts/pull/88), [#113](https://github.com/sksat/orts/pull/113), [#105](https://github.com/sksat/orts/issues/105))
- LVLH (satellite-centred) central-body orientation corrected, with separate
  Earth (ERA) vs non-Earth (`body_orientation` + pole) paths. ([#51](https://github.com/sksat/orts/pull/51))
- Quaternion slerp guards on a complete quaternion (all of qw/qx/qy/qz) rather
  than `qw` alone; NaN passes through. Dropped a per-render allocation in the
  scene. ([#172](https://github.com/sksat/orts/pull/172))
- Trail-buffer mutations are applied in the commit phase; `satellites[]` is
  rebuilt on a trail-buffer reset; per-satellite position time is preserved for
  body-fixed markers; `SatelliteState.color` is honoured; file / RRD adapters
  reset cleanly on restart and tear down on fatal worker errors. ([#89](https://github.com/sksat/orts/pull/89), [#107](https://github.com/sksat/orts/pull/107), [#108](https://github.com/sksat/orts/pull/108), [#176](https://github.com/sksat/orts/pull/176))

#### Performance
- Trail-less satellites skip trail-buffer allocation and per-frame trail work. ([#107](https://github.com/sksat/orts/pull/107))

### `uneri` (npm: `@sksat/uneri`)

#### Added
- `initDuckDB` can load the DuckDB-wasm worker / wasm from caller-injected,
  self-hosted bundle URLs instead of the jsDelivr CDN. New `DuckDBInitOptions`
  (`bundles?`, `fallbackToJsDelivr?`) and `DuckDBBundleUrls` types, plus a pure
  `resolveBundleSource(options?)`. uneri stays bundler-neutral; the app resolves
  and passes the URLs. ([#171](https://github.com/sksat/orts/pull/171))
- Resilient init: `initDuckDB` retries with linear backoff, fast-fails on a dead
  worker via an `error` listener instead of hanging, and drops the cached
  rejected promise after terminal failure so a later call retries. ([#76](https://github.com/sksat/orts/pull/76), [#70](https://github.com/sksat/orts/issues/70))

#### Changed
- Calling `initDuckDB()` with no options is unchanged — it still sources bundles
  from the jsDelivr CDN — so existing consumers keep working; self-hosting is
  opt-in via `options.bundles`. ([#171](https://github.com/sksat/orts/pull/171))

#### Fixed
- Worker 404 / "invalid URL" on init: bundle URLs are absolutized against the
  worker origin inside `initDuckDB`, because DuckDB instantiates its worker from a
  `blob:` URL against which a root-relative path cannot resolve. ([#171](https://github.com/sksat/orts/pull/171))

### Docs

#### Added
- `llms.txt`, `llms-full.txt`, and `llms-small.txt` are generated for the
  documentation site (via `starlight-llms-txt`), so coding agents and LLM
  tooling can ingest the docs — e.g. point an agent at
  <https://sksat.github.io/orts/llms.txt>. `llms-full.txt` is the complete
  corpus; `llms-small.txt` is a condensed overview that excludes the
  autogenerated rustdoc/typedoc API reference. ([#225](https://github.com/sksat/orts/pull/225))

### Dependencies

- Rust toolchain → 1.96.0.
- Rust: `wasmtime` / `wasmtime-wasi` 44 (security), `rerun` 0.33,
  `tokio-tungstenite` 0.29, `nalgebra` 0.35, `tokio` 1.52, `axum` 0.8.9.
- `notalawyer` 0.3 — the embedded third-party license NOTICE is generated
  through the cargo-about *library* (an `orts-cli` build-dependency) instead
  of the `cargo about` binary, so no binary is installed in CI or baked into
  the cross build image.
- npm: `vite` 8, `@vitejs/plugin-react` 6, the React monorepo, `ws` 8.21
  (security), `mermaid` 11.15 (security).

## [0.2.0](https://github.com/sksat/orts/releases/tag/v0.2.0) - 2026-04-20

Release blog post: [orts: 人工衛星シミュレーションプラットフォームを作りました](https://sksat.hatenablog.com/entry/orts-release)

- `ARCHITECTURE.md` (EN / JA) with automatic cross-language link
  rewriting
- orts logo kit integrated across docs / viewer / README
- Brand name unified as `orts` (lowercase) across the repository,
  replacing `Orts`
- Notable dependency updates:
  - Rust: `nalgebra` 0.34, `clap` 4.6, `criterion` 0.8, `ureq` 3.3,
    `toml` 1.1, `proptest` 1.11, `rand` 0.9.4 (security)
  - npm: `@astrojs/starlight` 0.38.3, `@biomejs/biome` 2.4,
    `happy-dom` 20.8.9 (security, dev only)

### `orts` (Rust, crates.io)

#### Added
- SRP and sun sensor now consume `arika::eclipse` for continuous
  illumination scaling and eclipse detection through the conical
  penumbra
- Per-device actuator commands
  - MTQ and reaction wheels are individually addressable device lists
    with per-device command dispatch
- Multi-instance sensors: sensors are now `Vec`-based for arbitrary
  multiplicity
- Reaction wheel motor first-order lag model
- RW speed / torque command variants and `MtqCommand` variant
- Pseudo-inverse torque / dipole allocation for non-orthogonal RW / MTQ
  layouts
- Sun sensor model with fine / coarse measurement variants
- Controlled simulation attitude / command / telemetry logging
  - Dynamic CSV column generation
- `ThrusterSpec` shared between host-scheduled `Thruster` and
  plugin-commanded `ThrusterAssembly`, following the MTQ Core+Assembly
  pattern

#### Changed
- **BREAKING**: B-dot detumble controller renamed `BdotDetumbler` →
  `BdotCross` for naming consistency with `BdotFiniteDiff`. The
  rename makes the dB/dt estimation method (cross-product `-ω × B` vs
  finite difference) explicit
- Actuator telemetry restructured into a unified representation across
  actuator types
- `orts convert` extended to output full data including attitude,
  commands, and telemetry (not just orbital state)
- CSV metadata and satellite output unified via
  `SimMetadata::write_csv_header` / `write_satellite_csv`

### `orts-cli` (Rust, crates.io, binary)

#### Added
- WASM plugin thruster throttle commands (`[0,1]` per device) are
  wired through the controlled simulation loop (Phase P4)

#### Changed
- **BREAKING**: `orts run` now requires an orbit specification. If none
  of `--sat` / `--tle` / `--norad-id` / `--config` / an `orts.toml` in
  CWD is provided, the command errors out. The previous silent default
  of a 400 km circular orbit was too implicit
- **BREAKING**: `--altitude` flag removed. Orbit specification is done
  via `--sat "altitude=400,inclination=51.6"` or a config file so the
  parameters are explicit
- `orts run` auto-detects `orts.toml` in CWD (resolution order:
  `--config` > CLI orbit args > `orts.toml` > error)

### `orts-plugin-sdk` (Rust, crates.io)

#### Added
- `no_std` support
  - Compilable without the standard library (no allocator required)
  - Optional `alloc` feature flag for heap usage under `no_std`
- WIT plugin interface gains a thruster throttle command (`[0,1]` per
  device). All example plugins updated for the new command field
- New example: `nos3-adcs` — NOS3 `generic_adcs` WASM plugin (SILS demo)
  - All-mode tests, IGRF integration, visualization scripts, CI workflow
- New example: `constellation-phasing` — satellite constellation phase
  control demo
- New example: `transfer-burn-with-tcm` — orbit transfer with
  trajectory correction maneuver demo

#### Changed
- **BREAKING**: WIT v0 sensor / actuator / command records restructured.
  Existing plugins must regenerate bindings and update tick handlers:
  - Sensors: `option<T>` → `list<T>` (magnetometer / gyroscope /
    star-tracker / sun-sensor are now multi-instance)
  - Actuators: `ActuatorState` → `ActuatorTelemetry` (RW is now a
    structured `RwTelemetry` record)
  - Commands: `commanded-magnetic-moment` / `commanded-rw-torque`
    replaced with `mtq-command` / `rw-command` variants, and
    `thruster-command` variant added
  - Sun sensor: `sun-fine-output.direction` is now an `option`
    (`None` during total eclipse); fine / coarse variants introduced
- Example plugins moved to `plugin-sdk/examples/` workspace
- WIT bindings generation migrated to `wit_bindgen::generate!()`,
  reducing the `cargo component` dependency surface
- `bdot-finite-diff` example revamped with a longer simulation and
  multi-model comparison layout

### `arika` (Rust, crates.io)

#### Added
- `eclipse` module — generic illumination API (observer / light /
  occulter) providing both cylindrical (binary) and conical
  (Montenbruck & Gill penumbra) shadow models
- `no_std` + `alloc` support (tiered feature hierarchy)
  - no alloc: core math (coordinate frames, epoch arithmetic, analytical
    ephemerides, geodetic conversions, IAU 2006 precession/nutation)
  - `+ alloc`: Horizons, EopTable, HorizonsMoonEphemeris
  - `+ std`: `Epoch::now()`, file I/O, fetch-horizons
  - `libm`-backed `F64Ext` trait for transcendental functions under
    no_std

#### Changed
- Browser-facing WASM facade split into a dedicated `arika-wasm` crate

### `utsuroi` (Rust, crates.io)

#### Added
- `no_std` support — pure math with no heap allocation, so no `alloc`
  feature is needed. Adds `libm`-backed `F64Ext` trait

### `tobari` (Rust, crates.io)

#### Added
- `no_std` + `alloc` support (tiered feature hierarchy)
  - no alloc: Exponential, Harris-Priester, TiltedDipole,
    SpaceWeather traits, ConstantWeather
  - `+ alloc`: NRLMSISE-00, IGRF, CSSI/GFZ parsing
  - `+ std`: file I/O, fetch, OnceLock

#### Changed
- Browser-facing WASM facade split into a dedicated `tobari-wasm` crate
- `Nrlmsise00` is now generic over `SpaceWeatherProvider` (alloc-free)
- IGRF / NRLMSISE-00 internal storage changed from `Vec` to fixed-size
  arrays (alloc-free)

### `starlight-rustdoc` (npm)

#### Added
- Display feature-gate badges on generated API documentation pages

### Docs

#### Added
- LaTeX math rendering on the Starlight docs site
  (`remark-math` + `rehype-katex`)
- Mermaid diagram rendering on the Starlight docs site via
  `astro-mermaid`
- Example READMEs auto-discovered via YAML frontmatter and published as
  docs pages

#### Changed
- Example control-law descriptions migrated to LaTeX math
- Crate sidebar groups expanded by default; API entries remain collapsed
  for navigation efficiency

## [0.1.1](https://github.com/sksat/orts/releases/tag/v0.1.1)

### `orts-cli` (Rust, crates.io, binary)

- Fix `include_bytes!` texture paths for `cargo install` from crates.io.
  Textures are now copied into `cli/textures/` by `build.rs` and referenced
  via `CARGO_MANIFEST_DIR`, matching the `viewer-dist/` pattern.

### `uneri` (npm: `@sksat/uneri`)

- Renamed from `uneri` to `@sksat/uneri` (scoped package). npm rejected
  the unscoped name as too similar to existing packages.

## [0.1.0](https://github.com/sksat/orts/releases/tag/v0.1.0)

### `orts` (Rust, crates.io)

- Core orbital mechanics simulation: `OrbitalState` (position + velocity),
  `AttitudeState` (quaternion + angular velocity), and `SpacecraftState`
  combining both. Capability-based model composition via `HasOrbit`,
  `HasAttitude`, `HasMass` trait bounds.
- Orbital dynamics: two-body, Brouwer mean-element propagator, gravity
  spherical harmonics (up to degree 16), and a TLE/SGP4-equivalent path.
- Perturbation force models: atmospheric drag (with pluggable density via
  `tobari`), solar radiation pressure with eclipse shadow, third-body
  gravity (Sun / Moon), and scheduled / constant-throttle thrust.
- Attitude dynamics and control: rigid-body dynamics, gravity-gradient and
  aerodynamic torques, reaction wheels, thrusters, surface panels, and
  controllers including B-dot detumbler, PD tracker, and nadir/inertial
  pointing references.
- Sensor models: magnetometer, gyroscope, and star tracker with optional
  noise injection.
- WebAssembly Component Model plugin runtime via wasmtime (`plugin-wasm`
  feature) for loading guest controllers at runtime, with an optional
  fiber-based async backend (`plugin-wasm-async`) for multiplexing many
  satellites on a single worker thread.
- Recording and telemetry to Rerun RRD with structured archetypes for
  position / velocity / attitude / angular velocity in multiple frames.
- Event detection and integration termination for spacecraft constraints
  (deorbit, apogee / perigee passage, ground contact).
- Optional features: `fetch-weather` (CSSI / GFZ space weather download,
  via `tobari/fetch`), `fetch-horizons` (JPL Horizons ephemeris HTTP fetch,
  via `arika/fetch-horizons`).
- Depends on workspace crates `arika` (frames / epochs / ephemerides),
  `utsuroi` (integrators), and `tobari` (atmosphere + magnetic field).
- Ships simulation examples under `orts/examples/`:
  - `apollo11` — full Apollo 11 trajectory propagation and 3D
    visualisation validated against JPL Horizons reference.
  - `artemis1` — NASA Artemis 1 coast feasibility spike (three major
    phases of the 2022-11-16 → 2022-12-11 mission) propagated with
    Earth-centric DOP853 and compared to Horizons Orion target `-1023`.
  - `orbital_lifetime` — long-arc decay simulation demonstrating drag +
    mean-element propagation.
  - `wasm_bdot_simulate` / `wasm_pd_rw_simulate` — host-side examples
    that load the `orts-example-plugin-*` WASM guests (see
    `orts-plugin-sdk` below) and run a detumbling / RW-control scenario
    end-to-end.

### `orts-cli` (Rust, crates.io, binary)

- `orts` binary with four primary subcommands:
  - `orts run` — batch simulation, writes `.rrd` (default) or `.csv`.
  - `orts serve` — WebSocket telemetry server on port 9001 plus the
    embedded 3D viewer SPA at `http://localhost:9001`.
  - `orts replay` — streams a recorded `.rrd` through the embedded viewer.
  - `orts convert` — transforms between `.rrd` and `.csv` formats.
- CLI flags cover altitude, central body (Earth / Moon / Mars), time step,
  output interval, epoch (ISO 8601), TLE input (file or
  `--tle-line1` / `--tle-line2`), YAML config, and WASM plugin controller
  specification.
- Embedded 3D viewer (`viewer` feature, on by default): React +
  Three.js + `@react-three/fiber` SPA bundled into the binary via
  `rust-embed`, served over the same WebSocket process for zero-setup
  visualization.
- Multi-satellite plugin backend: default thread-per-satellite (`sync`) or
  fiber-multiplexed (`async`) runtime, selectable at runtime for
  constellation-scale scenarios.
- `[package.metadata.binstall]` installed so
  `cargo binstall orts-cli` fetches the prebuilt GitHub Release tarball
  directly, no compilation required. Both `x86_64-unknown-linux-gnu` and
  `x86_64-unknown-linux-musl` (fully static) targets available.
- Single-binary distributable: ships the simulator, WebSocket server, and
  viewer SPA together.

### `orts-plugin-sdk` (Rust, crates.io)

- SDK for writing orts WASM plugin guests targeting the Component Model
  via `cargo component`.
- Callback-style `Plugin<I, C>` trait: implement `sample_period()`,
  `init(config)`, `update(input) -> Option<Command>`, and optional
  `current_mode()`; the `orts_plugin!(MyController)` macro wraps it into
  a world-conforming `Guest` impl (tick loop, mode reporting, error
  propagation).
- Main-loop style: call `wait_tick()` / `send_command()` from a custom
  `impl Guest` for sequential "phase 1 → wait → phase 2" controllers.
- `I` / `C` are generic and default to the WIT-generated `TickInput`
  (orbital / attitude state + sensor readings) and `Command`
  (thruster authority, magnetorquer dipole, reaction wheel torque).
- No runtime dependencies — the macro references the consumer's
  `bindings` module generated by `cargo component` from the orts plugin
  WIT world.
- Example plugin guest crates shipped under `plugins/` as independent
  cargo workspaces (not published to crates.io, reference implementations
  for users writing their own controllers):
  - `orts-example-plugin-bdot-finite-diff` — main-loop-style B-dot
    detumbling controller using a finite-difference `dB/dt` estimate from
    successive magnetometer samples.
  - `orts-example-plugin-pd-rw-control` — callback-style PD attitude
    tracker driving reaction wheels via left-invariant quaternion error.
  - `orts-example-plugin-pd-rw-unloading` — callback-style PD attitude
    control plus simultaneous magnetorquer-based reaction wheel momentum
    unloading.
  - `orts-example-plugin-detumble-nadir` — callback-style detumble →
    nadir-pointing mode transition with a user-defined convergence
    criterion.

### `arika` (Rust, crates.io)

- Phantom-typed frame system: `Vec3<F>` for frame-tagged 3D vectors and
  `Rotation<From, To>` for frame transforms. Frame markers include
  `SimpleEci`, `SimpleEcef` (ERA-only rotation), `Gcrs`, `Cirs`, `Tirs`,
  `Itrs` (IAU 2006 CIO chain), `Rsw` (local orbital
  radial / along-track / cross-track), and `Body` (spacecraft-fixed).
- IAU 2006 / 2000A_R06 CIO-based Earth rotation: precession, nutation,
  CIP X / Y / s series evaluators, and full `Rotation<Gcrs, Itrs>`
  composition with EOP provider traits.
- Scale-tagged `Epoch<S>` with `S ∈ {Utc, Tai, Tt, Ut1, Tdb}` — compile-time
  prevents silent mixing of time scales. Conversions between scales are
  explicit methods (`to_tai()`, `to_tt()`, etc.).
- Celestial body ephemerides via the `EphemerisProvider` trait: low-precision
  Meeus analytic models for Sun / Moon / planets, plus an optional JPL
  Horizons vector-table parser with Hermite interpolation and disk caching
  (`fetch-horizons` feature).
- WGS84 geodetic ↔ ECEF conversion, RSW orbital frame computation
  (`rsw_quaternion(pos, vel)`), and body-to-RSW attitude transforms.
- `wasm` feature: compiles to `wasm32-unknown-unknown` via `wasm-bindgen`
  so browser viewers can run ECI ↔ ECEF transforms and ephemeris lookups
  without a native round-trip.

### `utsuroi` (Rust, crates.io)

- Unified `Integrator` trait with multi-step integration, event detection,
  and NaN / Inf guards via `integrate_with_events()`.
- Fixed-step integrators: RK4 (4th-order Runge-Kutta), Störmer-Verlet
  (2nd-order symplectic, long-arc energy conservation), and Yoshida 4th /
  6th / 8th-order symplectic compositions.
- Adaptive step-size integrators: Dormand-Prince RK5(4)7M with FSAL
  (a.k.a. DP45) and DOP853 (Hairer / Nørsett / Wanner 8th-order RK8(5,3)).
- Trait-based problem definition: `DynamicalSystem` defines the derivative,
  `OdeState` provides BLAS-like operations (`axpy`, `scale`, `error_norm`),
  so solver code is generic over any state dimension.
- Pure Rust, no LAPACK / BLAS dependency.

### `tobari` (Rust, crates.io)

- Atmospheric density models behind the `AtmosphereModel` trait:
  `Exponential` (US Standard Atmosphere 1976, altitude-only),
  `HarrisPriester` (diurnal variation via Sun position), and
  `Nrlmsise00` (full NRLMSISE-00 empirical model with solar / geomagnetic
  activity inputs).
- Geomagnetic field via IGRF-14 spherical-harmonic expansion (`Igrf`,
  degree 1-13 configurable) with vendored 2020 DGRF + 2025 IGRF +
  secular variation coefficients. Custom coefficients can be injected at
  runtime. Tilted-dipole approximation also available.
- `SpaceWeatherProvider` trait with built-in providers: `ConstantWeather`
  (fixed F10.7 / Ap), `CssiSpaceWeather` (CelesTrak CSSI CSV parser),
  and `GfzSpaceWeather` (GFZ Kp / Ap / F10.7 parser).
- Default `fetch-igrf` feature builds against vendored coefficients; the
  optional `fetch` feature pulls live CSSI / GFZ data over HTTP.
- `wasm` feature exposes density and field lookups via `wasm-bindgen` for
  browser-side atmosphere / magnetic-field visualizers.
- Depends on `arika` for frame-tagged positions and geodetic conversions.
- Shipped demo: `tobari-example-web` (private npm workspace under
  `tobari/examples/web/`) — React + Three.js browser demo visualising
  atmosphere density, IGRF geomagnetic field, and space weather data
  entirely in-browser via the `tobari` + `arika` WASM builds. Not
  published to npm; used as an integration smoke test and as the
  embedded live demo on the docs site.

### `rrd-wasm` (Rust, crates.io)

- WebAssembly-friendly Rerun RRD decoder wrapping the decoder portion of
  the Rerun SDK (`re_log_encoding`, `re_chunk`, `re_log_types`,
  `re_sdk_types`).
- `wasm` feature exposes a `parse_rrd(bytes)` entry point returning a
  structured `{metadata, rows}` object serializable via
  `serde-wasm-bindgen`. Browser viewers can decode `.rrd` byte streams on
  a Web Worker without shelling out to the native Rerun Viewer.
- Metadata: epoch (Julian Date), gravitational parameter μ, body radius,
  body name, orbital altitude, period.
- Row payload: timestamp, position / velocity (km, km/s), entity path,
  and optional quaternion / angular velocity.
- Zero dependency on orts-specific simulation logic — pure data
  serialization layer.

### `uneri` (npm)

- React `<TimeSeriesChart />` component wrapping
  [uPlot](https://github.com/leeoniya/uPlot) for real-time time-series
  visualization, with series isolation in the legend.
- Schema-driven API: declare columns (`DOUBLE`, `INTEGER`, `FLOAT`,
  `BIGINT`) and derived SQL expressions; uneri handles table creation,
  ingestion, and query-time downsampling inside the browser.
- `IngestBuffer<T>` staging buffer with a drain pattern, decoupling
  stream arrival (WebSocket, file, etc.) from DuckDB insert cadence.
- `useTimeSeriesStore` hook for a realtime tick loop:
  accumulate → INSERT → periodic downsampled query with configurable
  refresh rates.
- Time-bucketed downsampling at query time so chart coverage remains
  proportional regardless of data density (sparse / dense mixtures stay
  visually balanced).
- `ChartDataWorkerClient` / `MultiChartDataWorkerClient` offload DuckDB
  operations onto a dedicated Web Worker so multiple charts stay
  non-blocking during ingestion and rendering.
- Subpath exports for advanced use: `uneri/align` (time-series alignment
  helpers), `uneri/multiWorkerClient` (multi-chart worker client), and
  `uneri/workerProtocol` (worker message types).
- Built on `@duckdb/duckdb-wasm` 1.32.0 for in-browser OLAP with `uplot`
  1.6 as the render layer. React ≥ 18 as peer dependency.

### `viewer`

- Web-based real-time 3D orbit viewer built on React + `@react-three/fiber`
  (Three.js) + Vite. Bundled into the `orts-cli` binary and served at
  `http://localhost:9001`, and also deployed as a standalone SPA.
- 3D scene: textured central bodies via a generic `CelestialBody` component
  (Earth / Moon / Sun / Mars), with custom GLSL shaders for Earth's day/night
  terminator and atmospheric scattering, plus an orbit-controls camera.
- Per-satellite visualization: 3D trajectory trails, 3D satellite models with
  a configurable display scale (true-scale sizing in the satellite-centered
  view), and body-frame attitude axes driven by the attitude quaternion.
- Reference-frame selection: a central-body-centered inertial (ECI) or
  body-fixed (ECEF) view, or recenter on a satellite to track it in its
  local-orbital (LVLH) frame. ECI ↔ ECEF transforms run in-browser via the
  `arika` WASM build.
- Data sources: CSV and `.rrd` orbit-file loading (`.rrd` decoded in-browser
  via `rrd-wasm`) and a live WebSocket mode (`useWebSocket`) streaming
  telemetry from `orts serve` for one or many satellites.
- In-browser simulation control: configure simulation parameters and
  pause / resume / terminate the running `orts serve` simulation from the UI.
- Replay / playback: a `useRealtimePlayback` hook drives time-based orbit replay
  with progressive trail drawing and a `PlaybackBar` scrubber
  (play / pause / seek).
- In-browser analytics: DuckDB-wasm + uPlot time-series charts (built on
  `uneri`) with drag-zoom and multi-satellite series.

### `starlight-rustdoc` (npm)

- Astro / Starlight integration that turns `cargo rustdoc --output-format
  json` output into auto-generated Markdown API pages.
- Generates per-item pages grouped by category (Traits, Structs, Enums,
  Functions, Type Aliases, Constants) and wires them into the Starlight
  sidebar automatically.
- Cross-crate link resolver: maintains a page registry and emits
  locale-agnostic relative URLs so the same generated Markdown works
  under `/en/...` and `/ja/...` without per-locale re-rendering.
- Multi-crate support with per-crate configuration: Cargo feature flags,
  default-features toggle, and Rust toolchain selection (defaults to
  `nightly`, which is currently required for stable `rustdoc -Z
  unstable-options --output-format json`).
- Configurable source-link integration (embeds `repository` + branch into
  generated pages) and skippable generation for preview builds.
- `sidebar: false` option to disable auto-appending sidebar entries, allowing
  full manual control over sidebar structure.
- Generic and reusable — not orts-specific despite living in this repo.
  Invoked as a Starlight `config:setup` hook plugin, so any Astro /
  Starlight site can adopt it to document Rust crates.

