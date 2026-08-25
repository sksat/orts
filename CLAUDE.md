# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

orts is a numerical computation and optimization platform for spacecraft simulation — orbital and attitude dynamics.

- Design doc: [DESIGN.md](DESIGN.md) (Japanese) / top-level structure: [ARCHITECTURE.md](ARCHITECTURE.md)
- Crate / package inventory and roles: see the Project Structure tables in [README.md](README.md)

## Languages

- **Rust**: core simulation platform (Cargo workspace)
- **TypeScript/React**: real-time viewer and related packages (pnpm workspace)
- **Python**: helper scripts under examples/ and tools/, managed with uv

## Build & Test

- `plugin-sdk/examples/` is a standalone workspace (`cargo component build`, target `wasm32-wasip1`); `cargo test --workspace` does not cover it
- Some crates have no_std / wasm checks in CI (per-feature clippy in the lint job for no_std; wasm32 via the wasm-pack jobs such as viewer-build). `.github/workflows/ci.yml` is the source of truth — when changing such a crate, run the same checks locally

## Footguns

- `cargo test -p orts-cli` regenerates the TypeScript bindings in `viewer/src/protocol/generated/` (`TS_RS_EXPORT_DIR` in `.cargo/config.toml`), and CI enforces a clean diff — after changing protocol types, regenerate and commit
- `.cargo/config.toml` configures the mold + clang linker; a non-empty global `RUSTFLAGS` silently disables it
- Release process: see [RELEASING.md](RELEASING.md)

## Development Workflow

- For architecture-level changes, update DESIGN.md first, then implement
- Before starting implementation, get a smart-friend review of the plan
- TDD-first: verify behavior with unit tests before integrating. Validate E2E against GMAT / Orekit as reference implementations (fixture generators live in tools/)
- Before committing, run `cargo fmt` / `cargo clippy --workspace -- -D warnings` / the relevant tests / `pnpm lint`
- Changes touching logic, APIs, or design get an external review via the code-review skill before commit (typo fixes and mechanical replacements may skip it); after addressing findings, re-review until it passes
- After pushing, checking the CI result is part of the task
- When changing parts that are hard to mock (WebSocket communication, data flow, UI integration), also run the Playwright E2E tests (use the Playwright CLI, not MCP tools)

## Testing Rules

- Make every test state what it verifies; don't write tautological tests
- When you find a bug, write a reproducing test first, then fix it (regression prevention)
- Before attributing a failure to a "pre-existing issue" or "flakiness", show evidence such as a reproduction
- To delete tests or test modules, first enumerate the targets, reasons, and coverage status for user review. Solve dependency problems by adding dependencies, not by deleting tests
- For behavior-preserving refactors, pin the existing behavior with characterization tests, including boundary and non-finite inputs (`NaN`, `±∞`)

## Working Rules

- For heavy commands (e.g. `cargo test --workspace`), save the full log to a file and extract what you need from it; don't truncate with `| tail` from the start
- Don't Read large binary artifacts (gifs, images, etc.) yourself — leave judging them to human eyes. Don't commit large binary files
- Name things after what they actually are (e.g. whether a value is ground truth or noisy, whether an implicit default exists — make it readable from the name)
- Leave a TODO comment when deferring work
- Define magic numbers as constants, or comment their rationale

## Documentation

- Verify technical claims (CHANGELOG, docs, etc.) against the implementation and tests before writing them down
- In Japanese documents, keep technical terms in English (crate, workspace, commit) — no katakana transliteration

## Dependencies

- When adding a new library, look up the latest stable version first; don't pin an old version
