# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Orts is a numerical computation and optimization platform primarily for orbital mechanics. The design document is [DESIGN.md](DESIGN.md) (written in Japanese).

## Languages and Structure

- **Rust**: Core simulation platform — coordinate transformations, numerical integration, orbital mechanics solvers, CLI interface
- **TypeScript/React**: Web-based real-time viewer for simulation visualization (React + @react-three/fiber + Vite)

Rust libraries are split by responsibility (e.g., coordinate transforms, numerical integration) with independent test suites per module.

## Build Commands

### Rust (Cargo workspace)
- `cargo build --workspace` — build all crates
- `cargo test --workspace` — run all tests (55 tests across 4 crates)
- `cargo clippy --workspace` — lint all crates
- `cargo run --bin orts` — run the CLI simulator (outputs CSV)
- `cargo run --bin orts -- --serve` — start WebSocket server (port 9001)
- `cargo run --bin orts -- --serve --altitude 800 --dt 5` — custom parameters
- `cargo run --bin orts -- --serve --dt 1 --output-interval 10` — fine dt with decimated output
- `cargo test -p orts-integrator` — test only the integrator crate
- `cargo test -p orts-orbits` — test only the orbits crate
- `cargo test -p kaname` — test only the kaname crate
- `cargo test -p orts` — run CLI E2E tests

### Viewer (React + TypeScript)
- `cd viewer && pnpm install` — install dependencies
- `cd viewer && pnpm dev` — start dev server (hot reload)
- `cd viewer && pnpm build` — production build

## Development Methodology

- **TDD-first**: Write unit tests before integration. Every module (numerical integration, coordinate transforms, etc.) must have unit tests verifying behavior before being integrated.
- **Reference validation**: Use GMAT and Orekit as reference implementations for E2E black-box testing.
- **Test cases**: SSO orbits, satellite constellations, multi-year solar system trajectories, Lagrange points, gravity assists (swing-by).
- **Playwright** for viewer E2E tests.
- **CLI execution** enables simple E2E testing of the simulator independently from the viewer.

## Architecture Notes

- Systems and precision are configurable — e.g., Earth-Moon-Sun for SSO vs. full N-body for solar system simulations; detailed atmospheric drag vs. simple drag coefficients.
- Start simple (2-body/3-body at low precision), build test infrastructure, then progressively increase accuracy and problem complexity.
- Strict separation of concerns across modules to enable parallel development.
