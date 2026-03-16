# orts

A numerical simulation platform for astrodynamics.

Rust core for orbital mechanics and simulation, with a React-based real-time 3D viewer connected via WebSocket.

## Features

- N-body orbital simulation with adaptive (Dormand-Prince) and symplectic (Verlet, Yoshida) integrators
- Gravity models: point-mass, zonal harmonics (J2/J3/J4)
- Perturbations: atmospheric drag, solar radiation pressure, third-body gravity
- Atmosphere models: exponential, Harris-Priester, NRLMSISE-00
- Coordinate transforms, Keplerian elements, TLE/SGP4, epoch handling
- Spacecraft modeling: mass, attitude dynamics, thruster, surface panels
- Multi-spacecraft group propagation with event scheduling
- CLI with simulation, WebSocket server, and format conversion modes
- Real-time 3D viewer with time-series charting (DuckDB-wasm + uPlot)
- Rerun `.rrd` data format for recording and export

## Project Structure

### Rust crates

| Crate | Directory | Description |
|-------|-----------|-------------|
| `orts` | `orts/` | Orbital mechanics, simulation, perturbations, spacecraft, events |
| `orts-cli` | `cli/` | CLI + WebSocket server (binary name: `orts`) |
| `utsuroi` | `utsuroi/` | Generic ODE solvers (RK4, Dormand-Prince, Störmer-Verlet, Yoshida) |
| `kaname` | `kaname/` | Geodesy, astronomy, coordinate systems, epoch, sun/moon position |
| `tobari` | `tobari/` | Atmosphere density models, space weather (CSSI) |

### TypeScript packages

| Package | Directory | Description |
|---------|-----------|-------------|
| `@orts/uneri` | `uneri/` | DuckDB-wasm + uPlot time-series charting library |
| `viewer` | `viewer/` | Real-time 3D orbit viewer (React + @react-three/fiber) |

## Quick Start

### Simulation

```bash
cargo build --workspace
cargo test --workspace

# Run simulation and save as .rrd
cargo run --bin orts -- run

# Output CSV to stdout
cargo run --bin orts -- run --output stdout --format csv

# Custom orbit parameters
cargo run --bin orts -- run --body earth --altitude 800 --dt 5
```

### WebSocket Server + Viewer

```bash
# Terminal 1: Start simulation server
cargo run --bin orts -- serve --altitude 400 --dt 10

# Terminal 2: Start viewer dev server
cd viewer && pnpm install && pnpm dev
```

### Design Document

See [DESIGN.md](DESIGN.md) (Japanese) for the full design document.
