# orts

A numerical computation and optimization platform for astrodynamics.

Rust core for simulation with a React-based real-time 3D viewer connected via WebSocket.

## Features

- Two-body orbital simulation with RK4 numerical integration
- Coordinate transformations (ECI, Keplerian elements, epoch handling)
- Sun-synchronous orbit support with analytical sun position
- CLI with simulation, WebSocket server, and format conversion modes
- Real-time 3D viewer with time-series charting (DuckDB-wasm + uPlot)
- History replay with downsampled overview and full-resolution detail
- Rerun `.rrd` data format for recording and export

## Project Structure

| Package | Description |
|---------|-------------|
| `orts` | CLI interface (run, serve, convert) |
| `orts-integrator` | Numerical integrators (RK4) |
| `orts-orbits` | Orbital mechanics (two-body, Keplerian elements) |
| `orts-datamodel` | ECS-inspired data model with Rerun SDK integration |
| `orts-viewer` | Real-time 3D orbit viewer (React + R3F) |
| `kaname` | Coordinate systems, epoch, sun position |
| `uneri` | DuckDB-wasm + uPlot time-series charting library |

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
