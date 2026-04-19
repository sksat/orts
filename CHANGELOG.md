# Changelog

All notable changes to this project will be documented in this file.

The format loosely follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versioning follows [Semantic Versioning](https://semver.org/).

orts is a multi-package workspace (Rust crates on crates.io and npm packages
on npm). Releases are tagged together on the same version, and each version
section is subdivided by package.

## [Unreleased]

## [0.2.0](https://github.com/sksat/orts/releases/tag/v0.2.0) - 2026-04-20

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

