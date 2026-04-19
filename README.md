# orts

[![crates.io](https://img.shields.io/crates/v/orts)](https://crates.io/crates/orts)
[![CI](https://github.com/sksat/orts/actions/workflows/ci.yml/badge.svg)](https://github.com/sksat/orts/actions/workflows/ci.yml)
[![docs.rs](https://img.shields.io/docsrs/orts)](https://docs.rs/orts)
[![Docs](https://img.shields.io/badge/docs-sksat.github.io%2Forts-blue)](https://sksat.github.io/orts/)
[![License: MIT](https://img.shields.io/crates/l/orts)](LICENSE)
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/sksat/orts)

**orts** is an astrodynamics simulation platform — orbit and attitude dynamics with real-time 3D visualization, extensible WASM plugins, and in-browser analytics.

## Features

- N-body orbital simulation with adaptive (DOP853, Dormand-Prince) and symplectic (Störmer-Verlet, Yoshida) integrators
- Gravity models: point-mass, zonal harmonics (J2, J3, J4)
- Perturbations: atmospheric drag, solar radiation pressure, third-body gravity, constant thrust
- Atmosphere models: Exponential, Harris-Priester, NRLMSISE-00
- Geomagnetic field: IGRF-14 spherical harmonic expansion + tilted-dipole approximation
- Space weather: CSSI and GFZ providers (F10.7, Ap, Kp)
- IAU 2006/2000A CIO-based Earth rotation with typed coordinate frames and EOP
- Celestial body ephemerides (Meeus analytic + JPL Horizons)
- Attitude dynamics and control: reaction wheels, magnetorquers, B-dot / PD controllers
- Sensor models: magnetometer, gyroscope, sun sensor, star tracker (with noise)
- WASM Component Model plugin runtime for guest controllers via wasmtime
- CLI with embedded 3D viewer, WebSocket telemetry, and format conversion
- Real-time charting with DuckDB-wasm + uPlot (uneri library)
- Rerun `.rrd` data format for recording and replay

## Installation

```bash
# From source
cargo install orts-cli

# Pre-built binary (cargo-binstall)
cargo binstall orts-cli
```

## Quick Start

```bash
# Run a simulation (auto-detects orts.toml in current directory)
orts run

# WebSocket server with embedded 3D viewer
orts serve --config orts.toml
# Open http://localhost:9001

# Replay a recorded simulation
orts replay output.rrd

# Convert between formats
orts convert output.rrd --format csv
```

Example config (`orts.toml`):

```toml
body = "earth"
dt = 0.01
duration = 120.0

[[satellites]]
id = "sat-1"
sensors = ["gyroscope", "star_tracker"]

[satellites.orbit]
type = "circular"
altitude = 400

[satellites.attitude]
inertia_diag = [10, 10, 10]
mass = 500

[satellites.reaction_wheels]
type = "three_axis"
inertia = 0.01
max_torque = 0.5
```

### WASM Plugin

Write satellite attitude controllers in any language that compiles to WebAssembly.
Plugins receive sensor readings (magnetometer, gyroscope, star tracker, etc.) each tick and return actuator commands (reaction wheels, magnetorquers) — the simulator handles all dynamics and environment models.

```bash
# Install cargo-component
cargo install cargo-component

# Build an example plugin
cd plugin-sdk/examples/pd-rw-control
cargo component build --release
```

Add a plugin controller to your config:

```toml
[satellites.controller]
type = "wasm"
path = "target/wasm32-wasip1/release/orts_example_plugin_pd_rw_control.wasm"

[satellites.controller.config]
kp = 1.0
kd = 2.0
sample_period = 0.1
```

See [plugin-sdk/examples/](https://github.com/sksat/orts/tree/main/plugin-sdk/examples) for more plugin examples,
and [examples/](https://github.com/sksat/orts/tree/main/orts/examples) for
Apollo 11, Artemis 1, and orbital lifetime analysis demos.

## Project Structure

### Rust crates

| Crate | Directory | Description |
|-------|-----------|-------------|
| `orts` | `orts/` | Core simulation: dynamics, force/torque models, sensors, plugin host |
| `orts-cli` | `cli/` | CLI binary with embedded viewer + WebSocket server |
| `orts-plugin-sdk` | `plugin-sdk/` | SDK for writing WASM plugin guest controllers |
| `arika` (在処) | `arika/` | Coordinate frames, epochs, Earth rotation, ephemerides |
| `utsuroi` (移ろい) | `utsuroi/` | ODE integrators (RK4, DOP853, Störmer-Verlet, Yoshida) |
| `tobari` (帳) | `tobari/` | Atmosphere density, IGRF geomagnetic field, space weather |
| `rrd-wasm` | `rrd-wasm/` | Rerun RRD decoder compiled to WebAssembly |

### TypeScript / npm packages

| Package | Directory | Description |
|---------|-----------|-------------|
| `uneri` (うねり) | `uneri/` | DuckDB-wasm + uPlot streaming time-series charts |
| `orts-viewer` | `viewer/` | Real-time 3D orbit viewer (React + @react-three/fiber) |
| `starlight-rustdoc` | `starlight-rustdoc/` | Astro/Starlight plugin for Rust API docs from rustdoc JSON |

### Example plugins (`plugin-sdk/examples/`)

| Plugin | Style | Description |
|--------|-------|-------------|
| `bdot-finite-diff` | main-loop | B-dot detumbling via finite-difference dB/dt |
| `pd-rw-control` | callback | PD attitude control + reaction wheels |
| `pd-rw-unloading` | callback | PD control + magnetorquer RW unloading |
| `detumble-nadir` | callback | Detumble → nadir pointing mode transition |

## Documentation

- [Docs site](https://sksat.github.io/orts/) — API reference, examples, guides
- [DESIGN.md](DESIGN.md) — Design document (Japanese)
- [CHANGELOG.md](CHANGELOG.md) — English changelog
- [CHANGELOG.ja.md](CHANGELOG.ja.md) — Japanese changelog
- [RELEASING.md](RELEASING.md) — Release process

## License

MIT
