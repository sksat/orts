# CLAUDE.md

## Project Overview

orts is a numerical computation and optimization platform for spacecraft simulation — orbital and attitude dynamics.

- Design doc (why): [DESIGN.md](DESIGN.md)
- Architecture map (what): [ARCHITECTURE.md](ARCHITECTURE.md)
- crate / package inventory and roles: see the Project Structure tables in [README.md](README.md)

## Development Policy

- For architecture-level changes, update DESIGN.md first, then implement; keep ARCHITECTURE.md (en/ja) in sync when the structure changes
- Before starting implementation, get a smart-friend review of the plan
- TDD-first: verify behavior with unit tests before integrating. For numerical-dynamics changes, validate against reference implementations such as Orekit (fixture generators live in tools/)
- Keep responsibilities strictly separated across crates and modules — that separation is what enables parallel development and independent testing
- Before committing, run `cargo fmt` / `cargo clippy --workspace -- -D warnings` / the relevant tests / `pnpm lint`
- Changes touching logic, APIs, or design get an external review via the code-review skill before commit (typo fixes and mechanical replacements may skip it); after addressing findings, re-review until it passes
- After pushing, checking the CI result is part of the task
- When changing parts that are hard to mock (WebSocket communication, data flow, UI integration), also run the Playwright E2E tests (use the Playwright CLI, not MCP tools)

## Testing Rules

- Make every test state what it verifies; don't write tautological tests
- When you find a bug, write a reproducing test first, then fix it (regression prevention)
- Before attributing a failure to a "pre-existing issue" or "flakiness", show evidence such as a reproduction
- To delete tests or test modules, first enumerate the targets, reasons, and coverage status for user review
- For behavior-preserving refactors, pin the existing behavior with characterization tests, including boundary inputs; for floating-point code, also non-finite inputs (`NaN`, `±∞`)

## Working Rules

- For heavy commands (e.g. `cargo test --workspace`), save the full log to a file outside the worktree and extract what you need from it; don't truncate with `| tail` from the start
- Don't Read large binary artifacts (gifs, images, etc.) yourself — leave judging them to human eyes. Don't commit newly generated large binaries without user approval
- Name things after what they actually are (e.g. whether a value is ground truth or noisy, whether an implicit default exists — make it readable from the name)
- Leave a TODO comment when deferring work
- Define magic numbers as constants, or comment their rationale
- Python helper scripts (under examples/ and tools/) are managed with uv

## Footguns

- `cargo test -p orts-cli` regenerates the TypeScript bindings in `viewer/src/protocol/generated/` (`TS_RS_EXPORT_DIR` in `.cargo/config.toml`), and CI enforces a clean diff — after changing protocol types, regenerate and commit
- `.cargo/config.toml` configures the mold + clang linker; a non-empty global `RUSTFLAGS` silently disables it
- Release process: see [RELEASING.md](RELEASING.md)

## Documentation

- Verify technical claims (CHANGELOG, docs, etc.) against the implementation and tests before writing them down
- In Japanese documents, don't transliterate English technical terms into katakana (crate, not クレート)
- Use a negation ("not A, but B") only where it records a rejected alternative or an actual failure mode; decorative contrasts read as filler

## Build & Test

- `plugin-sdk/examples/` is a standalone workspace (`cargo component build`, target `wasm32-wasip1`); `cargo test --workspace` does not cover it
- Some crates have no_std / wasm checks in CI (per-feature clippy in the lint job for no_std; wasm32 via the wasm-pack jobs such as viewer-build). `.github/workflows/ci.yml` is the source of truth — when changing such a crate, run the same checks locally

## Dependencies

- When adding a new library, look up the latest stable version first; don't pin an old version
