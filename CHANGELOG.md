# Changelog

All notable changes to this project will be documented in this file.

The format loosely follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versioning follows [Semantic Versioning](https://semver.org/).

orts is a multi-package workspace (Rust crates on crates.io and npm packages
on npm). Releases are tagged together on the same version, and each version
section is subdivided by package.

## [Unreleased]

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

[Unreleased]: https://github.com/sksat/orts/commits/main
